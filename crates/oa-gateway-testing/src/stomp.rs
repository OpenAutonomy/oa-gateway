//! Mini STOMP broker, a peer client, and an adapter starter.
//!
//! [`start_mini_broker`] is an in-process stand-in for ActiveMQ
//! Classic: exact-destination fan-out, no auth, no persistence, no
//! heartbeats. Most tests need no Docker. [`start_stomp_adapter`]
//! turns reconnect off so a failed session fails the test instead of
//! looping. Helpers panic on timeout or a surprising frame.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use oa_gateway_adapter::tls::{ClientTls, MaybeTlsStream, ServerTls};
use oa_gateway_core::Engine;
use oa_gateway_stomp::{decode_one, Frame, StompAdapter, StompConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::tls::{TestCa, TestCerts};

/// In-process STOMP 1.2 broker on an ephemeral port.
///
/// Speaks CONNECT/STOMP, SUBSCRIBE, UNSUBSCRIBE, SEND, and DISCONNECT.
/// SEND fans out to subscribers of that exact destination. There is no
/// queue, no durable sub, and no login check. Call [`Self::shutdown`]
/// when the test is done so the accept loop stops.
pub struct MiniBroker {
    pub addr: SocketAddr,
    shutdown: CancellationToken,
}

/// Binds `127.0.0.1:0` and serves until [`MiniBroker::shutdown`].
///
/// Panics if the port cannot be bound.
pub async fn start_mini_broker() -> MiniBroker {
    start_mini_broker_with(None).await
}

/// As [`start_mini_broker`], but the broker terminates TLS with `certs`.
///
/// Connect to it with [`StompPeer::connect_tls`] or a [`StompConfig`]
/// carrying a [`ClientTls`].
pub async fn start_mini_broker_tls(certs: &TestCerts) -> MiniBroker {
    start_mini_broker_with(Some(crate::tls::server_tls(certs))).await
}

/// As [`start_mini_broker_tls`], but the broker also requires and verifies
/// a client certificate issued by `client_ca` from every connection.
pub async fn start_mini_broker_mtls(certs: &TestCerts, client_ca: &TestCa) -> MiniBroker {
    start_mini_broker_with(Some(crate::tls::server_tls_with_client_ca(
        certs, client_ca,
    )))
    .await
}

async fn start_mini_broker_with(tls: Option<ServerTls>) -> MiniBroker {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    tokio::spawn(async move {
        run_broker(listener, token, tls).await;
    });
    MiniBroker { addr, shutdown }
}

impl MiniBroker {
    /// Stops the accept loop. Open connections close on the next read.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

/// One SUBSCRIBE: which connection, which `id`, and where to write
/// MESSAGE frames.
#[derive(Clone)]
struct Sub {
    conn_id: u64,
    id: String,
    tx: mpsc::Sender<Frame>,
}

/// Destinations → subscribers, plus a process-wide `message-id`.
struct BrokerState {
    subs: HashMap<String, Vec<Sub>>,
    next_msg: AtomicU64,
}

/// Accepts connections until `shutdown`. Each socket is its own task.
async fn run_broker(listener: TcpListener, shutdown: CancellationToken, tls: Option<ServerTls>) {
    let state = Arc::new(Mutex::new(BrokerState {
        subs: HashMap::new(),
        next_msg: AtomicU64::new(1),
    }));
    let mut conn_seq = 1u64;
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let conn_id = conn_seq;
                conn_seq += 1;
                let state = Arc::clone(&state);
                let shutdown = shutdown.clone();
                let tls = tls.clone();
                tokio::spawn(async move {
                    let _ = handle_conn(stream, conn_id, state, shutdown, tls).await;
                });
            }
        }
    }
}

/// Reads frames until the peer closes or `shutdown` fires, then drops
/// this connection's subscriptions.
async fn handle_conn(
    stream: TcpStream,
    conn_id: u64,
    state: Arc<Mutex<BrokerState>>,
    shutdown: CancellationToken,
    tls: Option<ServerTls>,
) -> Result<(), ()> {
    let stream: MaybeTlsStream<TcpStream> = match &tls {
        Some(tls) => tls.accept(stream).await.map_err(|_| ())?,
        None => MaybeTlsStream::Plain(stream),
    };
    let (mut read, mut write) = tokio::io::split(stream);
    let (out_tx, mut out_rx) = mpsc::channel::<Frame>(64);
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            if write.write_all(&frame.encode()).await.is_err() {
                break;
            }
        }
    });

    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            n = read.read(&mut tmp) => {
                let n = n.map_err(|_| ())?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                while let Some(frame) = decode_one(&mut buf).map_err(|_| ())? {
                    if dispatch(conn_id, &state, &out_tx, frame).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    {
        let mut guard = state.lock().await;
        for subs in guard.subs.values_mut() {
            subs.retain(|s| s.conn_id != conn_id);
        }
        guard.subs.retain(|_, v| !v.is_empty());
    }
    writer.abort();
    Ok(())
}

/// Handles one client frame. Unknown commands get an ERROR. SEND
/// copies headers except `destination` and assigns `message-id`.
async fn dispatch(
    conn_id: u64,
    state: &Mutex<BrokerState>,
    out_tx: &mpsc::Sender<Frame>,
    frame: Frame,
) -> Result<(), ()> {
    match frame.command.as_str() {
        "CONNECT" | "STOMP" => {
            let reply = Frame::new("CONNECTED")
                .with_header("version", "1.2")
                .with_header("server", "oa-gateway-mini-stomp/0.1")
                .with_header("heart-beat", "0,0");
            out_tx.send(reply).await.map_err(|_| ())?;
        }
        "SUBSCRIBE" => {
            let dest = frame.header("destination").ok_or(())?.to_owned();
            let sub_id = frame.header("id").unwrap_or("0").to_owned();
            let mut guard = state.lock().await;
            guard.subs.entry(dest).or_default().push(Sub {
                conn_id,
                id: sub_id,
                tx: out_tx.clone(),
            });
        }
        "UNSUBSCRIBE" => {
            let sub_id = frame.header("id").ok_or(())?.to_owned();
            let mut guard = state.lock().await;
            for subs in guard.subs.values_mut() {
                subs.retain(|s| !(s.conn_id == conn_id && s.id == sub_id));
            }
        }
        "SEND" => {
            let dest = frame.header("destination").ok_or(())?.to_owned();
            let guard = state.lock().await;
            let mid = guard.next_msg.fetch_add(1, Ordering::Relaxed);
            let targets: Vec<Sub> = guard.subs.get(&dest).cloned().unwrap_or_default();
            drop(guard);
            for sub in targets {
                let mut msg = Frame::new("MESSAGE")
                    .with_header("destination", dest.clone())
                    .with_header("subscription", sub.id.clone())
                    .with_header("message-id", mid.to_string());
                for (k, v) in &frame.headers {
                    if k == "destination" {
                        continue;
                    }
                    msg.headers.push((k.clone(), v.clone()));
                }
                msg.body.clone_from(&frame.body);
                let _ = sub.tx.send(msg).await;
            }
        }
        "DISCONNECT" => {
            let receipt = Frame::new("RECEIPT").with_header("receipt-id", "disconnect");
            let _ = out_tx.send(receipt).await;
        }
        _ => {
            let err =
                Frame::new("ERROR").with_header("message", format!("unknown {}", frame.command));
            let _ = out_tx.send(err).await;
        }
    }
    Ok(())
}

/// Raw STOMP client for tests: CONNECT, SUBSCRIBE, SEND, recv.
///
/// Not the gateway adapter. Use this as the "other side" of
/// [`start_mini_broker`] or a live ActiveMQ.
pub struct StompPeer {
    stream: MaybeTlsStream<TcpStream>,
    buf: Vec<u8>,
}

impl StompPeer {
    /// CONNECT 1.2 / `host` `/` / no heartbeats, then wait for
    /// CONNECTED.
    ///
    /// Panics if the socket cannot be opened or the first frame is not
    /// CONNECTED.
    pub async fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect(addr).await.unwrap();
        stream.set_nodelay(true).ok();
        Self::handshake(MaybeTlsStream::Plain(stream)).await
    }

    /// As [`Self::connect`], negotiating TLS per `tls` before CONNECT.
    ///
    /// Panics if the socket cannot be opened, the TLS handshake fails, or
    /// the first frame after CONNECT is not CONNECTED.
    pub async fn connect_tls(addr: SocketAddr, tls: &ClientTls) -> Self {
        let stream = TcpStream::connect(addr).await.unwrap();
        stream.set_nodelay(true).ok();
        let stream = tls.connect(stream).await.expect("tls handshake");
        Self::handshake(stream).await
    }

    async fn handshake(mut stream: MaybeTlsStream<TcpStream>) -> Self {
        let connect = Frame::new("CONNECT")
            .with_header("accept-version", "1.2")
            .with_header("host", "/")
            .with_header("heart-beat", "0,0");
        stream.write_all(&connect.encode()).await.unwrap();
        let mut peer = Self {
            stream,
            buf: Vec::new(),
        };
        let frame = peer.recv().await;
        assert_eq!(frame.command, "CONNECTED");
        peer
    }

    /// SUBSCRIBE with `ack:auto`. Does not wait for a receipt.
    pub async fn subscribe(&mut self, id: &str, dest: &str) {
        let frame = Frame::new("SUBSCRIBE")
            .with_header("id", id)
            .with_header("destination", dest)
            .with_header("ack", "auto");
        self.stream.write_all(&frame.encode()).await.unwrap();
    }

    /// SEND `body` to `dest`, then any `extra` headers.
    pub async fn send(&mut self, dest: &str, body: &[u8], extra: &[(&str, &str)]) {
        let mut frame = Frame::new("SEND")
            .with_header("destination", dest)
            .with_body(body.to_vec());
        for (k, v) in extra {
            frame = frame.with_header(*k, *v);
        }
        self.stream.write_all(&frame.encode()).await.unwrap();
    }

    /// Next complete frame. Waits up to two seconds.
    ///
    /// Panics on timeout or a clean close.
    pub async fn recv(&mut self) -> Frame {
        let mut tmp = [0u8; 8192];
        loop {
            if let Some(frame) = decode_one(&mut self.buf).unwrap() {
                return frame;
            }
            let n = timeout(Duration::from_secs(2), self.stream.read(&mut tmp))
                .await
                .expect("peer recv timeout")
                .unwrap();
            assert!(n > 0, "peer connection closed");
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }
}

/// Starts [`StompAdapter`] toward `broker` and waits until it is
/// subscribed.
///
/// Reconnect is off so a dropped session fails the test. A-GRA unwrap
/// is on. `settle` is an extra sleep after `serve_ready` — live
/// ActiveMQ needs a few hundred milliseconds for SUBSCRIBE to land;
/// the mini broker can use [`Duration::ZERO`].
///
/// Panics if the adapter is not ready within five seconds.
pub async fn start_stomp_adapter(
    engine: Arc<Engine>,
    id: impl Into<String>,
    broker: SocketAddr,
    topics: Vec<String>,
    settle: Duration,
) -> CancellationToken {
    start_stomp_adapter_with(engine, id, broker, topics, settle, |_| {}).await
}

/// As [`start_stomp_adapter`], with the config open for editing first —
/// for example to set [`StompConfig::tls`].
pub async fn start_stomp_adapter_with(
    engine: Arc<Engine>,
    id: impl Into<String>,
    broker: SocketAddr,
    topics: Vec<String>,
    settle: Duration,
    edit: impl FnOnce(&mut StompConfig),
) -> CancellationToken {
    let shutdown = CancellationToken::new();
    let mut config = StompConfig {
        broker,
        topics,
        reconnect: false,
        ..StompConfig::default()
    };
    edit(&mut config);
    let adapter = Arc::new(StompAdapter::new(id.into(), config));
    let (ready_tx, ready_rx) = oneshot::channel();
    let token = shutdown.clone();
    tokio::spawn(async move {
        adapter
            .serve_ready(engine, token, ready_tx)
            .await
            .expect("stomp adapter");
    });
    timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("adapter ready timeout")
        .expect("adapter dropped ready");
    if !settle.is_zero() {
        tokio::time::sleep(settle).await;
    }
    shutdown
}
