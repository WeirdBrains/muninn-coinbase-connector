//! Coinbase Exchange FIX ingest connector for a Muninn code space of kind
//! `ingest-connector`.
//!
//! Schedule options:
//!
//! - `gateway` — `drop-copy` (default), `market-data`, `market-data-nosnap`, `order-entry`
//! - `environment` — `sandbox` (default) or `production`
//! - `venue` — `exchange` (default) or `mica`
//! - `symbols` — comma-separated product ids; required for market data
//! - `depth` — `l2` (default), `l1`, or `l3`
//! - `listen_ms` — how long to read after logon (default 10000, at most 60000)
//! - `max_messages` — application messages stored this run (default 2000, at most 20000)
//! - `heartbeat` — seconds (default 30)
//! - `reset` — `y` or `n`. Market data resets every run. Drop copy and order entry
//!   reset until a sequence cursor exists, then resume
//! - `api_key`, `api_secret`, `api_passphrase` — or `COINBASE_API_KEY`,
//!   `COINBASE_API_SECRET`, and `COINBASE_API_PASSPHRASE`
//!
//! Each application message is stored as JSON. Admin traffic is not. Drop copy and
//! order entry keep the highest inbound sequence in connector state and, on the next
//! run, request only the range after it. This connector listens. It does not submit orders.

use std::time::Duration;

use coinbase_exchange_fix::{
    Credentials, Environment, FixSession, GatewayKind, Incoming, MarketDataRequest, MarketDepth,
    Message, SessionConfig, Subscription, Venue,
};
use muninn_plugin::prelude::*;

const DEFAULT_LISTEN_MS: u64 = 10_000;
const MAX_LISTEN_MS: u64 = 60_000;
const DEFAULT_MAX_MESSAGES: usize = 2_000;
const MAX_MESSAGES_CAP: usize = 20_000;
const DEFAULT_HEARTBEAT_SECS: u64 = 30;

pub fn run(host: &Host) -> Result<View> {
    let ctx = host.context()?;
    let opts = merged_options(&ctx, host);
    let settings = settings_from(&opts)?;
    let creds = credentials(&opts)?;
    let key = state_key(&settings);
    let cursor = load_cursor(host, &key)?;
    let reset = should_reset(settings.gateway, settings.reset, cursor);

    host.log(format!(
        "coinbase fix {} {key} reset={reset}",
        endpoint_of(&settings)
    ));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::new(format!("runtime: {e}")))?;
    let collected = rt.block_on(collect(host, &settings, creds, cursor, reset))?;

    match collected.last_seq {
        Some(seq) => host.state_set(&key, i64::from(seq))?,
        None if reset => host.state_set(&key, Json::Null)?,
        None => {}
    }

    let inserted = if ctx.pipeline.as_ref().is_some_and(|s| !s.is_empty()) {
        host.pipe()?.inserted
    } else {
        0
    };
    let mut rules_run = 0;
    if ctx.ruleset_id.as_ref().is_some_and(|s| !s.is_empty()) {
        let rules = host.run_ruleset(None)?;
        rules_run = rules.rules_run;
        for err in rules.errors {
            host.log(format!("ruleset: {err}"));
        }
    }

    Ok(View::table(
        "Coinbase FIX",
        &[
            "gateway", "stored", "last_seq", "inserted", "rules", "dry_run",
        ],
    )
    .row(vec![
        gateway_name(settings.gateway).into(),
        collected.stored.into(),
        collected.last_seq.map(i64::from).into(),
        inserted.into(),
        rules_run.into(),
        ctx.dry_run.into(),
    ]))
}

muninn_plugin::plugin!(run, kind = "ingest-connector");

struct Collected {
    stored: usize,
    last_seq: Option<u32>,
}

async fn collect(
    host: &Host,
    settings: &Settings,
    creds: Credentials,
    cursor: Option<u32>,
    reset: bool,
) -> Result<Collected> {
    let mut session_cfg = SessionConfig::new(
        settings.environment,
        settings.venue,
        settings.gateway,
        creds,
    )
    .heartbeat_secs(settings.heartbeat_secs)
    .reset_seq_num(reset);
    if !reset && let Some(next) = cursor.and_then(|n| n.checked_add(1)) {
        session_cfg = session_cfg.resume_from(next);
    }

    let mut session = FixSession::connect(session_cfg).await.map_err(fix_err)?;
    if let GatewayKind::MarketData { .. } = settings.gateway {
        let req = MarketDataRequest {
            md_req_id: settings.md_req_id.clone(),
            subscription: Subscription::Subscribe,
            depth: settings.depth,
            symbols: settings.symbols.clone(),
        };
        session
            .send(req.to_message().map_err(fix_err)?)
            .await
            .map_err(fix_err)?;
    }

    let deadline = tokio::time::Instant::now() + settings.listen;
    let mut stored = 0usize;
    // This run's highest seq only. Seeding it with the saved cursor would stop a
    // later run from moving backward when the gateway no longer has that history.
    let mut last_seq = None;
    let mut open = true;
    loop {
        if stored >= settings.max_messages || tokio::time::Instant::now() >= deadline {
            break;
        }
        let incoming = tokio::select! {
            biased;
            msg = session.recv() => msg,
            _ = tokio::time::sleep_until(deadline) => break,
        };
        let env = match incoming {
            Ok(env) => env,
            Err(e) => {
                open = false;
                if stored == 0 {
                    return Err(fix_err(e));
                }
                host.log(format!("session closed after {stored} messages: {e}"));
                break;
            }
        };
        if matches!(env.body, Incoming::Logout { .. }) {
            open = false;
            break;
        }
        note_seq(
            &env.body,
            env.message.seq().ok(),
            env.message.poss_dup(),
            &mut last_seq,
        );
        if !keep(&env.body) {
            continue;
        }
        let seq = env.message.seq().unwrap_or(0);
        let name = format!("{seq:09}");
        let body = message_json(&env.message);
        host.store_raw(&name, "json", body.as_bytes())?;
        stored += 1;
    }

    if open && let Err(e) = session.logout(Some("done".into())).await {
        host.log(format!("logout: {e}"));
    }
    Ok(Collected { stored, last_seq })
}

fn note_seq(body: &Incoming, seq: Option<u32>, poss_dup: bool, last: &mut Option<u32>) {
    let covered = match body {
        Incoming::SequenceReset {
            gap_fill,
            new_seq_no,
        } if *gap_fill || poss_dup => new_seq_no.saturating_sub(1),
        Incoming::SequenceReset { .. } => return,
        _ => match seq {
            Some(seq) => seq,
            None => return,
        },
    };
    if covered == 0 {
        return;
    }
    *last = Some(last.map_or(covered, |have| have.max(covered)));
}

fn keep(body: &Incoming) -> bool {
    !matches!(
        body,
        Incoming::Heartbeat { .. }
            | Incoming::TestRequest { .. }
            | Incoming::ResendRequest { .. }
            | Incoming::SequenceReset { .. }
            | Incoming::Logon(_)
            | Incoming::Logout { .. }
    )
}

fn message_json(msg: &Message) -> String {
    let msg_type = msg.msg_type().unwrap_or("");
    let seq = msg.seq().unwrap_or(0);
    let mut out = String::new();
    out.push_str("{\"msg_type\":");
    push_json_str(&mut out, msg_type);
    out.push_str(",\"seq\":");
    out.push_str(&seq.to_string());
    out.push_str(",\"poss_dup\":");
    out.push_str(if msg.poss_dup() { "true" } else { "false" });
    out.push_str(",\"fields\":[");
    for (i, (tag, value)) in msg.fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let shown = if *tag == 96 || *tag == 554 {
            "***"
        } else {
            value.as_str()
        };
        out.push_str("{\"tag\":");
        out.push_str(&tag.to_string());
        out.push_str(",\"value\":");
        push_json_str(&mut out, shown);
        out.push('}');
    }
    out.push_str("]}");
    out
}

fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{:04x}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[derive(Debug)]
struct Settings {
    environment: Environment,
    venue: Venue,
    gateway: GatewayKind,
    symbols: Vec<String>,
    depth: MarketDepth,
    listen: Duration,
    max_messages: usize,
    heartbeat_secs: u64,
    reset: Reset,
    md_req_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reset {
    Yes,
    No,
    Auto,
}

fn settings_from(opts: &[(String, String)]) -> Result<Settings> {
    let environment = match opt(opts, "environment").unwrap_or("sandbox") {
        "sandbox" => Environment::Sandbox,
        "production" => Environment::Production,
        other => {
            return Err(Error::new(format!(
                "environment `{other}` is not sandbox or production"
            )));
        }
    };
    let venue = match opt(opts, "venue").unwrap_or("exchange") {
        "exchange" => Venue::Exchange,
        "mica" => Venue::Mica,
        other => {
            return Err(Error::new(format!(
                "venue `{other}` is not exchange or mica"
            )));
        }
    };
    let gateway = match opt(opts, "gateway").unwrap_or("drop-copy") {
        "drop-copy" => GatewayKind::DropCopy,
        "order-entry" => GatewayKind::OrderEntry,
        "market-data" => GatewayKind::MarketData { snapshot: true },
        "market-data-nosnap" => GatewayKind::MarketData { snapshot: false },
        other => {
            return Err(Error::new(format!(
                "gateway `{other}` is not drop-copy, market-data, market-data-nosnap, or order-entry"
            )));
        }
    };
    let symbols = opt(opts, "symbols")
        .or_else(|| opt(opts, "symbol"))
        .unwrap_or("")
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if matches!(gateway, GatewayKind::MarketData { .. }) && symbols.is_empty() {
        return Err(Error::new(
            "market data needs a symbols option, for example symbols=BTC-USD",
        ));
    }
    let depth = match opt(opts, "depth").unwrap_or("l2") {
        "l1" | "1" => MarketDepth::L1,
        "l2" | "10" => MarketDepth::L2,
        "l3" | "0" => MarketDepth::L3,
        other => {
            return Err(Error::new(format!("depth `{other}` is not l1, l2, or l3")));
        }
    };
    let listen_ms = number(opts, "listen_ms", DEFAULT_LISTEN_MS)?;
    if !(1..=MAX_LISTEN_MS).contains(&listen_ms) {
        return Err(Error::new(format!("listen_ms must be 1..={MAX_LISTEN_MS}")));
    }
    let max_messages = number(opts, "max_messages", DEFAULT_MAX_MESSAGES as u64)? as usize;
    if !(1..=MAX_MESSAGES_CAP).contains(&max_messages) {
        return Err(Error::new(format!(
            "max_messages must be 1..={MAX_MESSAGES_CAP}"
        )));
    }
    let heartbeat_secs = number(opts, "heartbeat", DEFAULT_HEARTBEAT_SECS)?;
    if heartbeat_secs == 0 || heartbeat_secs > gateway.heartbeat_max_secs() {
        return Err(Error::new(format!(
            "heartbeat must be 1..={} seconds on this gateway",
            gateway.heartbeat_max_secs()
        )));
    }
    let reset = match opt(opts, "reset") {
        None => Reset::Auto,
        Some("y" | "yes" | "true" | "1") => Reset::Yes,
        Some("n" | "no" | "false" | "0") => Reset::No,
        Some(other) => {
            return Err(Error::new(format!("reset `{other}` is not y or n")));
        }
    };
    Ok(Settings {
        environment,
        venue,
        gateway,
        symbols,
        depth,
        listen: Duration::from_millis(listen_ms),
        max_messages,
        heartbeat_secs,
        reset,
        md_req_id: req_id(opt(opts, "run_id")),
    })
}

fn should_reset(gateway: GatewayKind, reset: Reset, cursor: Option<u32>) -> bool {
    match reset {
        Reset::Yes => true,
        Reset::No => false,
        Reset::Auto => matches!(gateway, GatewayKind::MarketData { .. }) || cursor.is_none(),
    }
}

fn credentials(opts: &[(String, String)]) -> Result<Credentials> {
    let key = opt(opts, "api_key");
    let secret = opt(opts, "api_secret");
    let pass = opt(opts, "api_passphrase");
    match (key, secret, pass) {
        (None, None, None) => Credentials::from_env().map_err(fix_err),
        (Some(key), Some(secret), Some(pass)) => {
            Credentials::new(key, secret, pass).map_err(fix_err)
        }
        _ => Err(Error::new(
            "set api_key, api_secret, and api_passphrase together, or COINBASE_API_KEY, COINBASE_API_SECRET, and COINBASE_API_PASSPHRASE",
        )),
    }
}

fn merged_options(ctx: &Context, host: &Host) -> Vec<(String, String)> {
    let mut opts = ctx.options.clone();
    for (key, value) in host.options() {
        if !opts.iter().any(|(have, _)| have.eq_ignore_ascii_case(&key)) {
            opts.push((key, value));
        }
    }
    if let Some(id) = &ctx.run_id
        && opt(&opts, "run_id").is_none()
    {
        opts.push(("run_id".into(), id.clone()));
    }
    opts
}

fn load_cursor(host: &Host, key: &str) -> Result<Option<u32>> {
    let Some(value) = host.state_get(key)? else {
        return Ok(None);
    };
    Ok(value
        .as_i64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 1))
}

fn state_key(settings: &Settings) -> String {
    let env = match settings.environment {
        Environment::Sandbox => "sandbox",
        Environment::Production => "production",
    };
    let venue = match settings.venue {
        Venue::Exchange => "exchange",
        Venue::Mica => "mica",
    };
    format!("seq:{env}:{venue}:{}", gateway_name(settings.gateway))
}

fn gateway_name(gateway: GatewayKind) -> &'static str {
    match gateway {
        GatewayKind::DropCopy => "drop-copy",
        GatewayKind::OrderEntry => "order-entry",
        GatewayKind::MarketData { snapshot: true } => "market-data",
        GatewayKind::MarketData { snapshot: false } => "market-data-nosnap",
    }
}

fn endpoint_of(settings: &Settings) -> String {
    coinbase_exchange_fix::Endpoint::resolve(settings.environment, settings.venue, settings.gateway)
        .to_string()
}

fn req_id(run_id: Option<&str>) -> String {
    let id: String = run_id
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    if id.is_empty() { "muninn".into() } else { id }
}

fn opt<'a>(opts: &'a [(String, String)], key: &str) -> Option<&'a str> {
    opts.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

fn number(opts: &[(String, String)], key: &str, default: u64) -> Result<u64> {
    let Some(raw) = opt(opts, key) else {
        return Ok(default);
    };
    raw.parse()
        .map_err(|_| Error::new(format!("{key} `{raw}` is not a number")))
}

fn fix_err(err: coinbase_exchange_fix::Error) -> Error {
    Error::new(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn drop_copy_defaults_to_sandbox_and_resets_until_a_cursor_exists() {
        let settings = settings_from(&pairs(&[])).unwrap();
        assert_eq!(settings.environment, Environment::Sandbox);
        assert_eq!(settings.venue, Venue::Exchange);
        assert_eq!(settings.gateway, GatewayKind::DropCopy);
        assert!(should_reset(settings.gateway, settings.reset, None));
        assert!(!should_reset(settings.gateway, settings.reset, Some(12)));
        assert_eq!(state_key(&settings), "seq:sandbox:exchange:drop-copy");
    }

    #[test]
    fn market_data_resets_even_with_a_cursor_and_needs_symbols() {
        let err = settings_from(&pairs(&[("gateway", "market-data")])).unwrap_err();
        assert!(err.to_string().contains("symbols"));
        let settings = settings_from(&pairs(&[
            ("gateway", "market-data-nosnap"),
            ("environment", "production"),
            ("venue", "mica"),
            ("symbols", "BTC-USD, ETH-USD"),
            ("depth", "l3"),
            ("reset", "n"),
        ]))
        .unwrap();
        assert_eq!(
            settings.gateway,
            GatewayKind::MarketData { snapshot: false }
        );
        assert_eq!(settings.symbols, vec!["BTC-USD", "ETH-USD"]);
        assert_eq!(settings.depth, MarketDepth::L3);
        assert_eq!(settings.environment, Environment::Production);
        assert_eq!(settings.venue, Venue::Mica);
        assert!(!should_reset(settings.gateway, settings.reset, None));
        assert!(should_reset(
            GatewayKind::MarketData { snapshot: true },
            Reset::Auto,
            Some(9)
        ));
    }

    #[test]
    fn listen_and_heartbeat_stay_inside_the_gateway_limits() {
        let err = settings_from(&pairs(&[("listen_ms", "60001")])).unwrap_err();
        assert!(err.to_string().contains("listen_ms"));
        let err = settings_from(&pairs(&[
            ("gateway", "market-data"),
            ("symbols", "BTC-USD"),
            ("heartbeat", "31"),
        ]))
        .unwrap_err();
        assert!(err.to_string().contains("heartbeat"));
        let settings =
            settings_from(&pairs(&[("gateway", "drop-copy"), ("heartbeat", "120")])).unwrap();
        assert_eq!(settings.heartbeat_secs, 120);
    }

    #[test]
    fn stored_json_redacts_the_secret_tags() {
        let msg = Message {
            fields: vec![
                (35, "8".into()),
                (34, "4".into()),
                (55, "BTC-USD".into()),
                (96, "super-secret-value".into()),
                (554, "passphrase-value".into()),
            ],
        };
        let json = message_json(&msg);
        assert!(json.contains("\"msg_type\":\"8\""));
        assert!(json.contains("\"seq\":4"));
        assert!(json.contains("BTC-USD"));
        assert!(json.contains("***"));
        assert!(!json.contains("super-secret-value"));
        assert!(!json.contains("passphrase-value"));
    }

    #[test]
    fn a_gap_fill_advances_the_cursor_and_the_announcement_does_not() {
        let mut last = Some(40);
        note_seq(
            &Incoming::SequenceReset {
                gap_fill: false,
                new_seq_no: 9_000,
            },
            Some(2),
            false,
            &mut last,
        );
        assert_eq!(last, Some(40));
        note_seq(
            &Incoming::SequenceReset {
                gap_fill: true,
                new_seq_no: 51,
            },
            Some(41),
            true,
            &mut last,
        );
        assert_eq!(last, Some(50));
    }
}
