//! OWP and loopback only communicate through the engine.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_agra::{wrap, WrapRequest, WrapShell, WrapperKind, RX_ELEMENT};
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_owp::{parse_server, ServerOp};
use oa_gateway_testing::owp::{connect, handshake, recv_text, send_text, start_owp};
use tokio::time::timeout;

#[tokio::test]
async fn owp_pub_reaches_loopback() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-b");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let (url, shutdown) = start_owp(engine).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    send_text(
        &mut ws,
        r#"PUB demo <Ping xmlns="https://www.vdl.afrl.af.mil/programs/oam"><n>7</n></Ping>"#,
    )
    .await;
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
    assert_eq!(got.content_type, ContentType::xml());
    assert_eq!(
        got.payload.as_ref(),
        br#"<Ping xmlns="https://www.vdl.afrl.af.mil/programs/oam"><n>7</n></Ping>"#
    );
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

    let (url, shutdown) = start_owp(engine).await;
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
                Bytes::from_static(
                    br#"<Ping xmlns="https://www.vdl.afrl.af.mil/programs/oam"><from>loop</from></Ping>"#,
                ),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;

    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Msg { sid, payload } => {
            assert_eq!(sid, "sub-1");
            assert_eq!(
                payload,
                r#"<Ping xmlns="https://www.vdl.afrl.af.mil/programs/oam"><from>loop</from></Ping>"#
            );
        }
        other => panic!("expected MSG, got {other}"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn disconnect_unsubscribes() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-a");

    let (url, shutdown) = start_owp(engine.clone()).await;
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
            Bytes::from_static(b"<Ping/>"),
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

    let inner = br#"<PositionReport xmlns="https://www.vdl.afrl.af.mil/programs/oam"><MessageData><n>1</n></MessageData></PositionReport>"#;
    let wrapped = wrap(WrapRequest {
        topic: "offboard".into(),
        kind: WrapperKind::Rx,
        inner: Bytes::from_static(inner),
        message_type_enum: "POSITION_REPORT".into(),
        shell: WrapShell {
            security_information: "<Classification>U</Classification>".into(),
            message_header:
                "<SystemID><UUID>00000000-0000-4000-8000-000000000001</UUID></SystemID>\
                <Timestamp>2026-08-09T21:00:00Z</Timestamp>\
                <SchemaVersion>002.5.0</SchemaVersion><Mode>SIMULATION</Mode>"
                    .into(),
        },
        destination_routing: "TOPIC_AND_SPECIFIC_DESTINATION".into(),
        originator_uuid: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
        specific_destination_uuids: vec![],
        timestamp: "2026-08-09T21:00:00Z".into(),
        priority: 4,
        precedence: 1,
    });
    let wrap_text = String::from_utf8(wrapped.payload.to_vec()).unwrap();

    let (url, shutdown) = start_owp(engine).await;
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

/// An XML publish is named by its root element, not by the topic it arrived on.
///
/// The two differ deliberately here: a subscriber keyed on the message type is
/// the normal case, and naming the payload after the topic would route past it
/// while the publisher was still told +OK.
#[tokio::test]
async fn xml_publish_takes_its_type_from_the_document() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-xml");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "PositionReport"))
        .await
        .unwrap();

    let (url, shutdown) = start_owp(engine).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    let xml = r#"<PositionReport xmlns="https://www.vdl.afrl.af.mil/programs/oam"><MessageData/></PositionReport>"#;
    send_text(&mut ws, &format!("PUB demo {xml}")).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK after PUB, got {other}"),
    }

    let got = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(got.route.type_hint.as_deref(), Some("PositionReport"));
    assert_eq!(got.route.topic, "demo");
    assert_eq!(got.content_type, ContentType::xml());

    shutdown.cancel();
}

#[tokio::test]
async fn xml_with_no_element_is_refused_rather_than_named_after_the_topic() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp(engine).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    // Opens like XML and declares nothing, so there is no name to route by.
    send_text(&mut ws, r#"PUB demo <?xml version="1.0"?>"#).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Err { details, .. } => {
            assert!(
                details
                    .as_deref()
                    .unwrap_or_default()
                    .contains("no element"),
                "unhelpful details: {details:?}"
            );
        }
        other => panic!("expected ERR for a payload with no element, got {other}"),
    }

    shutdown.cancel();
}
