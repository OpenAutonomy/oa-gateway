//! OWP and STOMP only meet at the engine.

use std::sync::Arc;
use std::time::Duration;

use mpg_core::Engine;
use mpg_owp::{parse_server, ServerOp};
use mpg_stomp::HDR_TYPE_HINT;
use mpg_testing::fixtures::POSITION_REPORT_XML_BYTES;
use mpg_testing::owp::{connect, handshake, recv_text, send_text, start_owp};
use mpg_testing::stomp::{start_mini_broker, start_stomp_adapter, StompPeer};

#[tokio::test]
async fn owp_pub_reaches_stomp() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let stomp_sd = start_stomp_adapter(
        engine.clone(),
        "stomp-test",
        broker.addr,
        vec!["PositionReport".into()],
        Duration::ZERO,
    )
    .await;
    let (url, owp_sd) = start_owp(engine, true).await;

    let mut peer = StompPeer::connect(broker.addr).await;
    peer.subscribe("s1", "/topic/PositionReport").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(
        &mut ws,
        r#"PUB PositionReport {"PositionReport":{"MessageData":{"n":1}}}"#,
    )
    .await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }

    let msg = peer.recv().await;
    assert_eq!(msg.header("destination"), Some("/topic/PositionReport"));
    assert_eq!(msg.header(HDR_TYPE_HINT), Some("PositionReport"));
    let body = String::from_utf8_lossy(&msg.body);
    assert!(body.contains("<PositionReport"), "{body}");
    assert!(body.contains("<n>1</n>"), "{body}");

    owp_sd.cancel();
    stomp_sd.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn stomp_send_reaches_owp_sub() {
    let broker = start_mini_broker().await;
    let engine = Arc::new(Engine::new());
    let stomp_sd = start_stomp_adapter(
        engine.clone(),
        "stomp-test",
        broker.addr,
        vec!["PositionReport".into()],
        Duration::ZERO,
    )
    .await;
    let (url, owp_sd) = start_owp(engine, true).await;

    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(&mut ws, "SUB sub-1 PositionReport PositionReport").await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }

    let mut peer = StompPeer::connect(broker.addr).await;
    peer.send(
        "/topic/PositionReport",
        POSITION_REPORT_XML_BYTES,
        &[("content-type", "application/xml")],
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

    owp_sd.cancel();
    stomp_sd.cancel();
    broker.shutdown();
}
