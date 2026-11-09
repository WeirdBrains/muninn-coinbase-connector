//! Fixed one-second windows, aligned to the Unix epoch.
//!
//! FIX 5.0 allows 2 logons per second per API key and 100 requests per second.
//! The gateway disconnects a session that sends more than 200 messages per second.
//! Heartbeats count toward the 200 and not toward the 100. A batch is one request.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

pub const LOGONS_PER_SECOND: u32 = 2;
pub const REQUESTS_PER_SECOND: u32 = 100;
pub const DISCONNECT_PER_SECOND: u32 = 200;
pub const MAX_BATCH_ORDERS: usize = 15;
pub const MAX_CONNECTIONS_PER_PROFILE: usize = 75;
pub const MAX_CONNECTIONS_PER_USER: usize = 175;
pub const MAX_PROFILES_PER_USER: usize = 100;
pub const MAX_RESEND_RANGE: u32 = 1000;
/// Largest `MsgSeqNum` the gateway will accept.
pub const MAX_SEQ: u32 = 2_147_483_647;
pub const DEFAULT_HEARTBEAT_SECS: u64 = 10;
/// Order entry and market data.
pub const MAX_HEARTBEAT_SECS: u64 = 30;
/// Drop copy only.
pub const MAX_DROP_COPY_HEARTBEAT_SECS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Traffic {
    Logon,
    /// Application messages: orders, cancels, market data requests, quotes.
    Request,
    /// Heartbeats, test requests, resends, gap fills, logout.
    Admin,
}

pub fn traffic(msg_type: &str) -> Traffic {
    match msg_type {
        "A" => Traffic::Logon,
        "0" | "1" | "2" | "3" | "4" | "5" => Traffic::Admin,
        _ => Traffic::Request,
    }
}

#[derive(Debug, Clone)]
pub struct RateLimiter {
    window: u64,
    logons: u32,
    requests: u32,
    messages: u32,
    open: bool,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            window: 0,
            logons: 0,
            requests: 0,
            messages: 0,
            open: false,
        }
    }

    /// Record one outbound message, or refuse it. A refusal does not consume budget.
    pub fn check(&mut self, now: SystemTime, kind: Traffic) -> Result<()> {
        let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        if !self.open || secs != self.window {
            self.window = secs;
            self.logons = 0;
            self.requests = 0;
            self.messages = 0;
            self.open = true;
        }
        let (logon, request) = match kind {
            Traffic::Logon => (1, 0),
            Traffic::Request => (0, 1),
            Traffic::Admin => (0, 0),
        };
        if self.messages + 1 > DISCONNECT_PER_SECOND {
            return Err(Error::RateLimit(format!(
                "sending another message would exceed {DISCONNECT_PER_SECOND}/s and the gateway would disconnect"
            )));
        }
        if self.logons + logon > LOGONS_PER_SECOND {
            return Err(Error::RateLimit(format!(
                "at most {LOGONS_PER_SECOND} logons per second per API key"
            )));
        }
        if self.requests + request > REQUESTS_PER_SECOND {
            return Err(Error::RateLimit(format!(
                "at most {REQUESTS_PER_SECOND} requests per second"
            )));
        }
        self.messages += 1;
        self.logons += logon;
        self.requests += request;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(sec: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(sec)
    }

    #[test]
    fn windows_and_buckets() {
        let mut lim = RateLimiter::new();
        let t = at(1_700_000_000);
        lim.check(t, Traffic::Logon).unwrap();
        lim.check(t, Traffic::Logon).unwrap();
        assert!(lim.check(t, Traffic::Logon).is_err());
        for _ in 0..100 {
            lim.check(t, Traffic::Request).unwrap();
        }
        assert!(lim.check(t, Traffic::Request).is_err());
        for _ in 0..98 {
            lim.check(t, Traffic::Admin).unwrap();
        }
        assert!(lim.check(t, Traffic::Admin).is_err());
        lim.check(at(1_700_000_001), Traffic::Request).unwrap();
    }
}
