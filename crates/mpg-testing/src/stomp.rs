//! Mini STOMP broker + peer client + adapter starter.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mpg_core::Engine;
use mpg_stomp::{decode_one, Frame, StompAdapter, StompConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

pub struct MiniBroker {
    pub addr: SocketAddr,
    shutdown: CancellationToken,
}

pub async fn start_mini_broker() -> MiniBroker {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    tokio::spawn(async move {
        run_broker(listener, token).await;
    });
    MiniBroker { addr, shutdown }
}

impl MiniBroker {
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

#[derive(Clone)]
struct Sub {
    conn_id: u64,
    sub_id: String,
    tx: mpsc::Sender<Frame>,
}

struct BrokerState {
    subs: HashMap<String, Vec<Sub>>,
    next_msg: AtomicU64,
}

async fn run_broker(listener: TcpListener, shutdown: CancellationToken) {
    let state = Arc::new(Mutex::new(BrokerState {
        subs: HashMap::new(),
        next_msg: AtomicU64::new(1),
    }));
    let mut conn_seq = 1u64;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let conn_id = conn_seq;
                conn_seq += 1;
                let state = Arc::clone(&state);
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    let _ = handle_conn(stream, conn_id, state, shutdown).await;
                });
            }
        }
    }
}

async fn handle_conn(
    stream: TcpStream,
    conn_id: u64,
    state: Arc<Mutex<BrokerState>>,
    shutdown: CancellationToken,
) -> Result<(), ()> {
    let (mut read, mut write) = stream.into_split();
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
            _ = shutdown.cancelled() => break,
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
                .with_header("server", "mpg-mini-stomp/0.1")
                .with_header("heart-beat", "0,0");
            out_tx.send(reply).await.map_err(|_| ())?;
        }
        "SUBSCRIBE" => {
            let dest = frame.header("destination").ok_or(())?.to_owned();
            let sub_id = frame.header("id").unwrap_or("0").to_owned();
            let mut guard = state.lock().await;
            guard.subs.entry(dest).or_default().push(Sub {
                conn_id,
                sub_id,
                tx: out_tx.clone(),
            });
        }
        "UNSUBSCRIBE" => {
            let sub_id = frame.header("id").ok_or(())?.to_owned();
            let mut guard = state.lock().await;
            for subs in guard.subs.values_mut() {
                subs.retain(|s| !(s.conn_id == conn_id && s.sub_id == sub_id));
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
                    .with_header("subscription", sub.sub_id.clone())
                    .with_header("message-id", mid.to_string());
                for (k, v) in &frame.headers {
                    if k == "destination" {
                        continue;
                    }
                    msg.headers.push((k.clone(), v.clone()));
                }
                msg.body = frame.body.clone();
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

pub struct StompPeer {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl StompPeer {
    pub async fn connect(addr: SocketAddr) -> Self {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.set_nodelay(true).ok();
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

    pub async fn subscribe(&mut self, id: &str, dest: &str) {
        let frame = Frame::new("SUBSCRIBE")
            .with_header("id", id)
            .with_header("destination", dest)
            .with_header("ack", "auto");
        self.stream.write_all(&frame.encode()).await.unwrap();
    }

    pub async fn send(&mut self, dest: &str, body: &[u8], extra: &[(&str, &str)]) {
        let mut frame = Frame::new("SEND")
            .with_header("destination", dest)
            .with_body(body.to_vec());
        for (k, v) in extra {
            frame = frame.with_header(*k, *v);
        }
        self.stream.write_all(&frame.encode()).await.unwrap();
    }

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

pub async fn start_stomp_adapter(
    engine: Arc<Engine>,
    id: impl Into<String>,
    broker: SocketAddr,
    topics: Vec<String>,
    settle: Duration,
) -> CancellationToken {
    let shutdown = CancellationToken::new();
    let adapter = Arc::new(StompAdapter::new(
        id.into(),
        StompConfig {
            broker,
            host: "/".into(),
            login: None,
            passcode: None,
            destination_prefix: "/topic/".into(),
            topics,
            unwrap_ma_payloads: true,
            reconnect: false,
            connect_timeout: Duration::from_secs(5),
            max_frame_size: mpg_stomp::DEFAULT_MAX_FRAME_SIZE,
        },
    ));
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
