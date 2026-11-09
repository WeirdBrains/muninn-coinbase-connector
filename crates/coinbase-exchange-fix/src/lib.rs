//! Coinbase Exchange FIX 5.0 SP2 client.
//!
//! This crate speaks the Exchange order-entry, market-data, and drop-copy gateways.
//! It does not know about Muninn. A connector can depend on it; nothing here depends
//! on a connector.
//!
//! Wire sessions are `FIXT.1.1` with `DefaultApplVerID=9` (FIX 5.0 SP2), signed with
//! the Exchange HMAC logon, over TLS 1.2. Coinbase publishes ECDHE-RSA AES-GCM
//! suites; this client offers those, and also ECDHE-ECDSA AES-128-GCM so a peer
//! with an ECDSA certificate can finish the handshake. Coinbase's RSA certificate
//! selects one of the published suites.
//!
//! Message builders check the order shapes Coinbase documents (take-profit/stop-loss
//! price order, limit-with-funds, synthetic products, batch size). The session layer
//! covers logon, heartbeats, test requests, sequence reset, and paged resend.

#![forbid(unsafe_code)]

mod auth;
mod book;
mod codec;
mod decimal;
mod dictionary;
mod endpoint;
mod engine;
mod error;
mod limits;
mod market;
mod order;
mod parse;
mod schedule;
mod session;
mod tags;
mod timefmt;

pub use auth::{CancelOnDisconnect, Credentials, sign_logon};
pub use book::{
    L2Book, L2BookBuilder, L3Book, L3BookBuilder, PriceLevel, RestingOrder, TradePrint,
    resting_order,
};
pub use codec::{AppMessage, Decode, Message, OutHeader, SOH, checksum, decode_one, encode};
pub use dictionary::{DICTIONARY_ARCHIVE, DICTIONARY_FILES, DICTIONARY_URL, DICTIONARY_VERSION};
pub use endpoint::{Endpoint, Environment, GatewayKind, Venue};
pub use error::{Error, Result};
pub use limits::{
    DISCONNECT_PER_SECOND, LOGONS_PER_SECOND, MAX_BATCH_ORDERS, MAX_CONNECTIONS_PER_PROFILE,
    MAX_CONNECTIONS_PER_USER, MAX_PROFILES_PER_USER, MAX_SEQ, REQUESTS_PER_SECOND, RateLimiter,
    Traffic,
};
pub use market::{
    AUCTION_MODE, CANCEL_ONLY, FULL_TRADING, Instrument, LIMIT_ONLY, MarketDataRequest,
    MarketDepth, MdEntry, MdIncremental, MdSnapshot, POST_ONLY, SecurityList, SecurityListAccum,
    SecurityListRequest, Subscription, TRADING_DISABLED,
};
pub use order::{
    BatchLeg, CancelBatch, CancelBatchItem, CancelOrder, CancelReplace, LimitOrder, LimitWithFunds,
    MarketOrder, MassCancel, NewOrderBatch, OrderSize, OrderStatusRequest, Quote, ReplaceType,
    RfqRequest, SelfTradePrevention, Side, StopLimitOrder, SyntheticOrder, SyntheticSelfTrade,
    SyntheticType, TakeProfitStopLoss, TimeInForce, TriggerDirection, new_client_order_id,
    validate_uuid_v4,
};
pub use parse::{
    BusinessReject, CancelReject, Envelope, ExecRestatementReason, ExecType, ExecutionReport,
    Incoming, LogonAck, MassCancelReport, MdReject, MiscFee, MultiLegReportingType,
    NewOrderBatchReject, OrdStatus, OrderCancelBatchReject, QuoteRequest, QuoteStatusReport,
    SecurityStatus, SessionReject, envelope,
};
pub use schedule::{next_deploy_logout, next_weekly_logout};
pub use session::{FixSession, SessionConfig};
pub use timefmt::{format_millis, parse_timestamp};

/// Session version written in `BeginString` (8).
pub const BEGIN_STRING: &str = "FIXT.1.1";
/// `TargetCompID` (56) on every client message.
pub const TARGET_COMP_ID: &str = "Coinbase";
/// `DefaultApplVerID` (1137): FIX 5.0 SP2.
pub const APPL_VER_FIX50SP2: &str = "9";
