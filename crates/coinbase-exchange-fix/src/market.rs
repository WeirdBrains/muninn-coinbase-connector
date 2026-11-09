//! Market data requests and the messages the gateway streams back.
//!
//! Depth: omit or `0` for L3 (full order-by-order), `1` for top of book, `10` for the
//! aggregated top 10 levels per side. L3 `RptSeq` is per symbol. L2 and L1 omit it.
//! Incremental updates that arrive before the snapshot should be queued; see [`crate::book`].

use crate::codec::AppMessage;
use crate::error::{Error, Result};
use crate::order::Side;
use crate::tags::*;

pub const TRADING_DISABLED: &str = "trading_disabled";
pub const CANCEL_ONLY: &str = "cancel_only";
pub const POST_ONLY: &str = "post_only";
pub const LIMIT_ONLY: &str = "limit_only";
pub const FULL_TRADING: &str = "full_trading";
pub const AUCTION_MODE: &str = "auction_mode";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subscription {
    Subscribe,
    Unsubscribe,
}

impl Subscription {
    fn wire(self) -> &'static str {
        match self {
            Subscription::Subscribe => "1",
            Subscription::Unsubscribe => "2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketDepth {
    /// Order-by-order. Sent as `MarketDepth=0`.
    L3,
    /// Aggregated top 10 price levels per side.
    L2,
    /// Best bid and best offer.
    L1,
}

impl MarketDepth {
    fn wire(self) -> &'static str {
        match self {
            MarketDepth::L3 => "0",
            MarketDepth::L2 => "10",
            MarketDepth::L1 => "1",
        }
    }
}

fn req_id(s: &str) -> Result<()> {
    if s.is_empty() || s.bytes().any(|b| b == crate::codec::SOH) {
        return Err(Error::rejected("request id is empty"));
    }
    Ok(())
}

fn product(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 32 || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Error::rejected(format!(
            "symbol `{s}` is empty or not a product id"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketDataRequest {
    pub md_req_id: String,
    pub subscription: Subscription,
    pub depth: MarketDepth,
    pub symbols: Vec<String>,
}

impl MarketDataRequest {
    pub fn to_message(&self) -> Result<AppMessage> {
        req_id(&self.md_req_id)?;
        if self.symbols.is_empty() {
            return Err(Error::rejected("a market data request needs a symbol"));
        }
        let mut fields = vec![
            (MD_REQ_ID, self.md_req_id.clone()),
            (SUBSCRIPTION_REQUEST_TYPE, self.subscription.wire().into()),
            (MARKET_DEPTH, self.depth.wire().into()),
            (NO_RELATED_SYM, self.symbols.len().to_string()),
        ];
        for s in &self.symbols {
            product(s)?;
            fields.push((SYMBOL, s.clone()));
        }
        Ok(AppMessage::new("V", fields))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityListRequest {
    pub security_req_id: String,
}

impl SecurityListRequest {
    pub fn to_message(&self) -> Result<AppMessage> {
        req_id(&self.security_req_id)?;
        Ok(AppMessage::new(
            "x",
            vec![
                (SECURITY_REQ_ID, self.security_req_id.clone()),
                (SECURITY_LIST_REQUEST_TYPE, "4".into()),
            ],
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdUpdateAction {
    New,
    Change,
    Delete,
    Other(char),
}

impl MdUpdateAction {
    pub(crate) fn from_wire(c: char) -> Self {
        match c {
            '0' => Self::New,
            '1' => Self::Change,
            '2' => Self::Delete,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdEntryType {
    Bid,
    Offer,
    Trade,
    Other(char),
}

impl MdEntryType {
    pub(crate) fn from_wire(c: char) -> Self {
        match c {
            '0' => Self::Bid,
            '1' => Self::Offer,
            '2' => Self::Trade,
            other => Self::Other(other),
        }
    }

    pub fn side(self) -> Option<Side> {
        match self {
            MdEntryType::Bid => Some(Side::Buy),
            MdEntryType::Offer => Some(Side::Sell),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdEntry {
    pub action: MdUpdateAction,
    pub entry_type: MdEntryType,
    /// Absent on the pre-match acknowledgement, and on L2/L1.
    pub entry_id: Option<String>,
    pub rpt_seq: Option<u64>,
    pub symbol: String,
    pub price: String,
    pub size: Option<String>,
    pub transact_time: Option<String>,
    pub ord_type: Option<char>,
    pub cl_ord_id: Option<String>,
    pub order_id: Option<String>,
    pub text: Option<String>,
    pub aggressor_side: Option<Side>,
    pub funds: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdIncremental {
    pub md_req_id: String,
    pub entries: Vec<MdEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapEntry {
    pub entry_type: MdEntryType,
    pub entry_id: Option<String>,
    pub price: String,
    pub size: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdSnapshot {
    pub md_req_id: String,
    pub rpt_seq: Option<u64>,
    pub last_fragment: bool,
    pub symbol: String,
    pub trading_status: Option<String>,
    pub entries: Vec<SnapEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instrument {
    pub symbol: String,
    pub currency: String,
    pub min_trade_vol: String,
    pub min_price_increment: String,
    pub min_size_increment: String,
    pub trading_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityList {
    pub security_req_id: String,
    pub security_response_id: String,
    pub result: i32,
    pub last_fragment: bool,
    pub total: u32,
    pub trading_status: Option<String>,
    pub instruments: Vec<Instrument>,
}

/// Collects fragmented security-list messages. [`Self::push`] returns the full list
/// once `LastFragment=Y` arrives.
#[derive(Debug, Default, Clone)]
pub struct SecurityListAccum {
    parts: Vec<Instrument>,
    status: Option<String>,
}

impl SecurityListAccum {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, list: SecurityList) -> Option<Vec<Instrument>> {
        if self.status.is_none() {
            self.status = list.trading_status.clone();
        }
        self.parts.extend(list.instruments);
        if list.last_fragment {
            Some(std::mem::take(&mut self.parts))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_data_request_sets_depth() {
        let msg = MarketDataRequest {
            md_req_id: "book-1".into(),
            subscription: Subscription::Subscribe,
            depth: MarketDepth::L2,
            symbols: vec!["BTC-USD".into(), "ETH-USD".into()],
        }
        .to_message()
        .unwrap();
        assert_eq!(msg.msg_type, "V");
        assert!(
            msg.fields
                .iter()
                .any(|(t, v)| *t == MARKET_DEPTH && v == "10")
        );
        assert_eq!(msg.fields.iter().filter(|(t, _)| *t == SYMBOL).count(), 2);
    }
}
