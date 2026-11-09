//! Inbound messages. Missing optional fields stay empty; a message is still
//! returned so a fill is not dropped because the gateway added a tag.

use crate::codec::Message;
use crate::market::{
    Instrument, MdEntry, MdEntryType, MdIncremental, MdSnapshot, MdUpdateAction, SecurityList,
    SnapEntry,
};
use crate::order::Side;
use crate::tags::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub message: Message,
    pub body: Incoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    Heartbeat { test_req_id: Option<String> },
    TestRequest { test_req_id: String },
    ResendRequest { begin: u32, end: u32 },
    SequenceReset { gap_fill: bool, new_seq_no: u32 },
    Reject(SessionReject),
    Logout { text: Option<String> },
    Logon(LogonAck),
    ExecutionReport(Box<ExecutionReport>),
    OrderCancelReject(CancelReject),
    NewOrderBatchReject(NewOrderBatchReject),
    OrderCancelBatchReject(OrderCancelBatchReject),
    BusinessMessageReject(BusinessReject),
    OrderMassCancelReport(MassCancelReport),
    MarketDataIncremental(MdIncremental),
    MarketDataSnapshot(MdSnapshot),
    MarketDataRequestReject(MdReject),
    SecurityStatus(SecurityStatus),
    SecurityList(SecurityList),
    QuoteRequest(QuoteRequest),
    QuoteStatusReport(Box<QuoteStatusReport>),
    Other { msg_type: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReject {
    pub ref_seq_num: Option<u32>,
    pub ref_tag_id: Option<u32>,
    pub ref_msg_type: Option<String>,
    pub reason: Option<i32>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogonAck {
    pub heart_bt_int: Option<u64>,
    pub reset_seq_num: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdStatus {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Replaced,
    Rejected,
    Expired,
    PendingNew,
    Other(char),
}

impl OrdStatus {
    fn from_wire(c: char) -> Self {
        match c {
            '0' => Self::New,
            '1' => Self::PartiallyFilled,
            '2' => Self::Filled,
            '4' => Self::Canceled,
            '5' => Self::Replaced,
            '8' => Self::Rejected,
            'C' => Self::Expired,
            'A' => Self::PendingNew,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecType {
    New,
    Canceled,
    Replaced,
    Rejected,
    Expired,
    Restated,
    Trade,
    OrderStatus,
    Other(char),
}

impl ExecType {
    fn from_wire(c: char) -> Self {
        match c {
            '0' => Self::New,
            '4' => Self::Canceled,
            '5' => Self::Replaced,
            '8' => Self::Rejected,
            'C' => Self::Expired,
            'D' => Self::Restated,
            'F' => Self::Trade,
            'I' => Self::OrderStatus,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecRestatementReason {
    /// TPSL repricing (`3`).
    Repricing,
    /// Partial decline from self-trade prevention (`5`).
    PartialDecline,
    Other(i32),
}

impl ExecRestatementReason {
    fn from_wire(n: i32) -> Self {
        match n {
            3 => Self::Repricing,
            5 => Self::PartialDecline,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiLegReportingType {
    Single,
    IndividualLeg,
    MultiLeg,
    Other(char),
}

impl MultiLegReportingType {
    fn from_wire(c: char) -> Self {
        match c {
            '1' => Self::Single,
            '2' => Self::IndividualLeg,
            '3' => Self::MultiLeg,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiscFee {
    pub amt: String,
    pub currency: Option<String>,
    pub fee_type: Option<String>,
    pub basis: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReport {
    pub cl_ord_id: Option<String>,
    pub order_id: Option<String>,
    pub orig_cl_ord_id: Option<String>,
    pub exec_id: Option<String>,
    pub ord_status: Option<OrdStatus>,
    pub exec_type: Option<ExecType>,
    pub symbol: Option<String>,
    pub side: Option<Side>,
    pub ord_type: Option<char>,
    pub avg_px: Option<String>,
    pub cum_qty: Option<String>,
    pub leaves_qty: Option<String>,
    pub last_qty: Option<String>,
    pub last_px: Option<String>,
    pub price: Option<String>,
    pub order_qty: Option<String>,
    pub cash_order_qty: Option<String>,
    pub stop_px: Option<String>,
    pub stop_limit_px: Option<String>,
    pub text: Option<String>,
    pub transact_time: Option<String>,
    pub ord_rej_reason: Option<i32>,
    pub restatement_reason: Option<ExecRestatementReason>,
    pub trade_id: Option<String>,
    pub aggressor: Option<bool>,
    pub time_in_force: Option<char>,
    pub trigger_direction: Option<char>,
    pub exec_inst: Option<char>,
    pub self_trade: Option<char>,
    pub expire_time: Option<String>,
    pub venue_type: Option<char>,
    pub multi_leg_reporting: Option<MultiLegReportingType>,
    pub fees: Vec<MiscFee>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelReject {
    pub cl_ord_id: Option<String>,
    pub order_id: Option<String>,
    pub orig_cl_ord_id: Option<String>,
    pub text: Option<String>,
    pub ord_status: Option<OrdStatus>,
    pub cxl_rej_reason: Option<i32>,
    pub response_to: Option<char>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOrderBatchReject {
    pub batch_id: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderCancelBatchReject {
    pub batch_id: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessReject {
    pub ref_seq_num: Option<u32>,
    pub ref_msg_type: Option<String>,
    pub business_reject_ref_id: Option<String>,
    pub reason: Option<i32>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MassCancelReport {
    pub cl_ord_id: Option<String>,
    pub request_type: Option<char>,
    pub response: Option<char>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdReject {
    pub md_req_id: Option<String>,
    pub reason: Option<char>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityStatus {
    pub md_req_id: Option<String>,
    pub rpt_seq: Option<u64>,
    pub symbol: Option<String>,
    pub trading_status: Option<String>,
    pub min_price_increment: Option<String>,
    pub min_size_increment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteRequest {
    pub quote_req_id: Option<String>,
    pub symbol: Option<String>,
    pub order_qty: Option<String>,
    pub cash_order_qty: Option<String>,
    pub valid_until: Option<String>,
    pub expire_time: Option<String>,
    pub fee_amt: Option<String>,
    pub order_capacity: Option<char>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteStatusReport {
    pub quote_req_id: Option<String>,
    pub quote_id: Option<String>,
    pub symbol: Option<String>,
    pub side: Option<Side>,
    pub status: Option<i32>,
    pub text: Option<String>,
    pub bid_px: Option<String>,
    pub offer_px: Option<String>,
    pub bid_size: Option<String>,
    pub offer_size: Option<String>,
}

pub fn envelope(message: Message) -> Envelope {
    let msg_type = message.msg_type().unwrap_or("").to_string();
    let body = match msg_type.as_str() {
        "0" => Incoming::Heartbeat {
            test_req_id: opt(&message, TEST_REQ_ID),
        },
        "1" => Incoming::TestRequest {
            test_req_id: opt(&message, TEST_REQ_ID).unwrap_or_default(),
        },
        "2" => Incoming::ResendRequest {
            begin: u32_of(&message, BEGIN_SEQ_NO).unwrap_or(0),
            end: u32_of(&message, END_SEQ_NO).unwrap_or(0),
        },
        "4" => Incoming::SequenceReset {
            gap_fill: message.get(GAP_FILL_FLAG) == Some("Y"),
            new_seq_no: u32_of(&message, NEW_SEQ_NO).unwrap_or(0),
        },
        "3" => Incoming::Reject(SessionReject {
            ref_seq_num: u32_of(&message, REF_SEQ_NUM),
            ref_tag_id: u32_of(&message, REF_TAG_ID),
            ref_msg_type: opt(&message, REF_MSG_TYPE),
            reason: i32_of(&message, SESSION_REJECT_REASON),
            text: opt(&message, TEXT),
        }),
        "5" => Incoming::Logout {
            text: opt(&message, TEXT),
        },
        "A" => Incoming::Logon(LogonAck {
            heart_bt_int: u32_of(&message, HEART_BT_INT).map(|n| n as u64),
            reset_seq_num: message.get(RESET_SEQ_NUM_FLAG).map(|v| v == "Y"),
        }),
        "8" => Incoming::ExecutionReport(Box::new(exec_report(&message))),
        "9" => Incoming::OrderCancelReject(CancelReject {
            cl_ord_id: opt(&message, CL_ORD_ID),
            order_id: opt(&message, ORDER_ID),
            orig_cl_ord_id: opt(&message, ORIG_CL_ORD_ID),
            text: opt(&message, TEXT),
            ord_status: ch(&message, ORD_STATUS).map(OrdStatus::from_wire),
            cxl_rej_reason: i32_of(&message, CXL_REJ_REASON),
            response_to: ch(&message, 434),
        }),
        "U7" => Incoming::NewOrderBatchReject(NewOrderBatchReject {
            batch_id: opt(&message, BATCH_ID),
            text: opt(&message, TEXT),
        }),
        "U5" => Incoming::OrderCancelBatchReject(OrderCancelBatchReject {
            batch_id: opt(&message, BATCH_ID),
            text: opt(&message, TEXT),
        }),
        "j" => Incoming::BusinessMessageReject(BusinessReject {
            ref_seq_num: u32_of(&message, REF_SEQ_NUM),
            ref_msg_type: opt(&message, REF_MSG_TYPE),
            business_reject_ref_id: opt(&message, BUSINESS_REJECT_REF_ID),
            reason: i32_of(&message, BUSINESS_REJECT_REASON),
            text: opt(&message, TEXT),
        }),
        "r" => Incoming::OrderMassCancelReport(MassCancelReport {
            cl_ord_id: opt(&message, CL_ORD_ID),
            request_type: ch(&message, MASS_CANCEL_REQUEST_TYPE),
            response: ch(&message, MASS_CANCEL_RESPONSE),
            text: opt(&message, TEXT),
        }),
        "X" => Incoming::MarketDataIncremental(md_incremental(&message)),
        "W" => Incoming::MarketDataSnapshot(md_snapshot(&message)),
        "Y" => Incoming::MarketDataRequestReject(MdReject {
            md_req_id: opt(&message, MD_REQ_ID),
            reason: ch(&message, MD_REQ_REJ_REASON),
            text: opt(&message, TEXT),
        }),
        "f" => Incoming::SecurityStatus(SecurityStatus {
            md_req_id: opt(&message, MD_REQ_ID),
            rpt_seq: u64_of(&message, RPT_SEQ),
            symbol: opt(&message, SYMBOL),
            trading_status: opt(&message, MD_SECURITY_TRADING_STATUS),
            min_price_increment: opt(&message, MIN_PRICE_INCREMENT),
            min_size_increment: opt(&message, MIN_SIZE_INCREMENT),
        }),
        "y" => Incoming::SecurityList(security_list(&message)),
        "R" => Incoming::QuoteRequest(QuoteRequest {
            quote_req_id: opt(&message, QUOTE_REQ_ID),
            symbol: opt(&message, SYMBOL),
            order_qty: opt(&message, ORDER_QTY),
            cash_order_qty: opt(&message, CASH_ORDER_QTY),
            valid_until: opt(&message, VALID_UNTIL_TIME),
            expire_time: opt(&message, EXPIRE_TIME),
            fee_amt: opt(&message, MISC_FEE_AMT),
            order_capacity: ch(&message, ORDER_CAPACITY),
        }),
        "AI" => Incoming::QuoteStatusReport(Box::new(QuoteStatusReport {
            quote_req_id: opt(&message, QUOTE_REQ_ID),
            quote_id: opt(&message, QUOTE_ID),
            symbol: opt(&message, SYMBOL),
            side: ch(&message, SIDE).map(|c| if c == '2' { Side::Sell } else { Side::Buy }),
            status: i32_of(&message, QUOTE_STATUS),
            text: opt(&message, TEXT),
            bid_px: opt(&message, BID_PX),
            offer_px: opt(&message, OFFER_PX),
            bid_size: opt(&message, BID_SIZE),
            offer_size: opt(&message, OFFER_SIZE),
        })),
        other => Incoming::Other {
            msg_type: other.to_string(),
        },
    };
    Envelope { message, body }
}

fn exec_report(msg: &Message) -> ExecutionReport {
    let fees = msg
        .group(NO_MISC_FEES, MISC_FEE_AMT)
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            let one = Message { fields: g };
            MiscFee {
                amt: opt(&one, MISC_FEE_AMT).unwrap_or_default(),
                currency: opt(&one, MISC_FEE_CURR),
                fee_type: opt(&one, MISC_FEE_TYPE),
                basis: i32_of(&one, MISC_FEE_BASIS),
            }
        })
        .collect();
    ExecutionReport {
        cl_ord_id: opt(msg, CL_ORD_ID),
        order_id: opt(msg, ORDER_ID),
        orig_cl_ord_id: opt(msg, ORIG_CL_ORD_ID),
        exec_id: opt(msg, EXEC_ID),
        ord_status: ch(msg, ORD_STATUS).map(OrdStatus::from_wire),
        exec_type: ch(msg, EXEC_TYPE).map(ExecType::from_wire),
        symbol: opt(msg, SYMBOL),
        side: ch(msg, SIDE).map(|c| if c == '2' { Side::Sell } else { Side::Buy }),
        ord_type: ch(msg, ORD_TYPE),
        avg_px: opt(msg, AVG_PX),
        cum_qty: opt(msg, CUM_QTY),
        leaves_qty: opt(msg, LEAVES_QTY),
        last_qty: opt(msg, LAST_QTY),
        last_px: opt(msg, LAST_PX),
        price: opt(msg, PRICE),
        order_qty: opt(msg, ORDER_QTY),
        cash_order_qty: opt(msg, CASH_ORDER_QTY),
        stop_px: opt(msg, STOP_PX),
        stop_limit_px: opt(msg, STOP_LIMIT_PX),
        text: opt(msg, TEXT),
        transact_time: opt(msg, TRANSACT_TIME),
        ord_rej_reason: i32_of(msg, ORD_REJ_REASON),
        restatement_reason: i32_of(msg, EXEC_RESTATEMENT_REASON)
            .map(ExecRestatementReason::from_wire),
        trade_id: opt(msg, TRADE_ID),
        aggressor: msg.get(AGGRESSOR_INDICATOR).map(|v| v == "Y"),
        time_in_force: ch(msg, TIME_IN_FORCE),
        trigger_direction: ch(msg, TRIGGER_PRICE_DIRECTION),
        exec_inst: ch(msg, EXEC_INST),
        self_trade: ch(msg, SELF_TRADE_TYPE),
        expire_time: opt(msg, EXPIRE_TIME),
        venue_type: ch(msg, VENUE_TYPE),
        multi_leg_reporting: ch(msg, MULTI_LEG_REPORTING_TYPE)
            .map(MultiLegReportingType::from_wire),
        fees,
    }
}

fn md_incremental(msg: &Message) -> MdIncremental {
    let entries = msg
        .group(NO_MD_ENTRIES, MD_UPDATE_ACTION)
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            let one = Message { fields: g };
            MdEntry {
                action: ch(&one, MD_UPDATE_ACTION)
                    .map(MdUpdateAction::from_wire)
                    .unwrap_or(MdUpdateAction::Other('?')),
                entry_type: ch(&one, MD_ENTRY_TYPE)
                    .map(MdEntryType::from_wire)
                    .unwrap_or(MdEntryType::Other('?')),
                entry_id: opt(&one, MD_ENTRY_ID),
                rpt_seq: u64_of(&one, RPT_SEQ),
                symbol: opt(&one, SYMBOL).unwrap_or_default(),
                price: opt(&one, MD_ENTRY_PX).unwrap_or_default(),
                size: opt(&one, MD_ENTRY_SIZE),
                transact_time: opt(&one, TRANSACT_TIME),
                ord_type: ch(&one, ORD_TYPE),
                cl_ord_id: opt(&one, CL_ORD_ID),
                order_id: opt(&one, ORDER_ID),
                text: opt(&one, TEXT),
                aggressor_side: i32_of(&one, AGGRESSOR_SIDE)
                    .map(|n| if n == 2 { Side::Sell } else { Side::Buy }),
                funds: opt(&one, FUNDS),
            }
        })
        .collect();
    MdIncremental {
        md_req_id: opt(msg, MD_REQ_ID).unwrap_or_default(),
        entries,
    }
}

fn md_snapshot(msg: &Message) -> MdSnapshot {
    let entries = msg
        .group(NO_MD_ENTRIES, MD_ENTRY_TYPE)
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            let one = Message { fields: g };
            SnapEntry {
                entry_type: ch(&one, MD_ENTRY_TYPE)
                    .map(MdEntryType::from_wire)
                    .unwrap_or(MdEntryType::Other('?')),
                entry_id: opt(&one, MD_ENTRY_ID),
                price: opt(&one, MD_ENTRY_PX).unwrap_or_default(),
                size: opt(&one, MD_ENTRY_SIZE).unwrap_or_default(),
            }
        })
        .collect();
    MdSnapshot {
        md_req_id: opt(msg, MD_REQ_ID).unwrap_or_default(),
        rpt_seq: u64_of(msg, RPT_SEQ),
        last_fragment: msg.get(LAST_FRAGMENT) != Some("N"),
        symbol: opt(msg, SYMBOL).unwrap_or_default(),
        trading_status: opt(msg, MD_SECURITY_TRADING_STATUS),
        entries,
    }
}

fn security_list(msg: &Message) -> SecurityList {
    let parent_status = opt(msg, MD_SECURITY_TRADING_STATUS);
    let instruments = msg
        .group(NO_RELATED_SYM, SYMBOL)
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            let one = Message { fields: g };
            Instrument {
                symbol: opt(&one, SYMBOL).unwrap_or_default(),
                currency: opt(&one, CURRENCY).unwrap_or_default(),
                min_trade_vol: opt(&one, MIN_TRADE_VOL).unwrap_or_default(),
                min_price_increment: opt(&one, MIN_PRICE_INCREMENT).unwrap_or_default(),
                min_size_increment: opt(&one, MIN_SIZE_INCREMENT).unwrap_or_default(),
                trading_status: opt(&one, MD_SECURITY_TRADING_STATUS)
                    .or_else(|| parent_status.clone()),
            }
        })
        .collect();
    SecurityList {
        security_req_id: opt(msg, SECURITY_REQ_ID).unwrap_or_default(),
        security_response_id: opt(msg, SECURITY_RESPONSE_ID).unwrap_or_default(),
        result: i32_of(msg, SECURITY_REQUEST_RESULT).unwrap_or(-1),
        last_fragment: msg.get(LAST_FRAGMENT) != Some("N"),
        total: u32_of(msg, TOT_NO_RELATED_SYM).unwrap_or(0),
        trading_status: parent_status,
        instruments,
    }
}

fn opt(msg: &Message, tag: u32) -> Option<String> {
    msg.get(tag).map(str::to_string)
}

fn ch(msg: &Message, tag: u32) -> Option<char> {
    let v = msg.get(tag)?;
    let mut chars = v.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

fn u32_of(msg: &Message, tag: u32) -> Option<u32> {
    msg.get(tag)?.parse().ok()
}

fn u64_of(msg: &Message, tag: u32) -> Option<u64> {
    msg.get(tag)?.parse().ok()
}

fn i32_of(msg: &Message, tag: u32) -> Option<i32> {
    msg.get(tag)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Decode, OutHeader, decode_one, encode};

    #[test]
    fn execution_report_fees_and_leg_type() {
        let header = OutHeader::from_parts("Coinbase", "key", 4, "20230822-20:43:30.000").unwrap();
        let fields = vec![
            (CL_ORD_ID, "1985ca2d-61ef-49f1-bfce-6c39d8462914".into()),
            (ORDER_ID, "2985ca2d-61ef-49f1-bfce-6c39d8462914".into()),
            (EXEC_ID, "3985ca2d-61ef-49f1-bfce-6c39d8462914".into()),
            (ORD_STATUS, "1".into()),
            (EXEC_TYPE, "F".into()),
            (SYMBOL, "QNT-USD".into()),
            (SIDE, "2".into()),
            (LAST_PX, "75".into()),
            (LAST_QTY, "10".into()),
            (MULTI_LEG_REPORTING_TYPE, "2".into()),
            (EXEC_RESTATEMENT_REASON, "3".into()),
            (NO_MISC_FEES, "1".into()),
            (MISC_FEE_AMT, "0.004".into()),
            (MISC_FEE_CURR, "USD".into()),
            (MISC_FEE_TYPE, "4".into()),
            (MISC_FEE_BASIS, "2".into()),
        ];
        let bytes = encode(&header, "8", &fields).unwrap();
        let Decode::Frame { message, .. } = decode_one(&bytes).unwrap() else {
            panic!("frame")
        };
        let Envelope { body, .. } = envelope(message);
        let Incoming::ExecutionReport(rep) = body else {
            panic!("exec")
        };
        assert_eq!(rep.exec_type, Some(ExecType::Trade));
        assert_eq!(
            rep.multi_leg_reporting,
            Some(MultiLegReportingType::IndividualLeg)
        );
        assert_eq!(
            rep.restatement_reason,
            Some(ExecRestatementReason::Repricing)
        );
        assert_eq!(rep.fees.len(), 1);
        assert_eq!(rep.fees[0].basis, Some(2));
        assert_eq!(rep.side, Some(Side::Sell));
    }
}
