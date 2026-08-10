//! STOMP adapter talks only to the engine; the mini broker stands in for ActiveMQ.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_agra::{wrap, WrapRequest, WrapShell, WrapperKind, RX_ELEMENT};
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_stomp::{HDR_ORIGIN, HDR_TYPE_HINT};
use oa_gateway_testing::stomp::{start_mini_broker, start_stomp_adapter, StompPeer};
use oa_gateway_testing::util::recv_envelope;
use serde_json::json;
use tokio::time::timeout;

#[tokio::test]
async fn stomp_send_reaches_loopback() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-b");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let shutdown = start_stomp_adapter(
        engine,
        "stomp-test",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
    )
    .await;

    let mut peer = StompPeer::connect(broker.addr).await;
    peer.send(
        "/topic/demo",
        br#"{"Ping":{"n":7}}"#,
        &[("content-type", "application/json")],
    )
    .await;

    let got = recv_envelope(&mut rx).await;
    assert_eq!(got.route.topic, "demo");
    assert_eq!(got.route.type_hint.as_deref(), Some("Ping"));
    assert_eq!(got.content_type, ContentType::json());
    assert_eq!(got.payload.as_ref(), br#"{"Ping":{"n":7}}"#);
    assert_eq!(
        got.headers.get(HDR_ORIGIN).map(String::as_str),
        Some("stomp-test")
    );

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn loopback_pub_reaches_stomp_peer() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-a");

    let shutdown = start_stomp_adapter(
        engine,
        "stomp-test",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
    )
    .await;

    let mut peer = StompPeer::connect(broker.addr).await;
    peer.subscribe("s1", "/topic/demo").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from_static(br#"{"Ping":{"from":"loop"}}"#),
            )
            .with_content_type(ContentType::json()),
        )
        .await;

    let msg = peer.recv().await;
    assert_eq!(msg.command, "MESSAGE");
    assert_eq!(msg.header("destination"), Some("/topic/demo"));
    assert_eq!(msg.header(HDR_TYPE_HINT), Some("Ping"));
    assert_eq!(msg.body, br#"{"Ping":{"from":"loop"}}"#);

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn inbound_does_not_echo_back_to_broker() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let shutdown = start_stomp_adapter(
        engine,
        "stomp-test",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
    )
    .await;

    let mut watcher = StompPeer::connect(broker.addr).await;
    watcher.subscribe("watch", "/topic/demo").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut sender = StompPeer::connect(broker.addr).await;
    sender
        .send(
            "/topic/demo",
            b"<Ping/>",
            &[("content-type", "application/xml")],
        )
        .await;

    let first = watcher.recv().await;
    assert_eq!(first.body, b"<Ping/>");
    match timeout(Duration::from_millis(80), watcher.recv()).await {
        Err(_) => {}
        Ok(dup) => panic!("echoed duplicate {:?}", dup.headers),
    }

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn xml_root_becomes_type_hint() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-b");
    let mut rx = loopback
        .subscribe(RouteKey::typed("PositionReport", "PositionReport"))
        .await
        .unwrap();

    let shutdown = start_stomp_adapter(
        engine,
        "stomp-test",
        broker.addr,
        vec!["PositionReport".into()],
        Duration::ZERO,
    )
    .await;

    let mut peer = StompPeer::connect(broker.addr).await;
    peer.send(
        "/topic/PositionReport",
        b"<?xml version=\"1.0\"?><PositionReport><n>1</n></PositionReport>",
        &[("content-type", "application/xml")],
    )
    .await;

    let got = recv_envelope(&mut rx).await;
    assert_eq!(got.content_type, ContentType::xml());
    assert_eq!(got.route.type_hint.as_deref(), Some("PositionReport"));

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn rx_wrapper_fans_out_wrapper_and_inner() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-b");
    let mut inner_rx = loopback
        .subscribe(RouteKey::typed("offboard", "PositionReport"))
        .await
        .unwrap();
    let mut wrap_rx = loopback
        .subscribe(RouteKey::typed("offboard", RX_ELEMENT))
        .await
        .unwrap();

    let shutdown = start_stomp_adapter(
        engine,
        "stomp-test",
        broker.addr,
        vec!["offboard".into()],
        Duration::ZERO,
    )
    .await;

    let inner = br#"{"PositionReport":{"MessageData":{"n":1}}}"#;
    let wrapped = wrap(WrapRequest {
        topic: "offboard".into(),
        kind: WrapperKind::Rx,
        inner: Bytes::from_static(inner),
        message_type_enum: "POSITION_REPORT".into(),
        shell: WrapShell {
            security_information: json!({"Classification": "U"}),
            message_header: json!({
                "SystemID": {"UUID": "00000000-0000-4000-8000-000000000001"},
                "Timestamp": "2026-08-09T21:00:00Z",
                "SchemaVersion": "002.5.0",
                "Mode": "SIMULATION"
            }),
        },
        destination_routing: "TOPIC_AND_SPECIFIC_DESTINATION".into(),
        originator_uuid: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
        specific_destination_uuids: vec![],
        timestamp: "2026-08-09T21:00:00Z".into(),
        priority: 4,
        precedence: 1,
    })
    .unwrap();

    let mut peer = StompPeer::connect(broker.addr).await;
    peer.send("/topic/offboard", &wrapped.payload, &[]).await;

    let inner_got = recv_envelope(&mut inner_rx).await;
    assert_eq!(inner_got.payload.as_ref(), inner);
    let wrap_got = recv_envelope(&mut wrap_rx).await;
    assert_eq!(wrap_got.route.type_hint.as_deref(), Some(RX_ELEMENT));

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn shutdown_drops_engine_subs() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let shutdown = start_stomp_adapter(
        engine.clone(),
        "stomp-test",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
    )
    .await;
    assert_eq!(engine.subscription_count().await, 1);
    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(engine.subscription_count().await, 0);
    broker.shutdown();
}
