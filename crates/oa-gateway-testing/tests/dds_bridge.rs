//! DDS adapter talks only to the engine; a second rustdds participant is the peer.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_agra::{WrapperKind, WrapperMeta};
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_dds::{provider_for, DdsProviderKind, DdsSample};
use oa_gateway_loopback::Loopback;
use oa_gateway_testing::dds::{shipped_qos_path, start_dds_adapter, start_dds_adapter_with};
use oa_gateway_uci::ValidateMode;
use tokio::time::timeout;

const DOMAIN: u16 = 71;

#[tokio::test]
async fn dds_peer_reaches_loopback() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-dds");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let shutdown = start_dds_adapter(engine, "dds-test", DOMAIN, vec!["demo".into()]).await;

    let provider = provider_for(DdsProviderKind::Rustdds);
    let mut peer = provider.join(DOMAIN, &shipped_qos_path()).unwrap();
    peer.create_topic("demo").unwrap();

    let inner = Bytes::from_static(br#"{"Ping":{"n":7}}"#);
    let sample = DdsSample {
        meta: WrapperMeta {
            kind: WrapperKind::Rx,
            message_type_enum: "PING".into(),
            originator_uuid: None,
            rx_payload_id: None,
            command_id: None,
            destination_routing: None,
        },
        encoded: inner.clone(),
    };

    // VOLATILE: a write before SEDP match is lost. Retry until the
    // adapter has discovered this participant.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut got = None;
    while tokio::time::Instant::now() < deadline {
        peer.write("demo", sample.clone()).unwrap();
        match timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(env)) => {
                got = Some(env);
                break;
            }
            Ok(None) => panic!("channel closed"),
            Err(_) => {}
        }
    }
    let got = got.expect("timeout waiting for dds inbound");
    assert_eq!(got.route.topic, "demo");
    assert_eq!(got.route.type_hint.as_deref(), Some("Ping"));
    assert_eq!(got.content_type, ContentType::json());
    assert_eq!(got.payload, inner);
    assert_eq!(
        got.headers.get("oag.origin_adapter").map(String::as_str),
        Some("dds-test")
    );

    shutdown.cancel();
}

#[tokio::test]
async fn loopback_pub_does_not_echo_back() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-dds-echo");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let shutdown =
        start_dds_adapter(engine.clone(), "dds-echo", DOMAIN + 1, vec!["demo".into()]).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    engine
        .publish(Envelope::new(
            RouteKey::typed("demo", "Ping"),
            Bytes::from_static(br#"{"Ping":{"n":1}}"#),
        ))
        .await;

    let first = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("loopback should see its own engine publish")
        .expect("channel closed");
    assert_eq!(first.route.type_hint.as_deref(), Some("Ping"));

    let echoed = timeout(Duration::from_millis(500), rx.recv()).await;
    assert!(
        echoed.is_err(),
        "dds must not echo the outbound sample back"
    );

    shutdown.cancel();
}

/// Writes `sample` to `topic` on `peer` until `rx` sees it or `deadline`
/// passes. VOLATILE QoS loses a write before SEDP match, so establishing
/// the match is folded into the same retry loop the other tests use.
async fn write_until_matched(
    peer: &mut dyn oa_gateway_dds::DdsSession,
    topic: &str,
    sample: &DdsSample,
    rx: &mut tokio::sync::mpsc::Receiver<Envelope>,
) -> Envelope {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        peer.write(topic, sample.clone()).unwrap();
        if let Ok(Some(env)) = timeout(Duration::from_millis(200), rx.recv()).await {
            return env;
        }
    }
    panic!("timeout waiting for dds inbound");
}

/// Whether any envelope received on `rx` within `window` contains `needle`.
///
/// Used to check a specific payload never arrives, without assuming the
/// channel is otherwise empty — [`write_until_matched`]'s own retries can
/// leave a duplicate of an earlier, harmless delivery still in flight.
async fn any_payload_contains(
    rx: &mut tokio::sync::mpsc::Receiver<Envelope>,
    window: Duration,
    needle: &str,
) -> bool {
    let deadline = tokio::time::Instant::now() + window;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match timeout(remaining, rx.recv()).await {
            Ok(Some(env))
                if env
                    .payload
                    .windows(needle.len())
                    .any(|w| w == needle.as_bytes()) =>
            {
                return true;
            }
            Ok(Some(_)) => continue,
            _ => return false,
        }
    }
    false
}

fn rx_sample(inner: Bytes) -> DdsSample {
    DdsSample {
        meta: WrapperMeta {
            kind: WrapperKind::Rx,
            message_type_enum: "PING".into(),
            originator_uuid: None,
            rx_payload_id: None,
            command_id: None,
            destination_routing: None,
        },
        encoded: inner,
    }
}

#[tokio::test]
async fn an_oversized_sample_is_dropped_not_the_adapter() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-dds-oversize");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let shutdown = start_dds_adapter_with(
        engine,
        "dds-oversize",
        DOMAIN + 2,
        vec!["demo".into()],
        |config| config.max_sample_size = 64,
    )
    .await;

    let provider = provider_for(DdsProviderKind::Rustdds);
    let mut peer = provider.join(DOMAIN + 2, &shipped_qos_path()).unwrap();
    peer.create_topic("demo").unwrap();

    // Establishes SEDP match. write_until_matched's own retries can still
    // leave a duplicate delivery in flight afterward, so what follows
    // checks the oversized payload never arrives by content, not by
    // whether the channel stays empty.
    let small = rx_sample(Bytes::from_static(br#"{"Ping":{"n":7}}"#));
    write_until_matched(&mut *peer, "demo", &small, &mut rx).await;

    // Well-formed, and far too large: dropped for its size, not its content.
    let padded = format!(r#"{{"Ping":{{"n":"{}"}}}}"#, "x".repeat(200));
    let big = rx_sample(Bytes::from(padded.into_bytes()));
    peer.write("demo", big).unwrap();
    assert!(
        !any_payload_contains(&mut rx, Duration::from_millis(800), "xxxxx").await,
        "an oversized sample must not reach the engine"
    );

    // The adapter itself must still be alive and processing after dropping it.
    let after = write_until_matched(&mut *peer, "demo", &small, &mut rx).await;
    assert_eq!(after.payload.as_ref(), br#"{"Ping":{"n":7}}"#);

    shutdown.cancel();
}

#[tokio::test]
async fn reject_drops_a_sample_that_does_not_follow_the_schema() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-dds-reject");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let shutdown = start_dds_adapter_with(
        engine,
        "dds-reject",
        DOMAIN + 3,
        vec!["demo".into()],
        |config| config.validate = ValidateMode::Reject,
    )
    .await;

    let provider = provider_for(DdsProviderKind::Rustdds);
    let mut peer = provider.join(DOMAIN + 3, &shipped_qos_path()).unwrap();
    peer.create_topic("demo").unwrap();

    // Establishes SEDP match and confirms a conforming sample still passes
    // under reject mode.
    let conforming = rx_sample(Bytes::from_static(br#"{"Ping":{"n":7}}"#));
    write_until_matched(&mut *peer, "demo", &conforming, &mut rx).await;

    // `nope` is not a field Ping declares.
    let bad = rx_sample(Bytes::from_static(br#"{"Ping":{"nope":1}}"#));
    peer.write("demo", bad).unwrap();
    assert!(
        !any_payload_contains(&mut rx, Duration::from_millis(800), "nope").await,
        "a sample that does not follow the schema must not reach the engine"
    );

    // The adapter itself must still be alive and processing after dropping it.
    let after = write_until_matched(&mut *peer, "demo", &conforming, &mut rx).await;
    assert_eq!(after.payload.as_ref(), br#"{"Ping":{"n":7}}"#);

    shutdown.cancel();
}
