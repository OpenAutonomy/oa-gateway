//! TLS on the OWP listener: the protocol works unchanged over `wss://`, and
//! a bad handshake — wrong protocol, wrong trust — does not take the accept
//! loop down for the next, well-behaved client.

use std::sync::Arc;

use oa_gateway_core::{Engine, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_owp::{parse_server, ServerOp};
use oa_gateway_testing::owp::{connect_tls, handshake, recv_text, send_text, start_owp_tls_with};
use oa_gateway_testing::tls::{client_tls, self_signed, untrusted_client_tls};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// A plaintext WebSocket connect that returns an error rather than
/// panicking, so a test can assert on a listener that only accepts TLS.
async fn try_connect_plain(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut req = url.into_client_request()?;
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "owp".parse()?);
    let (ws, _) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio_tungstenite::connect_async(req),
    )
    .await??;
    drop(ws);
    Ok(())
}

#[tokio::test]
async fn tls_round_trip() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-tls");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let certs = self_signed(&["127.0.0.1"]);
    let (url, shutdown) = start_owp_tls_with(engine, &certs, |_| {}).await;
    assert!(url.starts_with("wss://"), "{url}");

    let tls = client_tls(&certs, "127.0.0.1");
    let mut ws = connect_tls(&url, tls).await.expect("tls handshake");
    handshake(&mut ws).await;

    send_text(&mut ws, r#"PUB demo {"Ping":{"n":7}}"#).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }
    let got = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(got.payload.as_ref(), br#"{"Ping":{"n":7}}"#);

    shutdown.cancel();
}

#[tokio::test]
async fn a_plaintext_client_cannot_talk_to_a_tls_listener() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let (url, shutdown) = start_owp_tls_with(engine, &certs, |_| {}).await;
    let ws_url = url.replacen("wss://", "ws://", 1);

    // A plaintext client speaks the WebSocket handshake straight at a TLS
    // ClientHello reader, which never recognizes it as one — this should
    // fail one way or another (a rejected handshake or a timeout), not
    // succeed.
    assert!(
        try_connect_plain(&ws_url).await.is_err(),
        "a plaintext client should not be able to talk to a TLS listener"
    );

    // The accept loop is still serving TLS clients after the bad one.
    let tls = client_tls(&certs, "127.0.0.1");
    let mut ws = connect_tls(&url, tls)
        .await
        .expect("listener should still accept a well-formed TLS client");
    handshake(&mut ws).await;

    shutdown.cancel();
}

#[tokio::test]
async fn a_client_that_does_not_trust_the_certificate_is_refused() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let (url, shutdown) = start_owp_tls_with(engine, &certs, |_| {}).await;

    let untrusted = untrusted_client_tls("127.0.0.1");
    assert!(
        connect_tls(&url, untrusted).await.is_err(),
        "a client trusting a different authority should be refused"
    );

    // The accept loop is still serving trusting clients after the bad one.
    let tls = client_tls(&certs, "127.0.0.1");
    let mut ws = connect_tls(&url, tls)
        .await
        .expect("listener should still accept a trusting client");
    handshake(&mut ws).await;

    shutdown.cancel();
}
