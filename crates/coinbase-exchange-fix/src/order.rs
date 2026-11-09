//! Order-entry messages: single orders, batches, cancels, replaces, status,
//! mass cancel, and liquidity-provider quotes.
//!
//! Take-profit/stop-loss, limit-with-funds, and synthetic products are their own
//! types so the illegal combinations cannot be built by filling in the wrong fields.
//! Batches are capped at 15 and must share one symbol. A batch counts as one
//! request for rate limits.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::codec::AppMessage;
use crate::decimal::{is_multiple, positive, significant_digits};
use crate::error::{Error, Result};
use crate::limits::MAX_BATCH_ORDERS;
use crate::tags::*;
use crate::timefmt::format_millis;

const UUID_EXAMPLE: &str = "1985ca2d-61ef-49f1-bfce-6c39d8462914";

/// A new lowercase variant-1 UUID v4, the only `ClOrdID` shape the gateway accepts.
pub fn new_client_order_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn validate_uuid_v4(id: &str) -> Result<()> {
    if id.len() != 36 || id != id.to_ascii_lowercase() {
        return Err(Error::rejected(format!(
            "`{id}` must be a lowercase UUID like {UUID_EXAMPLE}"
        )));
    }
    let uuid = Uuid::parse_str(id).map_err(|_| Error::rejected(format!("`{id}` is not a UUID")))?;
    if uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(Error::rejected(format!(
            "`{id}` must be a variant-1 UUID v4"
        )));
    }
    Ok(())
}

fn symbol(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > 32 || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Error::rejected(format!(
            "symbol `{s}` is empty or not a product id"
        )));
    }
    Ok(())
}

fn id_field(fields: &mut Vec<(u32, String)>, tag: u32, id: &str) -> Result<()> {
    validate_uuid_v4(id)?;
    fields.push((tag, id.to_string()));
    Ok(())
}

fn dec_field(fields: &mut Vec<(u32, String)>, tag: u32, value: &str, what: &str) -> Result<()> {
    positive(value, what)?;
    fields.push((tag, value.to_string()));
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    fn wire(self) -> &'static str {
        match self {
            Side::Buy => "1",
            Side::Sell => "2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    GoodTillCancel,
    ImmediateOrCancel,
    FillOrKill,
    GoodTillDate,
}

impl TimeInForce {
    fn wire(self) -> &'static str {
        match self {
            TimeInForce::GoodTillCancel => "1",
            TimeInForce::ImmediateOrCancel => "3",
            TimeInForce::FillOrKill => "4",
            TimeInForce::GoodTillDate => "6",
        }
    }
}

/// `SelfTradeType` (7928). Omitted on the wire, the gateway uses decrement-and-cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfTradePrevention {
    DecrementAndCancel,
    CancelOldest,
    CancelNewest,
    CancelBoth,
}

impl SelfTradePrevention {
    fn wire(self) -> &'static str {
        match self {
            SelfTradePrevention::DecrementAndCancel => "D",
            SelfTradePrevention::CancelOldest => "O",
            SelfTradePrevention::CancelNewest => "N",
            SelfTradePrevention::CancelBoth => "B",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerDirection {
    Up,
    Down,
}

impl TriggerDirection {
    fn wire(self) -> &'static str {
        match self {
            TriggerDirection::Up => "U",
            TriggerDirection::Down => "D",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderSize {
    /// `OrderQty` (38), base units.
    Base(String),
    /// `CashOrderQty` (152), quote units. Market orders only, outside limit-with-funds.
    Cash(String),
}

fn push_tif(
    fields: &mut Vec<(u32, String)>,
    tif: TimeInForce,
    expire: Option<DateTime<Utc>>,
) -> Result<()> {
    fields.push((TIME_IN_FORCE, tif.wire().into()));
    match (tif, expire) {
        (TimeInForce::GoodTillDate, Some(t)) => fields.push((EXPIRE_TIME, format_millis(t))),
        (TimeInForce::GoodTillDate, None) => {
            return Err(Error::rejected("GTD orders need an expire time"));
        }
        (_, Some(_)) => return Err(Error::rejected("expire time is only valid on GTD orders")),
        _ => {}
    }
    Ok(())
}

fn push_stp(fields: &mut Vec<(u32, String)>, stp: Option<SelfTradePrevention>) {
    if let Some(stp) = stp {
        fields.push((SELF_TRADE_TYPE, stp.wire().into()));
    }
}

fn exn(
    fields: &mut Vec<(u32, String)>,
    on: bool,
    sender_sub_id: &Option<String>,
) -> Result<Option<String>> {
    if !on {
        return Ok(None);
    }
    let Some(id) = sender_sub_id else {
        return Err(Error::rejected("EXN orders need a sender sub id"));
    };
    validate_uuid_v4(id)?;
    fields.push((HANDL_INST, "E".into()));
    Ok(Some(id.clone()))
}

fn finish(msg_type: &str, fields: Vec<(u32, String)>, sender_sub_id: Option<String>) -> AppMessage {
    let mut msg = AppMessage::new(msg_type, fields);
    msg.sender_sub_id = sender_sub_id;
    msg
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketOrder {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub qty: OrderSize,
    pub time_in_force: TimeInForce,
    pub expire_time: Option<DateTime<Utc>>,
    pub self_trade: Option<SelfTradePrevention>,
    pub exn: bool,
    pub sender_sub_id: Option<String>,
}

impl MarketOrder {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        let sub = exn(&mut fields, self.exn, &self.sender_sub_id)?;
        match &self.qty {
            OrderSize::Base(q) => dec_field(&mut fields, ORDER_QTY, q, "order qty")?,
            OrderSize::Cash(q) => dec_field(&mut fields, CASH_ORDER_QTY, q, "cash order qty")?,
        }
        fields.push((ORD_TYPE, "1".into()));
        fields.push((SIDE, self.side.wire().into()));
        fields.push((SYMBOL, self.symbol.clone()));
        push_tif(&mut fields, self.time_in_force, self.expire_time)?;
        push_stp(&mut fields, self.self_trade);
        Ok(finish("D", fields, sub))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitOrder {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub order_qty: String,
    pub price: String,
    pub time_in_force: TimeInForce,
    pub expire_time: Option<DateTime<Utc>>,
    pub post_only: bool,
    pub self_trade: Option<SelfTradePrevention>,
    pub exn: bool,
    pub sender_sub_id: Option<String>,
}

impl LimitOrder {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        if self.post_only {
            fields.push((EXEC_INST, "A".into()));
        }
        let sub = exn(&mut fields, self.exn, &self.sender_sub_id)?;
        dec_field(&mut fields, ORDER_QTY, &self.order_qty, "order qty")?;
        fields.push((ORD_TYPE, "2".into()));
        dec_field(&mut fields, PRICE, &self.price, "price")?;
        fields.push((SIDE, self.side.wire().into()));
        fields.push((SYMBOL, self.symbol.clone()));
        push_tif(&mut fields, self.time_in_force, self.expire_time)?;
        push_stp(&mut fields, self.self_trade);
        Ok(finish("D", fields, sub))
    }
}

/// Limit order sized in quote units (`CashOrderQty`). Do not also send `OrderQty`.
/// The gateway computes base size. Not valid for stop, TPSL, batch, or replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitWithFunds {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub price: String,
    pub cash_order_qty: String,
    pub time_in_force: TimeInForce,
    pub expire_time: Option<DateTime<Utc>>,
    pub post_only: bool,
    pub self_trade: Option<SelfTradePrevention>,
}

impl LimitWithFunds {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        if self.post_only {
            fields.push((EXEC_INST, "A".into()));
        }
        dec_field(
            &mut fields,
            CASH_ORDER_QTY,
            &self.cash_order_qty,
            "cash order qty",
        )?;
        fields.push((ORD_TYPE, "2".into()));
        dec_field(&mut fields, PRICE, &self.price, "price")?;
        fields.push((SIDE, self.side.wire().into()));
        fields.push((SYMBOL, self.symbol.clone()));
        push_tif(&mut fields, self.time_in_force, self.expire_time)?;
        push_stp(&mut fields, self.self_trade);
        Ok(finish("D", fields, None))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopLimitOrder {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub order_qty: String,
    pub price: String,
    pub stop_px: String,
    pub trigger: Option<TriggerDirection>,
    pub time_in_force: TimeInForce,
    pub expire_time: Option<DateTime<Utc>>,
    pub post_only: bool,
    pub self_trade: Option<SelfTradePrevention>,
}

impl StopLimitOrder {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        if self.post_only {
            fields.push((EXEC_INST, "A".into()));
        }
        dec_field(&mut fields, ORDER_QTY, &self.order_qty, "order qty")?;
        fields.push((ORD_TYPE, "4".into()));
        dec_field(&mut fields, PRICE, &self.price, "price")?;
        fields.push((SIDE, self.side.wire().into()));
        fields.push((SYMBOL, self.symbol.clone()));
        push_tif(&mut fields, self.time_in_force, self.expire_time)?;
        dec_field(&mut fields, STOP_PX, &self.stop_px, "stop price")?;
        if let Some(dir) = self.trigger {
            fields.push((TRIGGER_PRICE_DIRECTION, dir.wire().into()));
        }
        push_stp(&mut fields, self.self_trade);
        Ok(finish("D", fields, None))
    }
}

/// Take-profit / stop-loss. `price` is the live take-profit limit.
/// Sell requires `price > stop_px > stop_limit_px`. Buy is the reverse.
/// GTC or GTD only. Post-only, batches, and trigger direction are rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeProfitStopLoss {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub order_qty: String,
    pub price: String,
    pub stop_px: String,
    pub stop_limit_px: String,
    pub time_in_force: TimeInForce,
    pub expire_time: Option<DateTime<Utc>>,
    pub self_trade: Option<SelfTradePrevention>,
}

impl TakeProfitStopLoss {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        if !matches!(
            self.time_in_force,
            TimeInForce::GoodTillCancel | TimeInForce::GoodTillDate
        ) {
            return Err(Error::rejected("TPSL orders are GTC or GTD only"));
        }
        check_tpsl_prices(self.side, &self.price, &self.stop_px, &self.stop_limit_px)?;
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        dec_field(&mut fields, ORDER_QTY, &self.order_qty, "order qty")?;
        fields.push((ORD_TYPE, "O".into()));
        dec_field(&mut fields, PRICE, &self.price, "take profit price")?;
        fields.push((SIDE, self.side.wire().into()));
        fields.push((SYMBOL, self.symbol.clone()));
        push_tif(&mut fields, self.time_in_force, self.expire_time)?;
        dec_field(&mut fields, STOP_PX, &self.stop_px, "stop price")?;
        dec_field(
            &mut fields,
            STOP_LIMIT_PX,
            &self.stop_limit_px,
            "stop limit price",
        )?;
        push_stp(&mut fields, self.self_trade);
        Ok(finish("D", fields, None))
    }
}

pub(crate) fn check_tpsl_prices(
    side: Side,
    price: &str,
    stop: &str,
    stop_limit: &str,
) -> Result<()> {
    let price = positive(price, "take profit price")?;
    let stop = positive(stop, "stop price")?;
    let stop_limit = positive(stop_limit, "stop limit price")?;
    let ok = match side {
        Side::Sell => price > stop && stop > stop_limit,
        Side::Buy => price < stop && stop < stop_limit,
    };
    if ok {
        Ok(())
    } else {
        Err(Error::rejected(match side {
            Side::Sell => "sell TPSL requires price > stop price > stop limit price",
            Side::Buy => "buy TPSL requires price < stop price < stop limit price",
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticType {
    Market,
    Limit,
}

/// Self-trade for a synthetic product. Decrement-and-cancel and cancel-both are rejected,
/// and omitting the tag defaults to decrement-and-cancel, so one of these is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticSelfTrade {
    CancelOldest,
    CancelNewest,
}

impl SyntheticSelfTrade {
    fn wire(self) -> &'static str {
        match self {
            SyntheticSelfTrade::CancelOldest => "O",
            SyntheticSelfTrade::CancelNewest => "N",
        }
    }
}

/// Sell-only order on an unlisted pair (`SecurityType=MLEG`). Sandbox only.
/// Limit orders are fill-or-kill. `base_increment` is the base leg's size increment;
/// when set, `order_qty` must be a positive multiple of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticOrder {
    pub cl_ord_id: String,
    pub symbol: String,
    pub ord_type: SyntheticType,
    pub order_qty: String,
    pub price: Option<String>,
    pub self_trade: SyntheticSelfTrade,
    pub base_increment: Option<String>,
}

impl SyntheticOrder {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        positive(&self.order_qty, "order qty")?;
        if significant_digits(&self.order_qty)? > 15 {
            return Err(Error::rejected(
                "synthetic order qty has more than 15 significant digits",
            ));
        }
        if let Some(inc) = &self.base_increment
            && !is_multiple(&self.order_qty, inc)?
        {
            return Err(Error::rejected(
                "synthetic order qty must be a multiple of the base increment",
            ));
        }
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        fields.push((SECURITY_TYPE, "MLEG".into()));
        dec_field(&mut fields, ORDER_QTY, &self.order_qty, "order qty")?;
        match self.ord_type {
            SyntheticType::Market => {
                if self.price.is_some() {
                    return Err(Error::rejected(
                        "synthetic market orders do not take a price",
                    ));
                }
                fields.push((ORD_TYPE, "1".into()));
            }
            SyntheticType::Limit => {
                let Some(price) = &self.price else {
                    return Err(Error::rejected("synthetic limit orders need a price"));
                };
                if significant_digits(price)? > 15 {
                    return Err(Error::rejected(
                        "synthetic price has more than 15 significant digits",
                    ));
                }
                fields.push((ORD_TYPE, "2".into()));
                dec_field(&mut fields, PRICE, price, "price")?;
            }
        }
        fields.push((SIDE, Side::Sell.wire().into()));
        fields.push((SYMBOL, self.symbol.clone()));
        if self.ord_type == SyntheticType::Limit {
            fields.push((TIME_IN_FORCE, TimeInForce::FillOrKill.wire().into()));
        }
        fields.push((SELF_TRADE_TYPE, self.self_trade.wire().into()));
        Ok(finish("D", fields, None))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchLeg {
    Market {
        cl_ord_id: String,
        side: Side,
        qty: OrderSize,
        time_in_force: TimeInForce,
        expire_time: Option<DateTime<Utc>>,
        self_trade: Option<SelfTradePrevention>,
    },
    Limit {
        cl_ord_id: String,
        side: Side,
        order_qty: String,
        price: String,
        time_in_force: TimeInForce,
        expire_time: Option<DateTime<Utc>>,
        post_only: bool,
        self_trade: Option<SelfTradePrevention>,
    },
    StopLimit {
        cl_ord_id: String,
        side: Side,
        order_qty: String,
        price: String,
        stop_px: String,
        time_in_force: TimeInForce,
        expire_time: Option<DateTime<Utc>>,
        self_trade: Option<SelfTradePrevention>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOrderBatch {
    pub batch_id: String,
    pub symbol: String,
    pub orders: Vec<BatchLeg>,
}

impl NewOrderBatch {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        if self.orders.is_empty() {
            return Err(Error::rejected("a batch needs at least one order"));
        }
        if self.orders.len() > MAX_BATCH_ORDERS {
            return Err(Error::rejected(format!(
                "a batch holds at most {MAX_BATCH_ORDERS} orders"
            )));
        }
        let mut seen = Vec::new();
        let mut fields = Vec::new();
        id_field(&mut fields, BATCH_ID, &self.batch_id)?;
        fields.push((NO_ORDERS, self.orders.len().to_string()));
        for leg in &self.orders {
            let cl = match leg {
                BatchLeg::Market { cl_ord_id, .. }
                | BatchLeg::Limit { cl_ord_id, .. }
                | BatchLeg::StopLimit { cl_ord_id, .. } => cl_ord_id,
            };
            if seen.iter().any(|s: &String| s == cl) {
                return Err(Error::rejected(format!(
                    "duplicate ClOrdID `{cl}` in the batch"
                )));
            }
            seen.push(cl.clone());
            leg.write(&self.symbol, &mut fields)?;
        }
        Ok(AppMessage::new("U6", fields))
    }
}

impl BatchLeg {
    fn write(&self, symbol_id: &str, fields: &mut Vec<(u32, String)>) -> Result<()> {
        match self {
            BatchLeg::Market {
                cl_ord_id,
                side,
                qty,
                time_in_force,
                expire_time,
                self_trade,
            } => {
                id_field(fields, CL_ORD_ID, cl_ord_id)?;
                match qty {
                    OrderSize::Base(q) => dec_field(fields, ORDER_QTY, q, "order qty")?,
                    OrderSize::Cash(q) => dec_field(fields, CASH_ORDER_QTY, q, "cash order qty")?,
                }
                fields.push((ORD_TYPE, "1".into()));
                fields.push((SIDE, side.wire().into()));
                fields.push((SYMBOL, symbol_id.into()));
                push_tif(fields, *time_in_force, *expire_time)?;
                push_stp(fields, *self_trade);
            }
            BatchLeg::Limit {
                cl_ord_id,
                side,
                order_qty,
                price,
                time_in_force,
                expire_time,
                post_only,
                self_trade,
            } => {
                id_field(fields, CL_ORD_ID, cl_ord_id)?;
                if *post_only {
                    fields.push((EXEC_INST, "A".into()));
                }
                dec_field(fields, ORDER_QTY, order_qty, "order qty")?;
                fields.push((ORD_TYPE, "2".into()));
                dec_field(fields, PRICE, price, "price")?;
                fields.push((SIDE, side.wire().into()));
                fields.push((SYMBOL, symbol_id.into()));
                push_tif(fields, *time_in_force, *expire_time)?;
                push_stp(fields, *self_trade);
            }
            BatchLeg::StopLimit {
                cl_ord_id,
                side,
                order_qty,
                price,
                stop_px,
                time_in_force,
                expire_time,
                self_trade,
            } => {
                id_field(fields, CL_ORD_ID, cl_ord_id)?;
                dec_field(fields, ORDER_QTY, order_qty, "order qty")?;
                fields.push((ORD_TYPE, "4".into()));
                dec_field(fields, PRICE, price, "price")?;
                fields.push((SIDE, side.wire().into()));
                fields.push((SYMBOL, symbol_id.into()));
                push_tif(fields, *time_in_force, *expire_time)?;
                dec_field(fields, STOP_PX, stop_px, "stop price")?;
                push_stp(fields, *self_trade);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrder {
    pub cl_ord_id: String,
    pub order_id: Option<String>,
    pub orig_cl_ord_id: Option<String>,
    pub symbol: String,
}

impl CancelOrder {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        if self.order_id.is_none() && self.orig_cl_ord_id.is_none() {
            return Err(Error::rejected(
                "cancel needs an order id or the original client order id",
            ));
        }
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        if let Some(id) = &self.order_id {
            id_field(&mut fields, ORDER_ID, id)?;
        }
        if let Some(id) = &self.orig_cl_ord_id {
            id_field(&mut fields, ORIG_CL_ORD_ID, id)?;
        }
        fields.push((SYMBOL, self.symbol.clone()));
        Ok(AppMessage::new("F", fields))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelBatchItem {
    pub cl_ord_id: String,
    pub order_id: Option<String>,
    pub orig_cl_ord_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelBatch {
    pub batch_id: String,
    pub symbol: String,
    pub orders: Vec<CancelBatchItem>,
}

impl CancelBatch {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        if self.orders.is_empty() || self.orders.len() > MAX_BATCH_ORDERS {
            return Err(Error::rejected(format!(
                "a cancel batch holds 1..={MAX_BATCH_ORDERS} orders"
            )));
        }
        let mut fields = Vec::new();
        id_field(&mut fields, BATCH_ID, &self.batch_id)?;
        fields.push((NO_ORDERS, self.orders.len().to_string()));
        for item in &self.orders {
            if item.order_id.is_none() && item.orig_cl_ord_id.is_none() {
                return Err(Error::rejected(
                    "each cancel needs an order id or the original client order id",
                ));
            }
            id_field(&mut fields, CL_ORD_ID, &item.cl_ord_id)?;
            if let Some(id) = &item.order_id {
                id_field(&mut fields, ORDER_ID, id)?;
            }
            if let Some(id) = &item.orig_cl_ord_id {
                id_field(&mut fields, ORIG_CL_ORD_ID, id)?;
            }
            fields.push((SYMBOL, self.symbol.clone()));
        }
        Ok(AppMessage::new("U4", fields))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceType {
    Limit,
    StopLimit,
    TakeProfitStopLoss,
}

/// Modify one resting order. `side` is not sent; it is only used to check TPSL prices.
/// Queue priority is kept when size is reduced and lost when size or price changes upward.
/// A triggered stop or TPSL cannot be modified; the gateway rejects that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelReplace {
    pub cl_ord_id: String,
    pub order_id: String,
    pub orig_cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub order_qty: String,
    pub price: String,
    pub ord_type: ReplaceType,
    pub stop_px: Option<String>,
    pub stop_limit_px: Option<String>,
}

impl CancelReplace {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        id_field(&mut fields, ORDER_ID, &self.order_id)?;
        id_field(&mut fields, ORIG_CL_ORD_ID, &self.orig_cl_ord_id)?;
        dec_field(&mut fields, ORDER_QTY, &self.order_qty, "order qty")?;
        dec_field(&mut fields, PRICE, &self.price, "price")?;
        fields.push((SYMBOL, self.symbol.clone()));
        match self.ord_type {
            ReplaceType::Limit => {
                if self.stop_px.is_some() || self.stop_limit_px.is_some() {
                    return Err(Error::rejected("limit replace does not take stop prices"));
                }
                fields.push((ORD_TYPE, "2".into()));
            }
            ReplaceType::StopLimit => {
                let Some(stop) = &self.stop_px else {
                    return Err(Error::rejected("stop-limit replace needs a stop price"));
                };
                fields.push((ORD_TYPE, "4".into()));
                dec_field(&mut fields, STOP_PX, stop, "stop price")?;
            }
            ReplaceType::TakeProfitStopLoss => {
                let (Some(stop), Some(limit)) = (&self.stop_px, &self.stop_limit_px) else {
                    return Err(Error::rejected(
                        "TPSL replace needs a stop price and a stop limit price",
                    ));
                };
                check_tpsl_prices(self.side, &self.price, stop, limit)?;
                fields.push((ORD_TYPE, "O".into()));
                dec_field(&mut fields, STOP_PX, stop, "stop price")?;
                dec_field(&mut fields, STOP_LIMIT_PX, limit, "stop limit price")?;
            }
        }
        Ok(AppMessage::new("G", fields))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderStatusRequest {
    pub cl_ord_id: Option<String>,
    pub order_id: Option<String>,
    pub symbol: String,
}

impl OrderStatusRequest {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        if self.cl_ord_id.is_none() && self.order_id.is_none() {
            return Err(Error::rejected(
                "status needs a client order id or an exchange order id",
            ));
        }
        let mut fields = Vec::new();
        if let Some(id) = &self.cl_ord_id {
            id_field(&mut fields, CL_ORD_ID, id)?;
        }
        if let Some(id) = &self.order_id {
            id_field(&mut fields, ORDER_ID, id)?;
        }
        fields.push((SYMBOL, self.symbol.clone()));
        Ok(AppMessage::new("H", fields))
    }
}

/// Cancel every order on this trading session (`MassCancelRequestType=6`).
/// Orders not yet acknowledged are not guaranteed to cancel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MassCancel {
    pub cl_ord_id: String,
    pub transact_time: DateTime<Utc>,
}

impl MassCancel {
    pub fn to_message(&self) -> Result<AppMessage> {
        let mut fields = Vec::new();
        id_field(&mut fields, CL_ORD_ID, &self.cl_ord_id)?;
        fields.push((MASS_CANCEL_REQUEST_TYPE, "6".into()));
        fields.push((TRANSACT_TIME, format_millis(self.transact_time)));
        Ok(AppMessage::new("q", fields))
    }
}

/// Liquidity-provider quote. At least one of bid or offer, with size and price together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub quote_req_id: String,
    pub quote_id: String,
    pub symbol: String,
    pub bid_px: Option<String>,
    pub bid_size: Option<String>,
    pub offer_px: Option<String>,
    pub offer_size: Option<String>,
}

impl Quote {
    pub fn to_message(&self) -> Result<AppMessage> {
        symbol(&self.symbol)?;
        let mut fields = Vec::new();
        id_field(&mut fields, QUOTE_REQ_ID, &self.quote_req_id)?;
        id_field(&mut fields, QUOTE_ID, &self.quote_id)?;
        fields.push((SYMBOL, self.symbol.clone()));
        let bid = side_px(
            &mut fields,
            self.bid_px.as_deref(),
            self.bid_size.as_deref(),
            BID_PX,
            BID_SIZE,
            "bid",
        )?;
        let offer = side_px(
            &mut fields,
            self.offer_px.as_deref(),
            self.offer_size.as_deref(),
            OFFER_PX,
            OFFER_SIZE,
            "offer",
        )?;
        if !bid && !offer {
            return Err(Error::rejected("a quote needs a bid, an offer, or both"));
        }
        Ok(AppMessage::new("S", fields))
    }
}

fn side_px(
    fields: &mut Vec<(u32, String)>,
    px: Option<&str>,
    size: Option<&str>,
    px_tag: u32,
    size_tag: u32,
    name: &str,
) -> Result<bool> {
    match (px, size) {
        (None, None) => Ok(false),
        (Some(px), Some(size)) => {
            dec_field(fields, px_tag, px, &format!("{name} price"))?;
            dec_field(fields, size_tag, size, &format!("{name} size"))?;
            Ok(true)
        }
        _ => Err(Error::rejected(format!(
            "{name} needs both a price and a size"
        ))),
    }
}

/// Sent after logon on a session that should receive quote requests. No reply is the success signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RfqRequest {
    pub rfq_req_id: String,
}

impl RfqRequest {
    pub fn to_message(&self) -> Result<AppMessage> {
        let mut fields = Vec::new();
        id_field(&mut fields, RFQ_REQ_ID, &self.rfq_req_id)?;
        Ok(AppMessage::new("AH", fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> String {
        "1985ca2d-61ef-49f1-bfce-6c39d8462914".into()
    }
    fn id2() -> String {
        "2985ca2d-61ef-49f1-bfce-6c39d8462914".into()
    }

    #[test]
    fn tpsl_price_order_and_time_in_force() {
        let mut order = TakeProfitStopLoss {
            cl_ord_id: id(),
            symbol: "BTC-USD".into(),
            side: Side::Sell,
            order_qty: "0.01".into(),
            price: "9785".into(),
            stop_px: "8500".into(),
            stop_limit_px: "8245".into(),
            time_in_force: TimeInForce::GoodTillCancel,
            expire_time: None,
            self_trade: None,
        };
        let msg = order.to_message().unwrap();
        assert_eq!(
            msg.fields.iter().find(|(t, _)| *t == ORD_TYPE).unwrap().1,
            "O"
        );
        assert!(
            msg.fields
                .iter()
                .all(|(t, _)| *t != TRIGGER_PRICE_DIRECTION)
        );
        order.side = Side::Buy;
        assert!(order.to_message().is_err());
        order.price = "8000".into();
        order.stop_px = "8500".into();
        order.stop_limit_px = "8600".into();
        assert!(order.to_message().is_ok());
        order.time_in_force = TimeInForce::ImmediateOrCancel;
        assert!(order.to_message().is_err());
    }

    #[test]
    fn limit_with_funds_has_cash_and_no_base_qty() {
        let msg = LimitWithFunds {
            cl_ord_id: id(),
            symbol: "BTC-USD".into(),
            side: Side::Buy,
            price: "60000".into(),
            cash_order_qty: "30000".into(),
            time_in_force: TimeInForce::GoodTillCancel,
            expire_time: None,
            post_only: false,
            self_trade: None,
        }
        .to_message()
        .unwrap();
        assert!(
            msg.fields
                .iter()
                .any(|(t, v)| *t == CASH_ORDER_QTY && v == "30000")
        );
        assert!(msg.fields.iter().all(|(t, _)| *t != ORDER_QTY));
    }

    #[test]
    fn synthetic_is_sell_fok_and_checks_increment() {
        let msg = SyntheticOrder {
            cl_ord_id: id(),
            symbol: "QNT-DASH".into(),
            ord_type: SyntheticType::Limit,
            order_qty: "10".into(),
            price: Some("3".into()),
            self_trade: SyntheticSelfTrade::CancelOldest,
            base_increment: Some("0.1".into()),
        }
        .to_message()
        .unwrap();
        assert_eq!(
            msg.fields
                .iter()
                .find(|(t, _)| *t == SECURITY_TYPE)
                .unwrap()
                .1,
            "MLEG"
        );
        assert_eq!(msg.fields.iter().find(|(t, _)| *t == SIDE).unwrap().1, "2");
        assert_eq!(
            msg.fields
                .iter()
                .find(|(t, _)| *t == TIME_IN_FORCE)
                .unwrap()
                .1,
            "4"
        );
        let mut bad = SyntheticOrder {
            cl_ord_id: id(),
            symbol: "QNT-DASH".into(),
            ord_type: SyntheticType::Market,
            order_qty: "0.15".into(),
            price: None,
            self_trade: SyntheticSelfTrade::CancelNewest,
            base_increment: Some("0.1".into()),
        };
        assert!(bad.to_message().is_err());
        bad.order_qty = "0.2".into();
        let msg = bad.to_message().unwrap();
        assert!(msg.fields.iter().all(|(t, _)| *t != TIME_IN_FORCE));
    }

    #[test]
    fn batch_rejects_sixteen_and_duplicate_ids() {
        let leg = |n: &str| BatchLeg::Limit {
            cl_ord_id: n.into(),
            side: Side::Buy,
            order_qty: "1".into(),
            price: "1".into(),
            time_in_force: TimeInForce::GoodTillCancel,
            expire_time: None,
            post_only: false,
            self_trade: None,
        };
        let mut orders = Vec::new();
        for i in 0..16 {
            let mut uuid = id();
            uuid.replace_range(0..2, &format!("{i:02x}"));
            orders.push(leg(&uuid));
        }
        let mut batch = NewOrderBatch {
            batch_id: id(),
            symbol: "BTC-USD".into(),
            orders,
        };
        assert!(batch.to_message().is_err());
        batch.orders.truncate(2);
        batch.orders[0] = leg(&id());
        batch.orders[1] = leg(&id());
        assert!(batch.to_message().is_err());
        batch.orders[1] = leg(&id2());
        assert!(batch.to_message().is_ok());
    }

    #[test]
    fn uppercase_uuid_is_rejected() {
        assert!(validate_uuid_v4(&id().to_ascii_uppercase()).is_err());
        assert!(validate_uuid_v4(&new_client_order_id()).is_ok());
    }
}
