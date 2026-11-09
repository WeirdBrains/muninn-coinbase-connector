//! Local books built from market-data snapshots and incrementals.
//!
//! On the snapshot-enabled gateway, queue incrementals until the snapshot for that
//! symbol arrives, then drop any L3 update whose `RptSeq` is less than or equal to
//! the snapshot sequence. On the snapshot-disabled gateway, do the same with a REST
//! book passed to [`L3BookBuilder::seed`]. A `Change` for an order id that was never
//! `New` is ignored, including a self-trade reduction that races the new.

use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::market::{MdEntry, MdEntryType, MdIncremental, MdSnapshot, MdUpdateAction, SnapEntry};
use crate::order::Side;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestingOrder {
    pub id: String,
    pub side: Side,
    pub price: String,
    pub size: String,
}

pub fn resting_order(
    id: impl Into<String>,
    side: Side,
    price: impl Into<String>,
    size: impl Into<String>,
) -> RestingOrder {
    RestingOrder {
        id: id.into(),
        side,
        price: price.into(),
        size: size.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradePrint {
    pub price: String,
    pub size: String,
    pub aggressor: Option<Side>,
    pub order_id: Option<String>,
    pub transact_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceLevel {
    pub side: Side,
    pub price: String,
    pub size: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct L3Book {
    pub symbol: String,
    pub rpt_seq: Option<u64>,
    pub trading_status: Option<String>,
    orders: HashMap<String, RestingOrder>,
    pub last_trade: Option<TradePrint>,
}

impl L3Book {
    pub fn orders(&self) -> impl Iterator<Item = &RestingOrder> {
        self.orders.values()
    }

    pub fn get(&self, id: &str) -> Option<&RestingOrder> {
        self.orders.get(id)
    }

    pub fn best_bid(&self) -> Option<&RestingOrder> {
        best(self.orders.values().filter(|o| o.side == Side::Buy), true)
    }

    pub fn best_ask(&self) -> Option<&RestingOrder> {
        best(self.orders.values().filter(|o| o.side == Side::Sell), false)
    }
}

fn best<'a>(
    orders: impl Iterator<Item = &'a RestingOrder>,
    highest: bool,
) -> Option<&'a RestingOrder> {
    orders
        .filter_map(|o| Decimal::from_str_radix_ok(&o.price).map(|px| (px, o)))
        .max_by(|a, b| {
            if highest {
                a.0.cmp(&b.0)
            } else {
                b.0.cmp(&a.0)
            }
        })
        .map(|(_, o)| o)
}

trait DecimalParse {
    fn from_str_radix_ok(s: &str) -> Option<Decimal>;
}

impl DecimalParse for Decimal {
    fn from_str_radix_ok(s: &str) -> Option<Decimal> {
        s.parse().ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2Book {
    pub symbol: String,
    pub trading_status: Option<String>,
    levels: Vec<PriceLevel>,
    pub last_trade: Option<TradePrint>,
}

impl L2Book {
    pub fn levels(&self) -> &[PriceLevel] {
        &self.levels
    }

    pub fn best_bid(&self) -> Option<&PriceLevel> {
        best_level(self.levels.iter().filter(|l| l.side == Side::Buy), true)
    }

    pub fn best_ask(&self) -> Option<&PriceLevel> {
        best_level(self.levels.iter().filter(|l| l.side == Side::Sell), false)
    }
}

fn best_level<'a>(
    levels: impl Iterator<Item = &'a PriceLevel>,
    highest: bool,
) -> Option<&'a PriceLevel> {
    levels
        .filter_map(|l| Decimal::from_str_radix_ok(&l.price).map(|px| (px, l)))
        .max_by(|a, b| {
            if highest {
                a.0.cmp(&b.0)
            } else {
                b.0.cmp(&a.0)
            }
        })
        .map(|(_, l)| l)
}

#[derive(Debug, Clone)]
pub struct L3BookBuilder {
    symbol: String,
    ready: bool,
    snapshot_seq: Option<u64>,
    queue: Vec<MdEntry>,
    fragments: Vec<SnapEntry>,
    book: L3Book,
}

impl L3BookBuilder {
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        Self {
            book: L3Book {
                symbol: symbol.clone(),
                ..L3Book::default()
            },
            symbol,
            ready: false,
            snapshot_seq: None,
            queue: Vec::new(),
            fragments: Vec::new(),
        }
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    pub fn book(&self) -> &L3Book {
        &self.book
    }

    pub fn on_incremental(&mut self, msg: &MdIncremental) {
        for entry in &msg.entries {
            if entry.symbol != self.symbol {
                continue;
            }
            if !self.ready {
                self.queue.push(entry.clone());
                continue;
            }
            self.apply(entry);
        }
    }

    pub fn on_snapshot(&mut self, msg: &MdSnapshot) {
        if msg.symbol != self.symbol {
            return;
        }
        if let Some(status) = &msg.trading_status {
            self.book.trading_status = Some(status.clone());
        }
        self.fragments.extend(msg.entries.iter().cloned());
        if !msg.last_fragment {
            return;
        }
        let seq = msg.rpt_seq.or(self.snapshot_seq);
        self.install(seq, &self.fragments.clone());
        self.fragments.clear();
    }

    /// REST book, or any full L3 image. `rpt_seq` is the snapshot sequence; queued
    /// incrementals at or below it are dropped.
    pub fn seed(&mut self, rpt_seq: u64, orders: Vec<RestingOrder>) {
        let entries: Vec<SnapEntry> = orders
            .into_iter()
            .map(|o| SnapEntry {
                entry_type: if o.side == Side::Buy {
                    MdEntryType::Bid
                } else {
                    MdEntryType::Offer
                },
                entry_id: Some(o.id),
                price: o.price,
                size: o.size,
            })
            .collect();
        self.install(Some(rpt_seq), &entries);
    }

    fn install(&mut self, seq: Option<u64>, entries: &[SnapEntry]) {
        self.book.orders.clear();
        self.book.rpt_seq = seq;
        self.snapshot_seq = seq;
        for entry in entries {
            let Some(id) = &entry.entry_id else { continue };
            let Some(side) = entry.entry_type.side() else {
                continue;
            };
            self.book.orders.insert(
                id.clone(),
                RestingOrder {
                    id: id.clone(),
                    side,
                    price: entry.price.clone(),
                    size: entry.size.clone(),
                },
            );
        }
        self.ready = true;
        let queued = std::mem::take(&mut self.queue);
        for entry in queued {
            self.apply(&entry);
        }
    }

    fn apply(&mut self, entry: &MdEntry) {
        if let Some(seq) = entry.rpt_seq {
            if self.snapshot_seq.is_some_and(|snap| seq <= snap) {
                return;
            }
            self.book.rpt_seq = Some(seq);
        }
        if entry.entry_type == MdEntryType::Trade {
            self.book.last_trade = Some(TradePrint {
                price: entry.price.clone(),
                size: entry.size.clone().unwrap_or_default(),
                aggressor: entry.aggressor_side,
                order_id: entry.order_id.clone(),
                transact_time: entry.transact_time.clone(),
            });
            return;
        }
        let Some(id) = &entry.entry_id else {
            return;
        };
        match entry.action {
            MdUpdateAction::New => {
                let Some(side) = entry.entry_type.side() else {
                    return;
                };
                self.book.orders.insert(
                    id.clone(),
                    RestingOrder {
                        id: id.clone(),
                        side,
                        price: entry.price.clone(),
                        size: entry.size.clone().unwrap_or_default(),
                    },
                );
            }
            MdUpdateAction::Change => {
                let Some(order) = self.book.orders.get_mut(id) else {
                    return;
                };
                if !entry.price.is_empty() {
                    order.price = entry.price.clone();
                }
                if let Some(size) = &entry.size {
                    order.size = size.clone();
                }
            }
            MdUpdateAction::Delete => {
                self.book.orders.remove(id);
            }
            MdUpdateAction::Other(_) => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct L2BookBuilder {
    symbol: String,
    ready: bool,
    queue: Vec<MdEntry>,
    book: L2Book,
}

impl L2BookBuilder {
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        Self {
            book: L2Book {
                symbol: symbol.clone(),
                trading_status: None,
                levels: Vec::new(),
                last_trade: None,
            },
            symbol,
            ready: false,
            queue: Vec::new(),
        }
    }

    pub fn book(&self) -> &L2Book {
        &self.book
    }

    pub fn ready(&self) -> bool {
        self.ready
    }

    pub fn on_incremental(&mut self, msg: &MdIncremental) {
        for entry in &msg.entries {
            if entry.symbol != self.symbol {
                continue;
            }
            if !self.ready {
                self.queue.push(entry.clone());
            } else {
                apply_l2(&mut self.book, entry);
            }
        }
    }

    pub fn on_snapshot(&mut self, msg: &MdSnapshot) {
        if msg.symbol != self.symbol || !msg.last_fragment {
            return;
        }
        self.book.levels = msg
            .entries
            .iter()
            .filter_map(|e| {
                e.entry_type.side().map(|side| PriceLevel {
                    side,
                    price: e.price.clone(),
                    size: e.size.clone(),
                })
            })
            .collect();
        self.book.trading_status = msg.trading_status.clone();
        self.ready = true;
        for entry in std::mem::take(&mut self.queue) {
            apply_l2(&mut self.book, &entry);
        }
    }
}

fn apply_l2(book: &mut L2Book, entry: &MdEntry) {
    if entry.entry_type == MdEntryType::Trade {
        book.last_trade = Some(TradePrint {
            price: entry.price.clone(),
            size: entry.size.clone().unwrap_or_default(),
            aggressor: entry.aggressor_side,
            order_id: entry.order_id.clone(),
            transact_time: entry.transact_time.clone(),
        });
        return;
    }
    let Some(side) = entry.entry_type.side() else {
        return;
    };
    book.levels
        .retain(|l| !(l.side == side && l.price == entry.price));
    if !matches!(entry.action, MdUpdateAction::Delete)
        && let Some(size) = &entry.size
    {
        book.levels.push(PriceLevel {
            side,
            price: entry.price.clone(),
            size: size.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        action: MdUpdateAction,
        id: Option<&str>,
        seq: u64,
        price: &str,
        size: Option<&str>,
    ) -> MdEntry {
        MdEntry {
            action,
            entry_type: MdEntryType::Bid,
            entry_id: id.map(str::to_string),
            rpt_seq: Some(seq),
            symbol: "BTC-USD".into(),
            price: price.into(),
            size: size.map(str::to_string),
            transact_time: None,
            ord_type: None,
            cl_ord_id: None,
            order_id: None,
            text: None,
            aggressor_side: None,
            funds: None,
        }
    }

    #[test]
    fn change_before_new_is_ignored_and_old_seq_is_dropped() {
        let mut book = L3BookBuilder::new("BTC-USD");
        book.on_incremental(&MdIncremental {
            md_req_id: "1".into(),
            entries: vec![
                entry(MdUpdateAction::New, Some("stale"), 3, "90", Some("1")),
                entry(MdUpdateAction::Change, Some("ord-1"), 5, "100", Some("2")),
            ],
        });
        book.seed(4, vec![resting_order("resting", Side::Buy, "99", "1")]);
        assert!(
            book.book().get("stale").is_none(),
            "updates at or below the snapshot sequence are dropped"
        );
        assert!(
            book.book().get("ord-1").is_none(),
            "a change for an order that was never new is ignored"
        );
        assert!(book.book().get("resting").is_some());
        book.on_incremental(&MdIncremental {
            md_req_id: "1".into(),
            entries: vec![entry(
                MdUpdateAction::New,
                Some("ord-2"),
                6,
                "101",
                Some("3"),
            )],
        });
        assert_eq!(book.book().get("ord-2").unwrap().size, "3");
        book.on_incremental(&MdIncremental {
            md_req_id: "1".into(),
            entries: vec![entry(MdUpdateAction::Delete, Some("ord-2"), 7, "101", None)],
        });
        assert!(book.book().get("ord-2").is_none());
        assert_eq!(book.book().best_bid().unwrap().id, "resting");
    }

    #[test]
    fn ack_without_entry_id_does_not_rest() {
        let mut book = L3BookBuilder::new("BTC-USD");
        book.seed(1, vec![]);
        book.on_incremental(&MdIncremental {
            md_req_id: "1".into(),
            entries: vec![entry(MdUpdateAction::New, None, 2, "100", Some("1"))],
        });
        assert_eq!(book.book().orders().count(), 0);
    }
}
