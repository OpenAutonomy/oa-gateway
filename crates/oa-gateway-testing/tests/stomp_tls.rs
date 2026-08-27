//! TLS on the STOMP client: the protocol works unchanged over a TLS broker
//! connection, an untrusted broker certificate fails the session rather than
//! being silently accepted, and a plaintext adapter cannot talk to a TLS
//! broker.

use std::sync::Arc;
use std::time::Duration;

use oa_gateway_core::{Engine, RouteKey};
use oa_gateway_loopback::Loopback;
use oa_gateway_stomp::{StompAdapter, StompConfig};
use oa_gateway_testing::stomp::{start_mini_broker_tls, start_stomp_adapter_with, StompPeer};
use oa_gateway_testing::tls::{client_tls, self_signed, untrusted_client_tls};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn tls_round_trip() {
    let engine = Arc::new(Engine::new());
    let loopback = Loopback::new(engine.clone(), "loop-stomp-tls");
    let mut rx = loopback
        .subscribe(RouteKey::typed("demo", "Ping"))
        .await
        .unwrap();

    let certs = self_signed(&["127.0.0.1"]);
    let broker = start_mini_broker_tls(&certs).await;
    let tls = client_tls(&certs, "127.0.0.1");

    let shutdown = start_stomp_adapter_with(
        engine.clone(),
        "stomp-tls-test",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
        |config| config.tls = Some(tls.clone()),
    )
    .await;

    // Broker -> adapter -> engine.
    let mut peer = StompPeer::connect_tls(broker.addr, &tls).await;
    peer.send("/topic/demo", br#"{"Ping":{"n":9}}"#, &[]).await;
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");
    assert_eq!(got.payload.as_ref(), br#"{"Ping":{"n":9}}"#);

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn an_untrusted_broker_certificate_fails_the_session() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let broker = start_mini_broker_tls(&certs).await;

    let untrusted = untrusted_client_tls("127.0.0.1");
    let adapter = Arc::new(StompAdapter::new(
        "stomp-tls-untrusted",
        StompConfig {
            broker: broker.addr,
            reconnect: false,
            tls: Some(untrusted),
            ..StompConfig::default()
        },
    ));

    let err = tokio::time::timeout(
        Duration::from_secs(2),
        adapter.serve(engine, CancellationToken::new()),
    )
    .await
    .expect("adapter should fail rather than hang")
    .expect_err("an untrusted certificate should fail the session");
    assert!(err.to_string().contains("tls handshake failed"), "{err}");

    broker.shutdown();
}

#[tokio::test]
async fn a_plaintext_adapter_cannot_talk_to_a_tls_broker() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let broker = start_mini_broker_tls(&certs).await;

    // No `tls` set: the adapter dials plaintext against a TLS-only broker.
    let adapter = Arc::new(StompAdapter::new(
        "stomp-tls-plaintext-client",
        StompConfig {
            broker: broker.addr,
            reconnect: false,
            connect_timeout: Duration::from_secs(1),
            ..StompConfig::default()
        },
    ));

    let result = tokio::time::timeout(
        Duration::from_secs(3),
        adapter.serve(engine, CancellationToken::new()),
    )
    .await
    .expect("adapter should fail within its own connect_timeout, not hang forever");
    assert!(
        result.is_err(),
        "a plaintext client should not be able to talk to a TLS-only broker"
    );

    broker.shutdown();
}
