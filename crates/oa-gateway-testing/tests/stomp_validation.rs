//! STOMP checks both directions against the libxml2-backed validator:
//! inbound MESSAGE frames from the broker, and outbound SEND frames to it.
//!
//! `<Ping><nope>1</nope></Ping>` is well-formed XML but `nope` is not a
//! field `PingType` declares — exactly the case validation exists to
//! notice. Mirrors `dds_bridge.rs`'s one validation test in scope:
//! `reject` proving the check actually blocks something and the adapter
//! keeps working afterward, not every mode combination.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_adapter::ValidateMode;
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_testing::stomp::{start_mini_broker, start_stomp_adapter_with, StompPeer};
use oa_gateway_testing::util::recv_envelope;
use tokio::sync::mpsc;
use tokio::time::timeout;

const NS: &str = "https://www.vdl.afrl.af.mil/programs/oam";

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
    let conforming = format!(r#"<Ping xmlns="{NS}"><n>7</n></Ping>"#);
    peer.send("/topic/demo", conforming.as_bytes(), &[]).await;
    let first = recv_envelope(&mut rx).await;
    assert_eq!(first.payload.as_ref(), conforming.as_bytes());

    // `nope` is not a field Ping declares.
    let bad = format!(r#"<Ping xmlns="{NS}"><nope>1</nope></Ping>"#);
    peer.send("/topic/demo", bad.as_bytes(), &[]).await;
    assert!(
        !any_payload_contains(&mut rx, Duration::from_millis(800), "nope").await,
        "a message that does not follow the schema must not reach the engine"
    );

    // The adapter itself must still be alive and processing after dropping it.
    let again = format!(r#"<Ping xmlns="{NS}"><n>9</n></Ping>"#);
    peer.send("/topic/demo", again.as_bytes(), &[]).await;
    let after = recv_envelope(&mut rx).await;
    assert_eq!(after.payload.as_ref(), again.as_bytes());

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
    let conforming = format!(r#"<Ping xmlns="{NS}"><n>7</n></Ping>"#);
    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from(conforming.clone().into_bytes()),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;
    let first = peer.recv().await;
    assert_eq!(first.command, "MESSAGE");
    assert_eq!(first.body, conforming.as_bytes());

    // `nope` is not a field Ping declares; nothing should reach the broker.
    let bad = format!(r#"<Ping xmlns="{NS}"><nope>1</nope></Ping>"#);
    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from(bad.into_bytes()),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;
    assert!(
        peer.try_recv(Duration::from_millis(800)).await.is_none(),
        "an outbound send that does not follow the schema must not reach the broker"
    );

    // The adapter itself must still be alive and processing after dropping it.
    let again = format!(r#"<Ping xmlns="{NS}"><n>9</n></Ping>"#);
    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from(again.clone().into_bytes()),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;
    let after = peer.recv().await;
    assert_eq!(after.body, again.as_bytes());

    shutdown.cancel();
    broker.shutdown();
}
