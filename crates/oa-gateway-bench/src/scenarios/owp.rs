//! WebSocket PUB → MSG, embedded or attached.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::cli::OwpArgs;
use crate::clock::SeqClock;
use crate::owp_client::{self, OwpWs};
use crate::payload::{self, PayloadKind};
use crate::report;
use crate::scenarios::drain_until_quiet;
use crate::scenarios::engine::rate_ticker;
use futures_util::{SinkExt, StreamExt};
use oa_gateway_core::Engine;
use oa_gateway_owp::ServerOp;
use oa_gateway_testing::owp::start_owp;
use serde_json::json;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

/// Runs the OWP scenario.
///
/// # Errors
///
/// Returns a message if the socket path fails, nothing is received, or
/// the JSON file cannot be written.
pub(crate) async fn run(args: OwpArgs) -> Result<(), String> {
    let kind = if args.xml_baseline {
        PayloadKind::PositionReport
    } else {
        PayloadKind::Ping
    };
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = if let Some(url) = &args.url {
        (url.clone(), None)
    } else {
        let (url, token) = start_owp(engine.clone(), args.xml_baseline).await;
        (url, Some(token))
    };

    let mut subscribers = Vec::new();
    for i in 0..args.subscribers {
        let mut ws = owp_client::connect(&url).await?;
        // Verbose so SUB is acknowledged before any publisher starts.
        owp_client::handshake(&mut ws, true).await?;
        owp_client::subscribe(&mut ws, &ws_sid(i), kind.type_hint(), kind.topic(), true).await?;
        subscribers.push(ws);
    }

    let mut publishers = Vec::new();
    for _ in 0..args.publishers {
        let mut ws = owp_client::connect(&url).await?;
        owp_client::handshake(&mut ws, args.ack_latency).await?;
        publishers.push(ws);
    }

    let clock = Arc::new(SeqClock::new());
    let samples = Arc::new(Mutex::new(Vec::new()));
    let ack_samples = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let unmatched = Arc::new(AtomicU64::new(0));
    let warmup_ns = args.shared.warmup.as_nanos() as u64;

    for ws in subscribers {
        let clock = Arc::clone(&clock);
        let samples = Arc::clone(&samples);
        let received = Arc::clone(&received);
        let errors = Arc::clone(&errors);
        let unmatched = Arc::clone(&unmatched);
        tokio::spawn(async move {
            drain_subscriber(ws, clock, samples, received, errors, unmatched, warmup_ns).await;
        });
    }

    let next_seq = Arc::new(AtomicU64::new(0));
    let sent = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let deadline = started + args.shared.duration;
    let mut joins = Vec::new();

    for ws in publishers {
        let clock = Arc::clone(&clock);
        let ack_samples = Arc::clone(&ack_samples);
        let errors = Arc::clone(&errors);
        let next_seq = Arc::clone(&next_seq);
        let sent = Arc::clone(&sent);
        let payload_bytes = args.shared.payload_bytes;
        let rate = args.shared.rate;
        let ack_latency = args.ack_latency;
        joins.push(tokio::spawn(async move {
            publish_loop(
                ws,
                kind,
                clock,
                ack_samples,
                errors,
                next_seq,
                sent,
                deadline,
                payload_bytes,
                rate,
                ack_latency,
                warmup_ns,
            )
            .await;
        }));
    }

    for join in joins {
        let _ = join.await;
    }
    drain_until_quiet(&received).await;
    if let Some(token) = shutdown {
        token.cancel();
    }

    let recv = received.load(Ordering::Relaxed);
    if recv == 0 {
        return Err("owp received 0 MSG frames".into());
    }

    let scenario = if args.xml_baseline {
        "owp-xml-baseline"
    } else {
        "owp"
    };
    let mut flags = BTreeMap::new();
    flags.insert(
        "duration_secs".into(),
        json!(args.shared.duration.as_secs_f64()),
    );
    flags.insert(
        "warmup_secs".into(),
        json!(args.shared.warmup.as_secs_f64()),
    );
    flags.insert("rate".into(), json!(args.shared.rate));
    flags.insert("payload_bytes".into(), json!(args.shared.payload_bytes));
    flags.insert("publishers".into(), json!(args.publishers));
    flags.insert("subscribers".into(), json!(args.subscribers));
    flags.insert("xml_baseline".into(), json!(args.xml_baseline));
    flags.insert("ack_latency".into(), json!(args.ack_latency));
    if let Some(url) = &args.url {
        flags.insert("url".into(), json!(url));
    }

    let samples = samples.lock().expect("samples").clone();
    let acks = if args.ack_latency {
        Some(ack_samples.lock().expect("acks").clone())
    } else {
        None
    };
    let mut report = report::blank(scenario, flags);
    report.sent = sent.load(Ordering::Relaxed);
    report.received = recv;
    report.errors = errors.load(Ordering::Relaxed);
    report.unmatched = unmatched.load(Ordering::Relaxed);
    report.duration_secs = started.elapsed().as_secs_f64();
    if args.url.is_none() {
        report.engine = Some(report::engine_snapshot(&engine));
    }
    let report = report.finish(samples, acks, args.shared.payload_bytes as u64);
    report.emit(args.shared.json.as_deref())
}

fn ws_sid(i: usize) -> String {
    format!("sub-{i}")
}

async fn drain_subscriber(
    mut ws: OwpWs,
    clock: Arc<SeqClock>,
    samples: Arc<Mutex<Vec<u64>>>,
    received: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    unmatched: Arc<AtomicU64>,
    warmup_ns: u64,
) {
    loop {
        match timeout(Duration::from_millis(250), owp_client::recv_op(&mut ws)).await {
            Err(_) => {
                // Quiet period; keep going until the socket is dropped
                // after shutdown. A long idle after the run is fine.
                if received.load(Ordering::Relaxed) > 0 {
                    // Still wait for more; the outer drain decides when to finish.
                    continue;
                }
            }
            Ok(Err(_)) => break,
            Ok(Ok(ServerOp::Msg { payload, .. })) => {
                received.fetch_add(1, Ordering::Relaxed);
                let Some(seq) = payload::parse_seq(payload.as_bytes()) else {
                    unmatched.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                let Some(sent_ns) = clock.sent_ns(seq) else {
                    unmatched.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                if sent_ns < warmup_ns {
                    continue;
                }
                if let Some(ns) = clock.latency_ns(seq) {
                    samples.lock().expect("samples").push(ns);
                }
            }
            Ok(Ok(ServerOp::Err { .. })) => {
                errors.fetch_add(1, Ordering::Relaxed);
            }
            Ok(Ok(_)) => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn publish_loop(
    mut ws: OwpWs,
    kind: PayloadKind,
    clock: Arc<SeqClock>,
    ack_samples: Arc<Mutex<Vec<u64>>>,
    errors: Arc<AtomicU64>,
    next_seq: Arc<AtomicU64>,
    sent: Arc<AtomicU64>,
    deadline: Instant,
    payload_bytes: usize,
    rate: u64,
    ack_latency: bool,
    warmup_ns: u64,
) {
    let topic = kind.topic();
    let mut ticker = rate_ticker(rate);
    while Instant::now() < deadline {
        if let Some(tick) = ticker.as_mut() {
            tick.tick().await;
        }
        let seq = next_seq.fetch_add(1, Ordering::Relaxed);
        let body = payload::render(kind, seq, payload_bytes);
        clock.stamp(seq);
        let t0 = Instant::now();
        if owp_client::send_text(&mut ws, &format!("PUB {topic} {body}"))
            .await
            .is_err()
        {
            errors.fetch_add(1, Ordering::Relaxed);
            break;
        }
        sent.fetch_add(1, Ordering::Relaxed);
        if ack_latency {
            match owp_client::recv_op(&mut ws).await {
                Ok(ServerOp::Ok) => {
                    let ns = t0.elapsed().as_nanos() as u64;
                    if clock.sent_ns(seq).unwrap_or(0) >= warmup_ns {
                        ack_samples.lock().expect("acks").push(ns);
                    }
                }
                Ok(ServerOp::Err { .. }) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
                Ok(_) => {}
                Err(_) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    break;
                }
            }
        } else {
            // Drain control frames so a verbose server cannot fill the socket.
            drain_pending(&mut ws, &errors).await;
        }
    }
}

async fn drain_pending(ws: &mut OwpWs, errors: &AtomicU64) {
    loop {
        match timeout(Duration::from_millis(0), ws.next()).await {
            Ok(Some(Ok(Message::Ping(p)))) => {
                let _ = ws.send(Message::Pong(p)).await;
            }
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(ServerOp::Err { .. }) = oa_gateway_owp::parse_server(&t) {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(Some(Ok(_) | Err(_)) | None) | Err(_) => break,
        }
    }
}
