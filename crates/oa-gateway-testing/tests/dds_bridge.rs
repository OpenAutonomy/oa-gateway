//! DDS adapter talks only to the engine; a second rustdds participant is the peer.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_agra::{WrapperKind, WrapperMeta};
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_dds::{provider_for, DdsProviderKind, DdsSample};
use oa_gateway_loopback::Loopback;
use oa_gateway_testing::dds::{shipped_qos_path, start_dds_adapter};
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
