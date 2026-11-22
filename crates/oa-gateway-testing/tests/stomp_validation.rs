//! STOMP checks both directions against the libxml2-backed validator:
//! inbound MESSAGE frames from the broker, and outbound SEND frames to it.
//!
//! `{"Ping":{"nope":1}}` converts without complaint — an element the type
//! does not declare is carried as a string — which is exactly the case
//! validation exists to notice. Mirrors `dds_bridge.rs`'s one validation
//! test in scope: `reject` proving the check actually blocks something and
//! the adapter keeps working afterward, not every mode combination.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_testing::stomp::{start_mini_broker, start_stomp_adapter_with, StompPeer};
use oa_gateway_testing::util::recv_envelope;
use oa_gateway_uci::ValidateMode;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// As `dds_bridge.rs`'s helper of the same shape: polls `rx` for up to
/// `window`, true the moment a payload containing `needle` arrives, false
/// if the window runs out first.
async fn any_payload_contains(
    rx: &mut mpsc::Receiver<Envelope>,
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

#[tokio::test]
async fn reject_drops_an_inbound_message_that_does_not_follow_the_schema() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-stomp-reject-in");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let shutdown = start_stomp_adapter_with(
        engine,
        "stomp-reject-in",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
        |config| config.validate = ValidateMode::Reject,
    )
    .await;

    let mut peer = StompPeer::connect(broker.addr).await;

    // A conforming send still crosses under reject mode.
    peer.send(
        "/topic/demo",
        br#"{"Ping":{"n":7}}"#,
        &[("content-type", "application/json")],
    )
    .await;
    let first = recv_envelope(&mut rx).await;
    assert_eq!(first.payload.as_ref(), br#"{"Ping":{"n":7}}"#);

    // `nope` is not a field Ping declares.
    peer.send(
        "/topic/demo",
        br#"{"Ping":{"nope":1}}"#,
        &[("content-type", "application/json")],
    )
    .await;
    assert!(
        !any_payload_contains(&mut rx, Duration::from_millis(800), "nope").await,
        "a message that does not follow the schema must not reach the engine"
    );

    // The adapter itself must still be alive and processing after dropping it.
    peer.send(
        "/topic/demo",
        br#"{"Ping":{"n":9}}"#,
        &[("content-type", "application/json")],
    )
    .await;
    let after = recv_envelope(&mut rx).await;
    assert_eq!(after.payload.as_ref(), br#"{"Ping":{"n":9}}"#);

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn reject_drops_an_outbound_message_that_does_not_follow_the_schema() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-stomp-reject-out");

    let shutdown = start_stomp_adapter_with(
        engine,
        "stomp-reject-out",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
        |config| config.validate = ValidateMode::Reject,
    )
    .await;

    let mut peer = StompPeer::connect(broker.addr).await;
    peer.subscribe("s1", "/topic/demo").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // A conforming publish still crosses under reject mode.
    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from_static(br#"{"Ping":{"n":7}}"#),
            )
            .with_content_type(ContentType::json()),
        )
        .await;
    let first = peer.recv().await;
    assert_eq!(first.command, "MESSAGE");
    assert_eq!(first.body, br#"{"Ping":{"n":7}}"#);

    // `nope` is not a field Ping declares; nothing should reach the broker.
    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from_static(br#"{"Ping":{"nope":1}}"#),
            )
            .with_content_type(ContentType::json()),
        )
        .await;
    assert!(
        peer.try_recv(Duration::from_millis(800)).await.is_none(),
        "an outbound send that does not follow the schema must not reach the broker"
    );

    // The adapter itself must still be alive and processing after dropping it.
    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from_static(br#"{"Ping":{"n":9}}"#),
            )
            .with_content_type(ContentType::json()),
        )
        .await;
    let after = peer.recv().await;
    assert_eq!(after.body, br#"{"Ping":{"n":9}}"#);

    shutdown.cancel();
    broker.shutdown();
}
