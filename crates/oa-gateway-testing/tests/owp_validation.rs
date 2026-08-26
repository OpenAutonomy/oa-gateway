//! What the two enforcement modes do to a payload the schema does not permit.
//!
//! `{"Ping":{"nope":1}}` converts without complaint — an element the type does
//! not declare is carried as a string — which is exactly the case validation
//! exists to notice.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use oa_gateway_agra::RX_ELEMENT;
use oa_gateway_core::{ContentType, Engine, Envelope, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_owp::{parse_server, ServerOp};
use oa_gateway_testing::owp::{
    connect, handshake, recv_text, send_text, start_owp_with, start_owp_with_schema,
};
use oa_gateway_uci::{el_opt, Facets, Schema, ValidateMode};
use tokio::time::timeout;

const UNDECLARED: &str = r#"PUB demo {"Ping":{"nope":1}}"#;

/// The A-GRA wrapper's `EncodedPayload` is hex of *some* inner document, and
/// `xml_baseline` re-encodes the inner from XML to JSON as part of the outer
/// conversion (`transcode_wrapper_inner`) — so its byte length, and therefore
/// its hex length, changes even when nothing about the message is wrong.
/// A `length` facet on the wrapper's own `EncodedPayload` lets that be schema-
/// valid on the bus and schema-invalid once converted, without needing the
/// converter to have any actual bug: JSON is shorter than XML for the same
/// content, on the nose here.
const INNER_XML: &str = "<Inner><n>1</n></Inner>";

fn wrapper_schema() -> Schema {
    let mut s = Schema::new();
    s.complex("InnerType", vec![el_opt("n", "xs:int")])
        .element("Inner", "InnerType");
    s.simple_with(
        "EncodedPayloadType",
        "xs:hexBinary",
        Facets {
            length: Some(INNER_XML.len()),
            ..Facets::default()
        },
    )
    .complex(
        "RxMessageDataType",
        vec![el_opt("EncodedPayload", "EncodedPayloadType")],
    )
    .complex(
        "MaWrapperType",
        vec![el_opt("MessageData", "RxMessageDataType")],
    )
    .element(RX_ELEMENT, "MaWrapperType");
    s
}

fn hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// A wrapper whose `EncodedPayload` is exactly [`INNER_XML`]'s length in
/// octets — valid against [`wrapper_schema`] as it sits on the bus.
fn wrapper_xml() -> String {
    let hex = hex_upper(INNER_XML.as_bytes());
    format!(
        "<{RX_ELEMENT}><MessageData><EncodedPayload>{hex}</EncodedPayload></MessageData></{RX_ELEMENT}>"
    )
}

#[tokio::test]
async fn warn_carries_the_message_and_says_nothing_to_the_client() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-warn");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let (url, shutdown) =
        start_owp_with(engine, |config| config.validate = ValidateMode::Warn).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    send_text(&mut ws, UNDECLARED).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("warn must not refuse the publish, got {other}"),
    }

    // The operator is warned; the subscriber still receives it. Reporting a
    // violation is not the same as deciding to drop traffic.
    let got = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(got.payload.as_ref(), br#"{"Ping":{"nope":1}}"#);

    shutdown.cancel();
}

#[tokio::test]
async fn reject_refuses_the_publish_and_names_the_violation() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-reject");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let (url, shutdown) =
        start_owp_with(engine, |config| config.validate = ValidateMode::Reject).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    send_text(&mut ws, UNDECLARED).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Err { details, .. } => {
            let details = details.unwrap_or_default();
            assert!(
                details.contains("'nope' is not declared"),
                "the client should be told what is wrong: {details}"
            );
        }
        other => panic!("reject must refuse the publish, got {other}"),
    }

    // Nothing reached the bus, so a rejected publish leaves no trace downstream.
    assert!(
        timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_err(),
        "a rejected publish must not be delivered"
    );

    // A payload that does follow the schema still goes through.
    send_text(&mut ws, r#"PUB demo {"Ping":{"n":7}}"#).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK for a conforming payload, got {other}"),
    }
    let got = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(got.payload.as_ref(), br#"{"Ping":{"n":7}}"#);

    shutdown.cancel();
}

#[tokio::test]
async fn off_does_not_look_at_the_payload() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-off");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let (url, shutdown) =
        start_owp_with(engine, |config| config.validate = ValidateMode::Off).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    send_text(&mut ws, UNDECLARED).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }
    assert!(timeout(Duration::from_secs(2), rx.recv()).await.is_ok());

    shutdown.cancel();
}

/// [`wrapper_xml`] is valid against [`wrapper_schema`] as it sits on the bus
/// — the pre-conversion check has nothing to say about it. `xml_baseline`
/// re-encodes the inner document as JSON, which is shorter, so the *converted*
/// payload is what the length facet on `EncodedPayload` refuses. `validate`
/// governs that the same way it governs a producer-side violation.
#[tokio::test]
async fn reject_refuses_a_delivery_whose_conversion_produced_an_invalid_payload() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-conv-reject");

    let (url, shutdown) = start_owp_with_schema(engine, wrapper_schema(), |config| {
        config.xml_baseline = true;
        config.validate = ValidateMode::Reject;
    })
    .await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(&mut ws, &format!("SUB sub-1 {RX_ELEMENT} wrapper")).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }

    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("wrapper", RX_ELEMENT),
                Bytes::from(wrapper_xml()),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;

    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Err { details, .. } => {
            let details = details.unwrap_or_default();
            assert!(
                details.contains("converted to a payload that does not follow the UCI schema"),
                "the client should be told this is a conversion-side violation, not a \
                 producer one: {details}"
            );
        }
        other => panic!("expected the converted payload to be refused, got {other}"),
    }

    shutdown.cancel();
}

/// Same conversion-side violation as above, but `warn` carries the message
/// anyway — matching how a producer-side violation is treated in `warn` mode.
#[tokio::test]
async fn warn_still_forwards_a_delivery_whose_conversion_produced_an_invalid_payload() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-conv-warn");

    let (url, shutdown) = start_owp_with_schema(engine, wrapper_schema(), |config| {
        config.xml_baseline = true;
        config.validate = ValidateMode::Warn;
    })
    .await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    send_text(&mut ws, &format!("SUB sub-1 {RX_ELEMENT} wrapper")).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }

    loopback
        .publish(
            Envelope::new(
                RouteKey::typed("wrapper", RX_ELEMENT),
                Bytes::from(wrapper_xml()),
            )
            .with_content_type(ContentType::xml()),
        )
        .await;

    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Msg { sid, payload } => {
            assert_eq!(sid, "sub-1");
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
            let encoded = v
                .pointer(&format!("/{RX_ELEMENT}/MessageData/EncodedPayload"))
                .and_then(|v| v.as_str())
                .expect("EncodedPayload should still be present");
            assert_ne!(
                encoded.len(),
                INNER_XML.len() * 2,
                "the re-encoded inner is JSON, not XML, so its hex length must have changed"
            );
        }
        other => panic!("warn must still forward the delivery, got {other}"),
    }

    shutdown.cancel();
}
