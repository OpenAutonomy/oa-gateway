//! `Engine::publish` → N subscriber channels.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use oa_gateway_core::{ContentType, Delivery, Engine, Envelope, RouteKey};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use crate::cli::EngineArgs;
use crate::clock::SeqClock;
use crate::payload::{self, PayloadKind};
use crate::report;
use crate::scenarios::drain_until_quiet;

/// Runs the engine scenario.
///
/// # Errors
///
/// Returns a message if subscribe fails, nothing is received, or the
/// JSON file cannot be written.
pub(crate) async fn run(args: EngineArgs) -> Result<(), String> {
    let engine = Arc::new(Engine::new());
    let clock = Arc::new(SeqClock::new());
    let samples = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::new(AtomicU64::new(0));
    let unmatched = Arc::new(AtomicU64::new(0));
    let warmup_ns = args.shared.warmup.as_nanos() as u64;
    let kind = PayloadKind::Ping;

    for i in 0..args.subscribers {
        let (tx, mut rx) = mpsc::channel::<Delivery>(args.capacity);
        engine
            .subscribe(
                "bench",
                format!("s{i}"),
                RouteKey::typed(kind.topic(), kind.type_hint()),
                tx,
            )
            .await
            .map_err(|err| err.to_string())?;
        let clock = Arc::clone(&clock);
        let samples = Arc::clone(&samples);
        let received = Arc::clone(&received);
        let unmatched = Arc::clone(&unmatched);
        tokio::spawn(async move {
            while let Some(delivery) = rx.recv().await {
                received.fetch_add(1, Ordering::Relaxed);
                let Some(seq) = payload::parse_seq(&delivery.envelope.payload) else {
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
        });
    }

    let started = Instant::now();
    let deadline = started + args.shared.duration;
    let mut sent = 0u64;
    let mut dropped = 0u64;
    let mut seq = 0u64;
    let mut ticker = rate_ticker(args.shared.rate);

    while Instant::now() < deadline {
        if let Some(tick) = ticker.as_mut() {
            tick.tick().await;
        }
        let body = payload::render(kind, seq, args.shared.payload_bytes);
        let env = Envelope::new(
            RouteKey::typed(kind.topic(), kind.type_hint()),
            Bytes::from(body),
        )
        .with_content_type(ContentType::json());
        clock.stamp(seq);
        let outcome = engine.publish(env).await;
        dropped += outcome.dropped as u64;
        sent += 1;
        seq += 1;
    }

    drain_until_quiet(&received).await;

    let recv = received.load(Ordering::Relaxed);
    if recv == 0 {
        return Err("engine received 0 deliveries".into());
    }

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
    flags.insert("subscribers".into(), json!(args.subscribers));
    flags.insert("capacity".into(), json!(args.capacity));

    let samples = samples.lock().expect("samples").clone();
    let mut report = report::blank("engine", flags);
    report.sent = sent;
    report.received = recv;
    report.dropped = dropped;
    report.unmatched = unmatched.load(Ordering::Relaxed);
    report.duration_secs = started.elapsed().as_secs_f64();
    report.engine = Some(report::engine_snapshot(&engine));
    let report = report.finish(samples, None, args.shared.payload_bytes as u64);
    report.emit(args.shared.json.as_deref())
}

pub(crate) fn rate_ticker(rate: u64) -> Option<tokio::time::Interval> {
    if rate == 0 {
        return None;
    }
    let period = Duration::from_secs_f64(1.0 / rate as f64);
    let mut tick = interval(period);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    Some(tick)
}
