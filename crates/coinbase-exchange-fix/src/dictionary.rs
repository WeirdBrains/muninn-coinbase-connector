//! Published Coinbase Exchange FIX dictionaries.
//!
//! The XML is the exchange's own QuickFIX-style dictionary. This crate does not
//! load it at runtime; the builders follow the same messages.

pub const DICTIONARY_VERSION: &str = "20260612";

pub const DICTIONARY_URL: &str =
    "https://docs.cdp.coinbase.com/exchange/files/cb_exch_fix_dictionaries-20260612.zip";

/// Paths inside `cb_exch_fix_dictionaries-latest`.
pub const DICTIONARY_FILES: &[&str] = &[
    "market-data/FIX50-prod-sand.xml",
    "market-data/FIXT11-prod-sand.xml",
    "order-entry/FIX42-prod-sand.xml",
    "order-entry/FIX50-prod-sand.xml",
    "order-entry/FIXT11-prod-sand.xml",
];

/// Older archives, newest first after [`DICTIONARY_URL`].
pub const DICTIONARY_ARCHIVE: &[(&str, &str)] = &[
    ("2026-06-12", DICTIONARY_URL),
    (
        "2026-05-21",
        "https://docs.cdp.coinbase.com/exchange/files/cb_exch_fix_dictionaries-20260521.zip",
    ),
    (
        "2025-11-19",
        "https://docs.cdp.coinbase.com/exchange/files/cb_exch_fix_dictionaries-20251119.zip",
    ),
    (
        "2023-10-06",
        "https://docs.cdp.coinbase.com/exchange/files/cb_exch_fix_dictionaries-20231006.tar.gz",
    ),
    (
        "2023-08-03",
        "https://docs.cdp.coinbase.com/exchange/files/cb_exch_fix_dictionaries-20230803.tar.gz",
    ),
];
