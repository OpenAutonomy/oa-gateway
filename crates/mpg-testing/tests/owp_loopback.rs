//! OWP and loopback only communicate through the engine.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use mpg_agra::{wrap, WrapRequest, WrapShell, WrapperKind, RX_ELEMENT};
use mpg_core::{ContentType, Engine, Envelope, RouteKey};
use mpg_loopback::Loopback;
use mpg_owp::{parse_server, ServerOp};
use mpg_testing::fixtures::POSITION_REPORT_XML_BYTES;
use mpg_testing::owp::{connect, handshake, recv_text, send_text, start_owp};
use serde_json::json;
use tokio::time::timeout;

#[tokio::test]
async fn owp_pub_reaches_loopback() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-b");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let (url, shutdown) = start_owp(engine, false).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    send_text(&mut ws, r#"PUB demo {"Ping":{"n":7}}"#).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK after PUB, got {other}"),
    }

    let got = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(got.route.type_hint.as_deref(), Some("Ping"));
    assert_eq!(got.route.topic, "demo");
    assert_eq!(got.content_type, ContentType::json());
    assert_eq!(got.payload.as_ref(), br#"{"Ping":{"n":7}}"#);
    assert_eq!(
        got.headers.get("owp.service_id").map(String::as_str),
        Some("web-app")
    );

    shutdown.cancel();
}

#[tokio::test]
async fn loopback_pub_reaches_owp_sub() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-a");

    let (url, shutdown) = start_owp(engine, false).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    send_text(&mut ws, "SUB sub-1 Ping demo").await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK after SUB, got {other}"),
    }

    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("demo", "Ping"),
                Bytes::from_static(br#"{"Ping":{"from":"loop"}}"#),
            )
            .with_content_type(ContentType::json()),
        )
        .await;

    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Msg { sid, payload } => {
            assert_eq!(sid, "sub-1");
            assert_eq!(payload, r#"{"Ping":{"from":"loop"}}"#);
        }
        other => panic!("expected MSG, got {other}"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn disconnect_unsubscribes() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-a");

    let (url, shutdown) = start_owp(engine.clone(), false).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(&mut ws, "SUB sub-1 Ping demo").await;
    let _ = recv_text(&mut ws).await;
    assert_eq!(engine.subscription_count().await, 1);

    ws.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(engine.subscription_count().await, 0);

    loopback
        .publish(Envelope::new(
            RouteKey::typed("demo", "Ping"),
            Bytes::from_static(b"{}"),
        ))
        .await;

    shutdown.cancel();
}

#[tokio::test]
async fn owp_rx_wrapper_fans_out_wrapper_and_inner() {
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
    let wrap_text = String::from_utf8(wrapped.payload.to_vec()).unwrap();

    let (url, shutdown) = start_owp(engine, false).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(&mut ws, &format!("PUB offboard {wrap_text}")).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }

    let inner_got = timeout(Duration::from_secs(2), inner_rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(inner_got.payload.as_ref(), inner);
    assert_eq!(
        inner_got
            .headers
            .get("agra.originator_uuid")
            .map(String::as_str),
        Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
    );

    let wrap_got = timeout(Duration::from_secs(2), wrap_rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(wrap_got.route.type_hint.as_deref(), Some(RX_ELEMENT));

    shutdown.cancel();
}

#[tokio::test]
async fn owp_json_pub_becomes_xml_on_engine() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-b");
    let mut rx = loopback
        .subscribe(RouteKey::typed("PositionReport", "PositionReport"))
        .await
        .unwrap();

    let (url, shutdown) = start_owp(engine, true).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(
        &mut ws,
        r#"PUB PositionReport {"PositionReport":{"MessageData":{"n":7}}}"#,
    )
    .await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }

    let got = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(got.content_type, ContentType::xml());
    let body = std::str::from_utf8(&got.payload).unwrap();
    assert!(body.contains("<PositionReport"), "{body}");
    assert!(body.contains("<n>7</n>"), "{body}");

    shutdown.cancel();
}

#[tokio::test]
async fn engine_xml_becomes_owp_json() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-a");

    let (url, shutdown) = start_owp(engine, true).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(&mut ws, "SUB sub-1 PositionReport PositionReport").await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }

    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("PositionReport", "PositionReport"),
                Bytes::from_static(POSITION_REPORT_XML_BYTES),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;

    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Msg { sid, payload } => {
            assert_eq!(sid, "sub-1");
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(
                v.pointer("/PositionReport/MessageData/n"),
                Some(&serde_json::json!(1))
            );
        }
        other => panic!("expected MSG, got {other}"),
    }

    shutdown.cancel();
}
