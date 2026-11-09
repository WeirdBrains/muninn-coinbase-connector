//! Gateway hostnames. Coinbase does not publish static IPs; connect by name.
//!
//! Order entry, market data, and drop copy each take one connection per API key.
//! Market data and drop copy do not consume order-entry rate limits.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Sandbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Venue {
    /// Exchange FIX gateways.
    Exchange,
    /// MiCA / DMA prime gateways.
    Mica,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayKind {
    OrderEntry,
    /// `snapshot`: the snapshot-enabled port. `false` is the snapshot-disabled port,
    /// used when the book snapshot comes from REST.
    MarketData {
        snapshot: bool,
    },
    DropCopy,
}

impl GatewayKind {
    pub fn heartbeat_max_secs(self) -> u64 {
        match self {
            GatewayKind::DropCopy => crate::limits::MAX_DROP_COPY_HEARTBEAT_SECS,
            GatewayKind::OrderEntry | GatewayKind::MarketData { .. } => {
                crate::limits::MAX_HEARTBEAT_SECS
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    pub fn resolve(env: Environment, venue: Venue, gateway: GatewayKind) -> Self {
        let (host, port) = match (env, venue, gateway) {
            (Environment::Production, Venue::Exchange, GatewayKind::OrderEntry) => {
                ("fix-ord.exchange.coinbase.com", 6121)
            }
            (
                Environment::Production,
                Venue::Exchange,
                GatewayKind::MarketData { snapshot: true },
            ) => ("fix-md.exchange.coinbase.com", 6121),
            (
                Environment::Production,
                Venue::Exchange,
                GatewayKind::MarketData { snapshot: false },
            ) => ("fix-md.exchange.coinbase.com", 6122),
            (Environment::Production, Venue::Exchange, GatewayKind::DropCopy) => {
                ("fix-dc.exchange.coinbase.com", 6122)
            }

            (Environment::Sandbox, Venue::Exchange, GatewayKind::OrderEntry) => {
                ("fix-ord.sandbox.exchange.coinbase.com", 6121)
            }
            (Environment::Sandbox, Venue::Exchange, GatewayKind::MarketData { snapshot: true }) => {
                ("fix-md.sandbox.exchange.coinbase.com", 6121)
            }
            (
                Environment::Sandbox,
                Venue::Exchange,
                GatewayKind::MarketData { snapshot: false },
            ) => ("fix-md.sandbox.exchange.coinbase.com", 6122),
            (Environment::Sandbox, Venue::Exchange, GatewayKind::DropCopy) => {
                ("fix-dc.sandbox.exchange.coinbase.com", 6122)
            }

            (Environment::Production, Venue::Mica, GatewayKind::OrderEntry) => {
                ("fix-us.dma.prime.coinbase.com", 7110)
            }
            (Environment::Production, Venue::Mica, GatewayKind::MarketData { snapshot: true }) => {
                ("fix-us.dma.prime.coinbase.com", 7120)
            }
            (Environment::Production, Venue::Mica, GatewayKind::MarketData { snapshot: false }) => {
                ("fix-us.dma.prime.coinbase.com", 7121)
            }
            (Environment::Production, Venue::Mica, GatewayKind::DropCopy) => {
                ("fix-dc.dma.prime.coinbase.com", 7122)
            }

            (Environment::Sandbox, Venue::Mica, GatewayKind::OrderEntry) => {
                ("fix-us.dma.sandbox.prime.coinbase.com", 7110)
            }
            (Environment::Sandbox, Venue::Mica, GatewayKind::MarketData { snapshot: true }) => {
                ("fix-us.dma.sandbox.prime.coinbase.com", 7120)
            }
            (Environment::Sandbox, Venue::Mica, GatewayKind::MarketData { snapshot: false }) => {
                ("fix-us.dma.sandbox.prime.coinbase.com", 7121)
            }
            (Environment::Sandbox, Venue::Mica, GatewayKind::DropCopy) => {
                ("fix-dc.dma.sandbox.prime.coinbase.com", 7122)
            }
        };
        Endpoint {
            host: host.to_string(),
            port,
        }
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tcp+ssl://{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_endpoints() {
        let oe = Endpoint::resolve(
            Environment::Production,
            Venue::Exchange,
            GatewayKind::OrderEntry,
        );
        assert_eq!(
            oe.to_string(),
            "tcp+ssl://fix-ord.exchange.coinbase.com:6121"
        );
        let md = Endpoint::resolve(
            Environment::Sandbox,
            Venue::Exchange,
            GatewayKind::MarketData { snapshot: false },
        );
        assert_eq!(md.port, 6122);
        let dc = Endpoint::resolve(Environment::Production, Venue::Mica, GatewayKind::DropCopy);
        assert_eq!(dc.host, "fix-dc.dma.prime.coinbase.com");
        assert_eq!(dc.port, 7122);
        let sand = Endpoint::resolve(Environment::Sandbox, Venue::Mica, GatewayKind::OrderEntry);
        assert_eq!(
            sand.to_string(),
            "tcp+ssl://fix-us.dma.sandbox.prime.coinbase.com:7110"
        );
    }
}
