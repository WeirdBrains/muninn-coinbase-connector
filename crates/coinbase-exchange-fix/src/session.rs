//! TLS session: logon, heartbeats, test requests, and resend.
//!
//! [`FixSession::connect`] completes only after the gateway's logon. Dropping the
//! session closes the socket. Call [`FixSession::logout`] to send a logout first.
//! `recv` must be polled; if the inbound queue fills, the session disconnects
//! rather than stop heartbeats.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::Utc;
use rustls::RootCertStore;
use rustls::client::ClientConfig;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{self, Instant};
use tokio_rustls::TlsConnector;

use crate::TARGET_COMP_ID;
use crate::auth::{CancelOnDisconnect, Credentials, build_logon};
use crate::codec::{AppMessage, Decode, Message, OutHeader, decode_one, encode};
use crate::endpoint::{Endpoint, Environment, GatewayKind, Venue};
use crate::engine::{Action, Engine, Outbound};
use crate::error::{Error, Result};
use crate::limits::{DEFAULT_HEARTBEAT_SECS, RateLimiter, traffic};
use crate::parse::Envelope;
use crate::tags::TEXT;
use crate::timefmt::format_millis;

const INBOUND_QUEUE: usize = 1024;
const MAX_FRAME: usize = 8 * 1024 * 1024;

/// What to connect, and how the logon is signed.
#[derive(Clone)]
pub struct SessionConfig {
    pub endpoint: Endpoint,
    pub credentials: Credentials,
    pub gateway: GatewayKind,
    pub heartbeat_secs: u64,
    pub reset_seq_num: bool,
    /// Next inbound sequence still wanted when `reset_seq_num` is false. The
    /// post-logon Sequence Reset then requests this seq through the server's
    /// stored last seq, instead of replaying the whole session.
    pub resume_from: Option<u32>,
    pub cancel_on_disconnect: Option<CancelOnDisconnect>,
    /// Extra trust anchors (DER). The webpki Mozilla roots are always loaded too.
    pub extra_root_ders: Vec<Vec<u8>>,
    pub connect_timeout: Duration,
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig")
            .field("endpoint", &self.endpoint)
            .field("credentials", &self.credentials)
            .field("gateway", &self.gateway)
            .field("heartbeat_secs", &self.heartbeat_secs)
            .field("reset_seq_num", &self.reset_seq_num)
            .field("resume_from", &self.resume_from)
            .field("cancel_on_disconnect", &self.cancel_on_disconnect)
            .field("extra_root_ders", &self.extra_root_ders.len())
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

impl SessionConfig {
    pub fn new(
        env: Environment,
        venue: Venue,
        gateway: GatewayKind,
        credentials: Credentials,
    ) -> Self {
        Self {
            endpoint: Endpoint::resolve(env, venue, gateway),
            credentials,
            gateway,
            heartbeat_secs: DEFAULT_HEARTBEAT_SECS,
            reset_seq_num: true,
            resume_from: None,
            cancel_on_disconnect: None,
            extra_root_ders: Vec::new(),
            connect_timeout: Duration::from_secs(10),
        }
    }

    pub fn heartbeat_secs(mut self, secs: u64) -> Self {
        self.heartbeat_secs = secs;
        self
    }

    pub fn reset_seq_num(mut self, reset: bool) -> Self {
        self.reset_seq_num = reset;
        self
    }

    /// Next inbound sequence to request when resuming. Ignored when the logon resets.
    pub fn resume_from(mut self, seq: u32) -> Self {
        self.resume_from = Some(seq);
        self
    }

    pub fn cancel_on_disconnect(mut self, mode: Option<CancelOnDisconnect>) -> Self {
        self.cancel_on_disconnect = mode;
        self
    }

    pub fn endpoint(mut self, endpoint: Endpoint) -> Self {
        self.endpoint = endpoint;
        self
    }

    pub fn extra_root_der(mut self, der: Vec<u8>) -> Self {
        self.extra_root_ders.push(der);
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    fn validate(&self) -> Result<()> {
        if self.heartbeat_secs == 0 || self.heartbeat_secs > self.gateway.heartbeat_max_secs() {
            return Err(Error::rejected(format!(
                "heartbeat must be 1..={} seconds on this gateway",
                self.gateway.heartbeat_max_secs()
            )));
        }
        if self.gateway == GatewayKind::DropCopy && self.cancel_on_disconnect.is_some() {
            return Err(Error::rejected(
                "drop copy sessions do not set CancelOrdersOnDisconnect",
            ));
        }
        Ok(())
    }
}

enum Command {
    Send {
        msg: AppMessage,
        reply: oneshot::Sender<Result<()>>,
    },
    Logout {
        text: Option<String>,
        reply: oneshot::Sender<Result<()>>,
    },
}

/// A logged-on FIX session.
pub struct FixSession {
    commands: mpsc::Sender<Command>,
    inbound: mpsc::Receiver<Result<Envelope>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FixSession {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FixSession {
    pub async fn connect(config: SessionConfig) -> Result<Self> {
        config.validate()?;
        let timeout = config.connect_timeout;
        time::timeout(timeout, handshake(config))
            .await
            .map_err(|_| Error::protocol("logon timed out"))?
    }

    pub async fn send(&self, msg: AppMessage) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Send { msg, reply })
            .await
            .map_err(|_| Error::Disconnected)?;
        rx.await.map_err(|_| Error::Disconnected)?
    }

    pub async fn logout(&self, text: Option<String>) -> Result<()> {
        let (reply, rx) = oneshot::channel();
        self.commands
            .send(Command::Logout { text, reply })
            .await
            .map_err(|_| Error::Disconnected)?;
        rx.await.map_err(|_| Error::Disconnected)?
    }

    pub async fn recv(&mut self) -> Result<Envelope> {
        match self.inbound.recv().await {
            Some(item) => item,
            None => Err(Error::Disconnected),
        }
    }

    pub fn try_recv(&mut self) -> Result<Option<Envelope>> {
        match self.inbound.try_recv() {
            Ok(item) => item.map(Some),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(Error::Disconnected),
        }
    }
}

async fn handshake(config: SessionConfig) -> Result<FixSession> {
    let tcp = TcpStream::connect((config.endpoint.host.as_str(), config.endpoint.port))
        .await
        .map_err(|e| {
            Error::Io(std::io::Error::new(
                e.kind(),
                format!("connect {}: {e}", config.endpoint),
            ))
        })?;
    tcp.set_nodelay(true)?;
    let connector = TlsConnector::from(client_config(&config)?);
    let name = server_name(&config.endpoint.host)?;
    let tls = connector
        .connect(name, tcp)
        .await
        .map_err(|e| Error::Tls(e.to_string()))?;
    let (mut reader, mut writer) = tokio::io::split(tls);

    let mut limiter = RateLimiter::new();
    limiter.check(SystemTime::now(), traffic("A"))?;
    let now = format_millis(Utc::now());
    let logon = build_logon(
        &config.credentials,
        &now,
        config.heartbeat_secs,
        config.reset_seq_num,
        config.cancel_on_disconnect,
        config.gateway == GatewayKind::DropCopy,
    )?;
    let header = OutHeader::from_parts(
        &config.credentials.api_key,
        TARGET_COMP_ID,
        1,
        &logon.sending_time,
    )?;
    writer
        .write_all(&encode(&header, "A", &logon.message.fields)?)
        .await?;
    writer.flush().await?;

    let mut buf = Vec::new();
    let msg = read_message(&mut reader, &mut buf).await?;
    match msg.msg_type()? {
        "A" => {
            if msg.seq()? != 1 {
                return Err(Error::protocol(format!(
                    "logon MsgSeqNum is {}, expected 1",
                    msg.seq()?
                )));
            }
            if msg
                .get(crate::tags::SENDER_COMP_ID)
                .is_some_and(|sender| sender != TARGET_COMP_ID)
            {
                return Err(Error::protocol(format!(
                    "logon SenderCompID is {}",
                    msg.get(crate::tags::SENDER_COMP_ID).unwrap_or("")
                )));
            }
        }
        "5" => {
            return Err(Error::Rejected(
                msg.get(TEXT).unwrap_or("logon rejected").to_string(),
            ));
        }
        "3" => {
            return Err(Error::Rejected(
                msg.get(TEXT).unwrap_or("session reject").to_string(),
            ));
        }
        other => return Err(Error::protocol(format!("expected logon, got {other}"))),
    }

    let (commands_tx, commands_rx) = mpsc::channel(32);
    let (inbound_tx, inbound_rx) = mpsc::channel(INBOUND_QUEUE);
    let api_key = config.credentials.api_key.clone();
    let heartbeat = Duration::from_secs(config.heartbeat_secs);
    let resume_from = config.resume_from.filter(|_| !config.reset_seq_num);
    let task = tokio::spawn(async move {
        let input = SessionLoop {
            commands: commands_rx,
            inbound: inbound_tx.clone(),
            limiter,
            api_key,
            heartbeat,
            resume_from,
        };
        if let Err(err) = run_session(reader, writer, buf, input).await {
            let _ = inbound_tx.try_send(Err(err));
        }
    });
    Ok(FixSession {
        commands: commands_tx,
        inbound: inbound_rx,
        task,
    })
}

struct SessionLoop {
    commands: mpsc::Receiver<Command>,
    inbound: mpsc::Sender<Result<Envelope>>,
    limiter: RateLimiter,
    api_key: String,
    heartbeat: Duration,
    resume_from: Option<u32>,
}

async fn run_session<R, W>(
    reader: R,
    mut writer: W,
    mut buf: Vec<u8>,
    input: SessionLoop,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send,
{
    let SessionLoop {
        mut commands,
        inbound,
        mut limiter,
        api_key,
        heartbeat,
        resume_from,
    } = input;
    let (decoded_tx, mut decoded_rx) = mpsc::channel::<Result<Message>>(256);
    let reader_task = tokio::spawn(async move {
        let mut reader = reader;
        loop {
            match read_message(&mut reader, &mut buf).await {
                Ok(msg) => {
                    if decoded_tx.send(Ok(msg)).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = decoded_tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });

    let mut engine = Engine::after_logon();
    if let Some(seq) = resume_from {
        engine.set_resume_from(seq);
    }
    let mut next_tx = Instant::now() + heartbeat;
    let mut peer_deadline = Instant::now() + heartbeat + heartbeat / 2;
    let mut awaiting_test = false;
    let mut test_serial = 0u64;
    let sleep = time::sleep_until(next_tx.min(peer_deadline));
    tokio::pin!(sleep);

    let outcome = loop {
        tokio::select! {
            biased;
            incoming = decoded_rx.recv() => {
                let Some(incoming) = incoming else { break Err(Error::Disconnected); };
                let msg = match incoming {
                    Ok(msg) => msg,
                    Err(err) => break Err(err),
                };
                let now = Instant::now();
                peer_deadline = now + heartbeat + heartbeat / 2;
                awaiting_test = false;
                let actions = engine.on_inbound(msg)?;
                if let Err(err) = dispatch(&mut writer, &mut limiter, &api_key, &inbound, actions).await {
                    break Err(err);
                }
                sleep.as_mut().reset(next_tx.min(peer_deadline));
            }
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { break Ok(()); };
                let (msg, reply) = match cmd {
                    Command::Send { msg, reply } => (msg, reply),
                    Command::Logout { text, reply } => {
                        let mut fields = Vec::new();
                        if let Some(text) = text {
                            fields.push((TEXT, text));
                        }
                        (AppMessage::new("5", fields), reply)
                    }
                };
                let actions = engine.on_app(msg)?;
                let result = dispatch(&mut writer, &mut limiter, &api_key, &inbound, actions).await;
                let _ = reply.send(match &result {
                    Ok(()) => Ok(()),
                    Err(err) => Err(err.same_kind()),
                });
                if result.is_err() {
                    break result;
                }
                next_tx = Instant::now() + heartbeat;
                sleep.as_mut().reset(next_tx.min(peer_deadline));
            }
            _ = &mut sleep => {
                let now = Instant::now();
                if now >= peer_deadline {
                    if awaiting_test {
                        break Err(Error::protocol("no answer to TestRequest before twice the heartbeat interval"));
                    }
                    test_serial += 1;
                    let id = format!("t{test_serial}");
                    awaiting_test = true;
                    peer_deadline = now + heartbeat / 2;
                    let msg = AppMessage::new("1", vec![(crate::tags::TEST_REQ_ID, id)]);
                    let actions = engine.on_app(msg)?;
                    if let Err(err) = dispatch(&mut writer, &mut limiter, &api_key, &inbound, actions).await {
                        break Err(err);
                    }
                }
                if Instant::now() >= next_tx {
                    let msg = AppMessage::new("0", vec![]);
                    let actions = engine.on_app(msg)?;
                    if let Err(err) = dispatch(&mut writer, &mut limiter, &api_key, &inbound, actions).await {
                        break Err(err);
                    }
                    next_tx = Instant::now() + heartbeat;
                }
                sleep.as_mut().reset(next_tx.min(peer_deadline));
            }
        }
    };
    reader_task.abort();
    outcome
}

async fn dispatch<W: AsyncWrite + Unpin>(
    writer: &mut W,
    limiter: &mut RateLimiter,
    api_key: &str,
    inbound: &mpsc::Sender<Result<Envelope>>,
    actions: Vec<Action>,
) -> Result<()> {
    for action in actions {
        match action {
            Action::Send(out) => write_out(writer, limiter, api_key, &out).await?,
            Action::Deliver(env) => {
                if inbound.try_send(Ok(*env)).is_err() {
                    return Err(Error::protocol(
                        "inbound queue is full; recv was not keeping up",
                    ));
                }
            }
            Action::Disconnect(reason) => {
                if reason != "peer logged out"
                    && inbound
                        .try_send(Err(Error::protocol(reason.clone())))
                        .is_err()
                {
                    return Err(Error::protocol(reason));
                }
                return Err(Error::protocol(reason));
            }
        }
    }
    Ok(())
}

async fn write_out<W: AsyncWrite + Unpin>(
    writer: &mut W,
    limiter: &mut RateLimiter,
    api_key: &str,
    out: &Outbound,
) -> Result<()> {
    limiter.check(SystemTime::now(), traffic(&out.msg_type))?;
    let mut header =
        OutHeader::from_parts(api_key, TARGET_COMP_ID, out.seq, format_millis(Utc::now()))?;
    header.poss_dup = out.poss_dup;
    header.sender_sub_id = out.sender_sub_id.clone();
    let bytes = encode(&header, &out.msg_type, &out.fields)?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

pub(crate) async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> Result<Message> {
    loop {
        match decode_one(buf)? {
            Decode::Frame { message, consumed } => {
                buf.drain(..consumed);
                return Ok(message);
            }
            Decode::Incomplete { discard } => {
                if discard > 0 {
                    buf.drain(..discard.min(buf.len()));
                }
            }
        }
        let mut tmp = [0u8; 8192];
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            return Err(Error::Disconnected);
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_FRAME {
            return Err(Error::codec("FIX message exceeds 8MiB"));
        }
    }
}

fn client_config(config: &SessionConfig) -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for der in &config.extra_root_ders {
        roots
            .add(CertificateDer::from(der.clone()))
            .map_err(|e| Error::Tls(e.to_string()))?;
    }
    let client = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS12])
        .map_err(|e| Error::Tls(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(client))
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites = vec![
        rustls::crypto::ring::cipher_suite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        rustls::crypto::ring::cipher_suite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        // The in-crate handshake test uses an ECDSA certificate. Coinbase's gateway
        // certificate is RSA and selects one of the two suites above.
        rustls::crypto::ring::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
    ];
    provider.kx_groups = vec![rustls::crypto::ring::kx_group::SECP256R1];
    Arc::new(provider)
}

fn server_name(host: &str) -> Result<ServerName<'static>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        Ok(ServerName::IpAddress(ip.into()))
    } else {
        ServerName::try_from(host.to_string())
            .map_err(|e| Error::Tls(format!("server name `{host}`: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{OutHeader, encode};
    use crate::order::{LimitOrder, Side, TimeInForce};
    use crate::parse::Incoming;
    use rustls::ServerConfig;
    use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    const SECRET: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

    #[tokio::test]
    async fn logon_order_execution_and_logout() {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = certified.cert.der().to_vec();
        let key_der = certified.key_pair.serialize_der();
        let server_cfg = ServerConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert_der.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
            )
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_cfg));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let tls = acceptor.accept(tcp).await.unwrap();
            let (mut reader, mut writer) = tokio::io::split(tls);
            let mut buf = Vec::new();
            let logon = read_message(&mut reader, &mut buf).await.unwrap();
            assert_eq!(logon.msg_type().unwrap(), "A");
            assert_eq!(logon.get(96).map(|s| s.len()), Some(44));
            assert_eq!(logon.get(1137), Some("9"));
            assert_eq!(logon.seq().unwrap(), 1);
            let header =
                OutHeader::from_parts("Coinbase", "test-key", 1, "20230822-20:43:30.000").unwrap();
            writer
                .write_all(
                    &encode(
                        &header,
                        "A",
                        &[(98, "0".into()), (108, "30".into()), (1137, "9".into())],
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
            let order = read_message(&mut reader, &mut buf).await.unwrap();
            assert_eq!(order.msg_type().unwrap(), "D");
            let mut exec =
                OutHeader::from_parts("Coinbase", "test-key", 2, "20230822-20:43:30.000").unwrap();
            exec.sending_time = format_millis(Utc::now());
            let fields = vec![
                (11, order.get(11).unwrap().to_string()),
                (37, "2985ca2d-61ef-49f1-bfce-6c39d8462914".into()),
                (17, "3985ca2d-61ef-49f1-bfce-6c39d8462914".into()),
                (39, "0".into()),
                (150, "0".into()),
                (55, "BTC-USD".into()),
                (54, "1".into()),
                (40, "2".into()),
                (44, "60000".into()),
                (38, "0.01".into()),
                (14, "0".into()),
                (151, "0.01".into()),
                (6, "0".into()),
                (60, exec.sending_time.clone()),
            ];
            writer
                .write_all(&encode(&exec, "8", &fields).unwrap())
                .await
                .unwrap();
            let bye = read_message(&mut reader, &mut buf).await.unwrap();
            assert_eq!(bye.msg_type().unwrap(), "5");
            let mut reply =
                OutHeader::from_parts("Coinbase", "test-key", 3, format_millis(Utc::now()))
                    .unwrap();
            reply.sending_time = format_millis(Utc::now());
            writer
                .write_all(&encode(&reply, "5", &[(58, "bye".into())]).unwrap())
                .await
                .unwrap();
        });

        let creds = Credentials::new("test-key", SECRET, "passphrase").unwrap();
        let cfg = SessionConfig::new(
            Environment::Sandbox,
            Venue::Exchange,
            GatewayKind::OrderEntry,
            creds,
        )
        .endpoint(Endpoint::new("localhost", addr.port()))
        .extra_root_der(cert_der)
        .heartbeat_secs(30);
        let mut session = FixSession::connect(cfg).await.unwrap();
        let order = LimitOrder {
            cl_ord_id: crate::new_client_order_id(),
            symbol: "BTC-USD".into(),
            side: Side::Buy,
            order_qty: "0.01".into(),
            price: "60000".into(),
            time_in_force: TimeInForce::GoodTillCancel,
            expire_time: None,
            post_only: true,
            self_trade: None,
            exn: false,
            sender_sub_id: None,
        }
        .to_message()
        .unwrap();
        session.send(order).await.unwrap();
        let env = time::timeout(Duration::from_secs(2), session.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(env.body, Incoming::ExecutionReport(_)));
        session.logout(Some("done".into())).await.unwrap();
        server.await.unwrap();
    }
}
