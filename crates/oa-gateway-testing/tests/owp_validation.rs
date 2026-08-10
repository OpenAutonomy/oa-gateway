//! What the two enforcement modes do to a payload the schema does not permit.
//!
//! `{"Ping":{"nope":1}}` converts without complaint — an element the type does
//! not declare is carried as a string — which is exactly the case validation
//! exists to notice.

use std::sync::Arc;
use std::time::Duration;

use oa_gateway_core::{Engine, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_owp::{parse_server, ServerOp};
use oa_gateway_testing::owp::{connect, handshake, recv_text, send_text, start_owp_with};
use oa_gateway_uci::ValidateMode;
use tokio::time::timeout;

const UNDECLARED: &str = r#"PUB demo {"Ping":{"nope":1}}"#;

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
