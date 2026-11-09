//! FIXT.1.1 framing: SOH fields, body length, and checksum.

use crate::error::{Error, Result};
use crate::tags::{
    BEGIN_STRING, BODY_LENGTH, CHECKSUM, MSG_SEQ_NUM, MSG_TYPE, POSS_DUP_FLAG, SENDER_COMP_ID,
    SENDER_SUB_ID, SENDING_TIME, TARGET_COMP_ID,
};
use crate::timefmt::{format_millis, is_fix_millis};
use crate::{BEGIN_STRING as BEGIN, TARGET_COMP_ID as TARGET};
use chrono::{DateTime, Utc};

/// Field delimiter. Displayed as `|` in [`Message::to_string`].
pub const SOH: u8 = 0x01;

const REDACTED: &[u32] = &[96, 554];

/// One FIX message, fields in wire order. Tag 8, 9, and 10 are included after decode.
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    pub fields: Vec<(u32, String)>,
}

impl Message {
    pub fn get(&self, tag: u32) -> Option<&str> {
        self.fields
            .iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, v)| v.as_str())
    }

    pub fn values(&self, tag: u32) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|(t, _)| *t == tag)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn msg_type(&self) -> Result<&str> {
        self.get(MSG_TYPE)
            .ok_or_else(|| Error::codec("message has no MsgType"))
    }

    pub fn seq(&self) -> Result<u32> {
        let raw = self
            .get(MSG_SEQ_NUM)
            .ok_or_else(|| Error::codec("message has no MsgSeqNum"))?;
        if raw.starts_with('0') && raw != "0" {
            return Err(Error::codec(format!(
                "MsgSeqNum `{raw}` has a leading zero"
            )));
        }
        raw.parse()
            .map_err(|_| Error::codec(format!("bad MsgSeqNum `{raw}`")))
    }

    pub fn poss_dup(&self) -> bool {
        self.get(POSS_DUP_FLAG) == Some("Y")
    }

    /// Repeating group that starts with `delim` immediately after `count_tag`.
    /// Fields between the count and the first delimiter are skipped (they belong
    /// to the parent). Returns `Ok(vec![])` when the count tag is absent.
    pub fn group(&self, count_tag: u32, delim: u32) -> Result<Vec<Vec<(u32, String)>>> {
        let Some(pos) = self.fields.iter().position(|(t, _)| *t == count_tag) else {
            return Ok(vec![]);
        };
        let count: usize = self.fields[pos]
            .1
            .parse()
            .map_err(|_| Error::codec(format!("bad NumInGroup for tag {count_tag}")))?;
        if count == 0 {
            return Ok(vec![]);
        }
        let mut groups = Vec::new();
        let mut cur: Vec<(u32, String)> = Vec::new();
        for field in self.fields.iter().skip(pos + 1) {
            if field.0 == delim {
                if !cur.is_empty() {
                    groups.push(std::mem::take(&mut cur));
                    if groups.len() == count {
                        break;
                    }
                }
                cur.push(field.clone());
            } else if !cur.is_empty() {
                cur.push(field.clone());
            }
        }
        if !cur.is_empty() {
            groups.push(cur);
        }
        if groups.len() != count {
            return Err(Error::codec(format!(
                "tag {count_tag} says {count} groups, found {}",
                groups.len()
            )));
        }
        Ok(groups)
    }
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Message({})", self.redacted())
    }
}

impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.redacted())
    }
}

impl Message {
    fn redacted(&self) -> String {
        let mut out = String::new();
        for (i, (tag, value)) in self.fields.iter().enumerate() {
            if i > 0 {
                out.push('|');
            }
            let shown = if REDACTED.contains(tag) {
                "***"
            } else {
                value.as_str()
            };
            out.push_str(&format!("{tag}={shown}"));
        }
        out
    }
}

/// Header fields the session fills in. `SendingTime` must already be millisecond FIX time
/// when it is a signed logon; pass [`OutHeader::from_parts`] for that.
#[derive(Debug, Clone)]
pub struct OutHeader {
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub msg_seq_num: u32,
    pub sending_time: String,
    pub poss_dup: bool,
    pub sender_sub_id: Option<String>,
}

impl OutHeader {
    pub fn new(sender_comp_id: impl Into<String>, seq: u32, now: DateTime<Utc>) -> Self {
        Self {
            sender_comp_id: sender_comp_id.into(),
            target_comp_id: TARGET.to_string(),
            msg_seq_num: seq,
            sending_time: format_millis(now),
            poss_dup: false,
            sender_sub_id: None,
        }
    }

    pub fn from_parts(
        sender_comp_id: impl Into<String>,
        target_comp_id: impl Into<String>,
        seq: u32,
        sending_time: impl Into<String>,
    ) -> Result<Self> {
        let sending_time = sending_time.into();
        if !is_fix_millis(&sending_time) {
            return Err(Error::codec(format!(
                "SendingTime `{sending_time}` must be YYYYMMDD-HH:MM:SS.sss"
            )));
        }
        if seq == 0 || seq > i32::MAX as u32 {
            return Err(Error::codec(format!(
                "MsgSeqNum {seq} is outside 1..={MAX_SEQ}"
            )));
        }
        Ok(Self {
            sender_comp_id: sender_comp_id.into(),
            target_comp_id: target_comp_id.into(),
            msg_seq_num: seq,
            sending_time,
            poss_dup: false,
            sender_sub_id: None,
        })
    }
}

const MAX_SEQ: u32 = 2_147_483_647;

/// An application (or admin) body. The session adds the standard header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMessage {
    pub msg_type: String,
    pub fields: Vec<(u32, String)>,
    /// Copied to `SenderSubID` (50). Required for EXN orders.
    pub sender_sub_id: Option<String>,
}

impl AppMessage {
    pub fn new(msg_type: impl Into<String>, fields: Vec<(u32, String)>) -> Self {
        Self {
            msg_type: msg_type.into(),
            fields,
            sender_sub_id: None,
        }
    }

    pub fn with_sender_sub_id(mut self, id: impl Into<String>) -> Self {
        self.sender_sub_id = Some(id.into());
        self
    }
}

/// Encode `msg_type` and `fields` under `header`. Field order in `fields` is kept.
pub fn encode(header: &OutHeader, msg_type: &str, fields: &[(u32, String)]) -> Result<Vec<u8>> {
    if msg_type.is_empty() || msg_type.bytes().any(|b| b == SOH) {
        return Err(Error::codec("empty or illegal MsgType"));
    }
    check_seq(header.msg_seq_num)?;
    if !is_fix_millis(&header.sending_time) {
        return Err(Error::codec(format!(
            "SendingTime `{}` must be YYYYMMDD-HH:MM:SS.sss",
            header.sending_time
        )));
    }
    let mut body: Vec<(u32, String)> = vec![
        (MSG_TYPE, msg_type.to_string()),
        (SENDER_COMP_ID, header.sender_comp_id.clone()),
        (TARGET_COMP_ID, header.target_comp_id.clone()),
        (MSG_SEQ_NUM, header.msg_seq_num.to_string()),
    ];
    if header.poss_dup {
        body.push((POSS_DUP_FLAG, "Y".into()));
    }
    body.push((SENDING_TIME, header.sending_time.clone()));
    if let Some(sub) = &header.sender_sub_id {
        push_value(&mut body, SENDER_SUB_ID, sub)?;
    }
    for (tag, value) in fields {
        if *tag == BEGIN_STRING || *tag == BODY_LENGTH || *tag == CHECKSUM || *tag == MSG_TYPE {
            return Err(Error::codec(format!("tag {tag} is written by the framer")));
        }
        push_value(&mut body, *tag, value)?;
    }

    let mut payload = Vec::new();
    for (tag, value) in &body {
        write_field(&mut payload, *tag, value);
    }
    let mut out = Vec::new();
    write_field(&mut out, BEGIN_STRING, BEGIN);
    write_field(&mut out, BODY_LENGTH, &payload.len().to_string());
    out.extend_from_slice(&payload);
    let sum = checksum(&out);
    write_field(&mut out, CHECKSUM, &format!("{sum:03}"));
    Ok(out)
}

fn check_seq(seq: u32) -> Result<()> {
    if seq == 0 || seq > MAX_SEQ {
        return Err(Error::codec(format!(
            "MsgSeqNum {seq} is outside 1..={MAX_SEQ}"
        )));
    }
    Ok(())
}

fn push_value(body: &mut Vec<(u32, String)>, tag: u32, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::codec(format!("tag {tag} is empty")));
    }
    if value.bytes().any(|b| b == SOH) {
        return Err(Error::codec(format!("tag {tag} contains SOH")));
    }
    body.push((tag, value.to_string()));
    Ok(())
}

fn write_field(out: &mut Vec<u8>, tag: u32, value: &str) {
    out.extend_from_slice(tag.to_string().as_bytes());
    out.push(b'=');
    out.extend_from_slice(value.as_bytes());
    out.push(SOH);
}

/// Sum of bytes, modulo 256.
pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, b| acc.wrapping_add(*b))
}

/// A decode step over a TCP buffer.
#[derive(Debug)]
pub enum Decode {
    /// Drop `discard` leading bytes and read more.
    Incomplete { discard: usize },
    /// A full message. `consumed` includes any junk skipped before `8=`.
    Frame { message: Message, consumed: usize },
}

/// Pull one message from `buf`. Checksum mismatches are errors once a full frame is present.
pub fn decode_one(buf: &[u8]) -> Result<Decode> {
    let Some(start) = find_subsequence(buf, b"8=") else {
        let discard = buf.len().saturating_sub(1);
        return Ok(Decode::Incomplete { discard });
    };
    let rest = &buf[start..];
    let Some(len_at) = find_subsequence(rest, b"\x019=") else {
        return Ok(Decode::Incomplete { discard: 0 });
    };
    let begin = &rest[..len_at];
    if begin != b"8=FIXT.1.1" {
        return Err(Error::codec(format!(
            "BeginString `{}` is not FIXT.1.1",
            String::from_utf8_lossy(begin)
        )));
    }
    let digits_at = len_at + 3;
    let Some(soh) = rest[digits_at..].iter().position(|b| *b == SOH) else {
        return Ok(Decode::Incomplete { discard: 0 });
    };
    let len_str = std::str::from_utf8(&rest[digits_at..digits_at + soh])
        .map_err(|_| Error::codec("BodyLength is not utf-8"))?;
    let body_len: usize = len_str
        .parse()
        .map_err(|_| Error::codec(format!("bad BodyLength `{len_str}`")))?;
    let body_at = digits_at + soh + 1;
    let frame_end = body_at + body_len;
    let checksum_end = frame_end + 3 + 3 + 1;
    if rest.len() < checksum_end {
        return Ok(Decode::Incomplete { discard: 0 });
    }
    if &rest[frame_end..frame_end + 3] != b"10=" {
        return Err(Error::codec(
            "checksum tag is not where BodyLength said it would be",
        ));
    }
    let sum_bytes = &rest[frame_end + 3..frame_end + 6];
    if rest[frame_end + 6] != SOH {
        return Err(Error::codec("checksum is not three digits"));
    }
    let got: u32 = std::str::from_utf8(sum_bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::codec("bad checksum"))?;
    let expect = checksum(&rest[..frame_end]) as u32;
    if got != expect {
        return Err(Error::codec(format!(
            "checksum {got:03} does not match {expect:03}"
        )));
    }
    let mut fields = vec![
        (BEGIN_STRING, BEGIN.to_string()),
        (BODY_LENGTH, len_str.to_string()),
    ];
    fields.extend(parse_fields(&rest[body_at..frame_end])?);
    fields.push((CHECKSUM, format!("{got:03}")));
    let message = Message { fields };
    Ok(Decode::Frame {
        message,
        consumed: start + checksum_end,
    })
}

fn parse_fields(mut body: &[u8]) -> Result<Vec<(u32, String)>> {
    let mut fields = Vec::new();
    while !body.is_empty() {
        let Some(eq) = body.iter().position(|b| *b == b'=') else {
            return Err(Error::codec("field has no '='"));
        };
        let tag: u32 = std::str::from_utf8(&body[..eq])
            .map_err(|_| Error::codec("tag is not utf-8"))?
            .parse()
            .map_err(|_| Error::codec("tag is not an integer"))?;
        let value_at = eq + 1;
        let Some(soh) = body[value_at..].iter().position(|b| *b == SOH) else {
            return Err(Error::codec("field has no SOH"));
        };
        let value = std::str::from_utf8(&body[value_at..value_at + soh])
            .map_err(|_| Error::codec("value is not utf-8"))?
            .to_string();
        if value.is_empty() {
            return Err(Error::codec(format!("tag {tag} is empty")));
        }
        fields.push((tag, value));
        body = &body[value_at + soh + 1..];
    }
    Ok(fields)
}

fn find_subsequence(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_leading_zero_seq_rejected() {
        let header =
            OutHeader::from_parts("test-key", "Coinbase", 1, "20230822-20:43:30.000").unwrap();
        let bytes = encode(&header, "0", &[]).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("34=01"));
        match decode_one(&bytes).unwrap() {
            Decode::Frame { message, consumed } => {
                assert_eq!(consumed, bytes.len());
                assert_eq!(message.msg_type().unwrap(), "0");
                assert_eq!(message.seq().unwrap(), 1);
                assert_eq!(message.get(52), Some("20230822-20:43:30.000"));
            }
            Decode::Incomplete { .. } => panic!("complete frame"),
        }
        let mut bad =
            OutHeader::from_parts("test-key", "Coinbase", 1, "20230822-20:43:30.000").unwrap();
        bad.sending_time = "20230822-20:43:30".into();
        assert!(encode(&bad, "0", &[]).is_err());
    }

    #[test]
    fn two_frames_in_one_buffer() {
        let header = OutHeader::from_parts("k", "Coinbase", 2, "20230822-20:43:30.000").unwrap();
        let mut buf = encode(&header, "0", &[]).unwrap();
        let second = encode(&header, "1", &[(112, "abc".into())]).unwrap();
        buf.extend_from_slice(&second);
        let Decode::Frame { message, consumed } = decode_one(&buf).unwrap() else {
            panic!("frame")
        };
        assert_eq!(message.msg_type().unwrap(), "0");
        let Decode::Frame { message, .. } = decode_one(&buf[consumed..]).unwrap() else {
            panic!("frame")
        };
        assert_eq!(message.get(112), Some("abc"));
    }

    #[test]
    fn checksum_catches_a_flipped_byte() {
        let header = OutHeader::from_parts("k", "Coinbase", 1, "20230822-20:43:30.000").unwrap();
        let mut bytes = encode(&header, "0", &[]).unwrap();
        let at = bytes.windows(3).position(|w| w == b"49=").unwrap() + 3;
        bytes[at] ^= 0x01;
        assert!(decode_one(&bytes).is_err());
    }
}
