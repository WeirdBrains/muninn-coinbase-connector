//! Sequence numbers, resend, and gap fill. The socket loop in `session` owns time.
//!
//! A Sequence Reset that arrives at inbound seq 2 with no PossDupFlag is the
//! post-logon announcement of the server's stored sequence. It is not applied as a
//! gap fill: the engine asks for that range, starting at [`Engine::set_resume_from`]
//! when the connector has already stored a prefix.

use std::collections::{BTreeMap, VecDeque};

use crate::codec::{AppMessage, Message};
use crate::error::{Error, Result};
use crate::limits::MAX_RESEND_RANGE;
use crate::parse::{Envelope, envelope};
use crate::tags::{BEGIN_SEQ_NO, END_SEQ_NO, GAP_FILL_FLAG, NEW_SEQ_NO, TEST_REQ_ID};

const LOG_CAP: usize = 100_000;

#[derive(Debug, Clone)]
pub(crate) struct Outbound {
    pub msg_type: String,
    pub fields: Vec<(u32, String)>,
    pub seq: u32,
    pub poss_dup: bool,
    pub sender_sub_id: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Action {
    Send(Outbound),
    Deliver(Box<Envelope>),
    Disconnect(String),
}

#[derive(Debug, Clone)]
struct Stored {
    seq: u32,
    msg_type: String,
    fields: Vec<(u32, String)>,
    admin: bool,
    sender_sub_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Engine {
    next_out: u32,
    next_in: u32,
    logout_sent: bool,
    pending: Option<(u32, u32)>,
    /// First inbound seq still wanted when resuming a previous session.
    resume_from: Option<u32>,
    /// Last seq to request after a post-logon sequence announcement. Pages of
    /// [`MAX_RESEND_RANGE`] follow until `next_in` passes it.
    resume_until: Option<u32>,
    log: VecDeque<Stored>,
    buffer: BTreeMap<u32, Message>,
}

impl Engine {
    /// Called after a successful logon exchange. Outbound and inbound sequences
    /// continue at 2; the logon itself is remembered as admin seq 1.
    pub(crate) fn after_logon() -> Self {
        let mut engine = Self {
            next_out: 2,
            next_in: 2,
            logout_sent: false,
            pending: None,
            resume_from: None,
            resume_until: None,
            log: VecDeque::new(),
            buffer: BTreeMap::new(),
        };
        engine.log.push_back(Stored {
            seq: 1,
            msg_type: "A".into(),
            fields: Vec::new(),
            admin: true,
            sender_sub_id: None,
        });
        engine
    }

    /// `seq` is the next inbound sequence the caller still wants. Lower sequences
    /// are treated as already stored and are not requested again.
    pub(crate) fn set_resume_from(&mut self, seq: u32) {
        if seq > 1 {
            self.resume_from = Some(seq);
        }
    }

    pub(crate) fn on_app(&mut self, msg: AppMessage) -> Result<Vec<Action>> {
        if msg.msg_type == "A" {
            return Err(Error::protocol("logon is sent by connect"));
        }
        if msg.msg_type == "5" {
            self.logout_sent = true;
        }
        Ok(vec![Action::Send(self.allocate(
            msg.msg_type,
            msg.fields,
            msg.sender_sub_id,
        ))])
    }

    pub(crate) fn on_inbound(&mut self, msg: Message) -> Result<Vec<Action>> {
        let seq = msg.seq()?;
        let msg_type = msg.msg_type()?.to_string();
        if msg_type == "4" {
            return self.on_sequence_reset(msg, seq);
        }
        if seq == self.next_in {
            self.next_in = self.next_in.saturating_add(1);
            let mut actions = self.handle(msg)?;
            actions.extend(self.after_advance()?);
            return Ok(actions);
        }
        if seq < self.next_in {
            if msg.poss_dup() {
                return Ok(vec![]);
            }
            return Ok(vec![Action::Disconnect(format!(
                "inbound seq {seq} is below expected {}",
                self.next_in
            ))]);
        }
        if self.buffer.len() >= MAX_RESEND_RANGE as usize {
            return Ok(vec![Action::Disconnect(
                "too many messages buffered while waiting on a gap".into(),
            )]);
        }
        self.buffer.insert(seq, msg);
        let mut actions = self.open_resend()?;
        actions.extend(self.after_advance()?);
        Ok(actions)
    }

    fn on_sequence_reset(&mut self, msg: Message, seq: u32) -> Result<Vec<Action>> {
        let new_seq = msg
            .get(NEW_SEQ_NO)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if seq < self.next_in {
            return Ok(if msg.poss_dup() {
                vec![]
            } else {
                vec![Action::Disconnect(format!(
                    "sequence reset seq {seq} is below expected {}",
                    self.next_in
                ))]
            });
        }
        if seq > self.next_in {
            self.buffer.insert(seq, msg);
            return self.open_resend();
        }
        if new_seq < self.next_in {
            return Ok(vec![Action::Disconnect(format!(
                "NewSeqNo {new_seq} does not move the sequence forward"
            ))]);
        }
        // Coinbase sends this, without PossDupFlag, immediately after logon to name
        // the last sequence it stored. The client requests that range; it does not skip it.
        if self.next_in == 2
            && !msg.poss_dup()
            && self.pending.is_none()
            && self.buffer.is_empty()
            && new_seq > self.next_in
        {
            return Ok(self.announce_resume(msg, new_seq));
        }
        self.next_in = new_seq;
        let mut actions = vec![Action::Deliver(Box::new(envelope(msg)))];
        actions.extend(self.after_advance()?);
        Ok(actions)
    }

    fn handle(&mut self, msg: Message) -> Result<Vec<Action>> {
        let msg_type = msg.msg_type()?.to_string();
        let mut actions = Vec::new();
        match msg_type.as_str() {
            "1" => {
                let id = msg.get(TEST_REQ_ID).unwrap_or("").to_string();
                let mut fields = Vec::new();
                if !id.is_empty() {
                    fields.push((TEST_REQ_ID, id));
                }
                actions.push(Action::Send(self.allocate("0".into(), fields, None)));
                actions.push(Action::Deliver(Box::new(envelope(msg))));
            }
            "2" => {
                let begin: u32 = msg
                    .get(BEGIN_SEQ_NO)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let end: u32 = msg
                    .get(END_SEQ_NO)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if begin < 1 || end < begin {
                    return Ok(vec![Action::Disconnect(format!(
                        "resend {begin}..={end} is not a valid range"
                    ))]);
                }
                let end = end.min(begin.saturating_add(MAX_RESEND_RANGE - 1));
                actions.extend(self.reply_resend(begin, end).into_iter().map(Action::Send));
                actions.push(Action::Deliver(Box::new(envelope(msg))));
            }
            "5" => {
                actions.push(Action::Deliver(Box::new(envelope(msg))));
                if !self.logout_sent {
                    self.logout_sent = true;
                    actions.push(Action::Send(self.allocate("5".into(), Vec::new(), None)));
                }
                actions.push(Action::Disconnect("peer logged out".into()));
            }
            _ => actions.push(Action::Deliver(Box::new(envelope(msg)))),
        }
        Ok(actions)
    }

    fn after_advance(&mut self) -> Result<Vec<Action>> {
        if let Some((_, end)) = self.pending
            && self.next_in > end
        {
            self.pending = None;
        }
        let mut actions = Vec::new();
        while let Some(msg) = self.buffer.remove(&self.next_in) {
            let seq = msg.seq()?;
            if msg.msg_type()? == "4" {
                actions.extend(self.on_sequence_reset(msg, seq)?);
                return Ok(actions);
            }
            self.next_in = self.next_in.saturating_add(1);
            actions.extend(self.handle(msg)?);
        }
        if self.pending.is_none()
            && let Some((&seq, _)) = self.buffer.iter().next()
            && seq > self.next_in
        {
            actions.extend(self.open_resend()?);
        }
        if self.pending.is_none()
            && let Some(until) = self.resume_until
        {
            if self.next_in > until {
                self.resume_until = None;
            } else {
                actions.extend(self.open_resume_page(until));
            }
        }
        Ok(actions)
    }

    fn announce_resume(&mut self, msg: Message, new_seq: u32) -> Vec<Action> {
        let start = match self.resume_from {
            Some(n) if n > self.next_in && n <= new_seq.saturating_add(1) => n,
            _ => self.next_in,
        };
        self.resume_from = None;
        if start > new_seq {
            self.next_in = start;
            return vec![Action::Deliver(Box::new(envelope(msg)))];
        }
        self.next_in = start;
        self.resume_until = Some(new_seq);
        let end = new_seq.min(start.saturating_add(MAX_RESEND_RANGE - 1));
        self.pending = Some((start, end));
        let resend = self.allocate(
            "2".into(),
            vec![
                (BEGIN_SEQ_NO, start.to_string()),
                (END_SEQ_NO, end.to_string()),
            ],
            None,
        );
        vec![
            Action::Deliver(Box::new(envelope(msg))),
            Action::Send(resend),
        ]
    }

    fn open_resume_page(&mut self, until: u32) -> Vec<Action> {
        let begin = self.next_in;
        if begin > until {
            self.resume_until = None;
            return vec![];
        }
        let end = until.min(begin.saturating_add(MAX_RESEND_RANGE - 1));
        self.pending = Some((begin, end));
        let msg = self.allocate(
            "2".into(),
            vec![
                (BEGIN_SEQ_NO, begin.to_string()),
                (END_SEQ_NO, end.to_string()),
            ],
            None,
        );
        vec![Action::Send(msg)]
    }

    fn open_resend(&mut self) -> Result<Vec<Action>> {
        if self.pending.is_some() {
            return Ok(vec![]);
        }
        let Some((&have, _)) = self.buffer.iter().next() else {
            return Ok(vec![]);
        };
        if have <= self.next_in {
            return Ok(vec![]);
        }
        let begin = self.next_in;
        let end = have
            .saturating_sub(1)
            .min(begin.saturating_add(MAX_RESEND_RANGE - 1));
        self.pending = Some((begin, end));
        let msg = self.allocate(
            "2".into(),
            vec![
                (BEGIN_SEQ_NO, begin.to_string()),
                (END_SEQ_NO, end.to_string()),
            ],
            None,
        );
        Ok(vec![Action::Send(msg)])
    }

    fn reply_resend(&mut self, begin: u32, end: u32) -> Vec<Outbound> {
        let mut out = Vec::new();
        let mut seq = begin;
        while seq <= end {
            match self.lookup(seq) {
                Some(stored) if !stored.admin => {
                    out.push(Outbound {
                        msg_type: stored.msg_type.clone(),
                        fields: stored.fields.clone(),
                        seq,
                        poss_dup: true,
                        sender_sub_id: stored.sender_sub_id.clone(),
                    });
                    seq += 1;
                }
                _ => {
                    let mut next = seq + 1;
                    while next <= end {
                        if matches!(self.lookup(next), Some(stored) if !stored.admin) {
                            break;
                        }
                        next += 1;
                    }
                    out.push(Outbound {
                        msg_type: "4".into(),
                        fields: vec![(GAP_FILL_FLAG, "Y".into()), (NEW_SEQ_NO, next.to_string())],
                        seq,
                        poss_dup: true,
                        sender_sub_id: None,
                    });
                    seq = next;
                }
            }
        }
        out
    }

    fn lookup(&self, seq: u32) -> Option<&Stored> {
        self.log.iter().find(|s| s.seq == seq)
    }

    fn allocate(
        &mut self,
        msg_type: String,
        fields: Vec<(u32, String)>,
        sender_sub_id: Option<String>,
    ) -> Outbound {
        let seq = self.next_out;
        self.next_out = self.next_out.saturating_add(1);
        let admin = matches!(msg_type.as_str(), "0" | "1" | "2" | "3" | "4" | "5" | "A");
        self.log.push_back(Stored {
            seq,
            msg_type: msg_type.clone(),
            fields: fields.clone(),
            admin,
            sender_sub_id: sender_sub_id.clone(),
        });
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
        Outbound {
            msg_type,
            fields,
            seq,
            poss_dup: false,
            sender_sub_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::OutHeader;
    use crate::codec::encode;
    use crate::parse::Incoming;

    fn inbound(msg_type: &str, seq: u32, fields: Vec<(u32, String)>) -> Message {
        let header =
            OutHeader::from_parts("Coinbase", "key", seq, "20230822-20:43:30.000").unwrap();
        let bytes = encode(&header, msg_type, &fields).unwrap();
        let crate::codec::Decode::Frame { message, .. } = crate::codec::decode_one(&bytes).unwrap()
        else {
            panic!("frame")
        };
        message
    }

    #[test]
    fn gap_requests_at_most_one_thousand() {
        let mut engine = Engine::after_logon();
        let actions = engine.on_inbound(inbound("0", 2_500, vec![])).unwrap();
        let Action::Send(resend) = &actions[0] else {
            panic!("resend")
        };
        assert_eq!(resend.msg_type, "2");
        assert_eq!(resend.fields[0].1, "2");
        assert_eq!(resend.fields[1].1, "1001");
        let again = engine.on_inbound(inbound("0", 2_501, vec![])).unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn sequence_reset_jumps_and_drains() {
        let mut engine = Engine::after_logon();
        engine
            .on_inbound(inbound("8", 5, vec![(11, "x".into())]))
            .unwrap();
        let actions = engine
            .on_inbound(inbound("4", 2, vec![(123, "Y".into()), (36, "5".into())]))
            .unwrap();
        assert!(actions.iter().any(|a| matches!(a, Action::Deliver(env) if matches!(env.body, Incoming::ExecutionReport(_)))));
        assert_eq!(engine.next_in, 6);
    }

    #[test]
    fn poss_dup_below_expected_is_ignored() {
        let mut engine = Engine::after_logon();
        engine.on_inbound(inbound("0", 2, vec![])).unwrap();
        let mut old = inbound("0", 2, vec![]);
        old.fields.push((43, "Y".into()));
        assert!(engine.on_inbound(old).unwrap().is_empty());
    }

    #[test]
    fn resend_reply_gap_fills_admin_and_replays_orders() {
        let mut engine = Engine::after_logon();
        engine.on_app(AppMessage::new("0", vec![])).unwrap();
        engine
            .on_app(AppMessage::new("D", vec![(11, "abc".into())]))
            .unwrap();
        engine.on_app(AppMessage::new("0", vec![])).unwrap();
        let actions = engine
            .on_inbound(inbound("2", 2, vec![(7, "2".into()), (16, "4".into())]))
            .unwrap();
        let sends: Vec<&Outbound> = actions
            .iter()
            .filter_map(|a| match a {
                Action::Send(o) => Some(o),
                _ => None,
            })
            .collect();
        assert_eq!(sends[0].msg_type, "4");
        assert_eq!(sends[0].seq, 2);
        assert_eq!(
            sends[0].fields.iter().find(|(t, _)| *t == 36).unwrap().1,
            "3"
        );
        assert!(sends[0].poss_dup);
        assert_eq!(sends[1].msg_type, "D");
        assert_eq!(sends[1].seq, 3);
        assert!(sends[1].poss_dup);
        assert_eq!(sends[2].msg_type, "4");
        assert_eq!(sends[2].seq, 4);
    }

    fn sent(actions: &[Action]) -> Vec<&Outbound> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Send(o) => Some(o),
                _ => None,
            })
            .collect()
    }

    fn tag(msg: &Outbound, t: u32) -> &str {
        msg.fields
            .iter()
            .find(|(k, _)| *k == t)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }

    #[test]
    fn post_logon_reset_requests_the_stored_range() {
        let mut engine = Engine::after_logon();
        let actions = engine
            .on_inbound(inbound("4", 2, vec![(36, "4".into())]))
            .unwrap();
        let send = sent(&actions);
        assert_eq!(send.len(), 1);
        assert_eq!(send[0].msg_type, "2");
        assert_eq!(tag(send[0], 7), "2");
        assert_eq!(tag(send[0], 16), "4");
        assert_eq!(engine.next_in, 2);
        for seq in 2..=4 {
            engine.on_inbound(inbound("0", seq, vec![])).unwrap();
        }
        assert_eq!(engine.next_in, 5);
        assert_eq!(engine.resume_until, None);
        assert!(engine.pending.is_none());
    }

    #[test]
    fn resume_from_skips_a_stored_prefix() {
        let mut engine = Engine::after_logon();
        engine.set_resume_from(4);
        let actions = engine
            .on_inbound(inbound("4", 2, vec![(36, "6".into())]))
            .unwrap();
        let send = sent(&actions);
        assert_eq!(tag(send[0], 7), "4");
        assert_eq!(tag(send[0], 16), "6");
        assert_eq!(engine.next_in, 4);
    }

    #[test]
    fn resume_from_already_caught_up_waits_for_the_live_seq() {
        let mut engine = Engine::after_logon();
        engine.set_resume_from(5);
        let actions = engine
            .on_inbound(inbound("4", 2, vec![(36, "4".into())]))
            .unwrap();
        assert!(sent(&actions).is_empty());
        assert_eq!(engine.next_in, 5);
    }

    #[test]
    fn resume_pages_the_next_thousand() {
        let mut engine = Engine::after_logon();
        let first = engine
            .on_inbound(inbound("4", 2, vec![(36, "1500".into())]))
            .unwrap();
        assert_eq!(tag(sent(&first)[0], 16), "1001");
        engine.next_in = 1002;
        engine.pending = None;
        let next = engine.after_advance().unwrap();
        assert_eq!(tag(sent(&next)[0], 7), "1002");
        assert_eq!(tag(sent(&next)[0], 16), "1500");
    }

    #[test]
    fn test_request_is_answered() {
        let mut engine = Engine::after_logon();
        let actions = engine
            .on_inbound(inbound("1", 2, vec![(112, "ping".into())]))
            .unwrap();
        let Action::Send(hb) = &actions[0] else {
            panic!("heartbeat")
        };
        assert_eq!(hb.msg_type, "0");
        assert_eq!(hb.fields[0], (112, "ping".into()));
    }
}
