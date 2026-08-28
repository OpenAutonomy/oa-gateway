//! Mutual TLS on the STOMP client: an adapter presenting a certificate
//! issued by the broker's trusted CA connects, one presenting none or one
//! from a different CA is refused before it ever reaches CONNECTED.

use std::sync::Arc;
use std::time::Duration;

use oa_gateway_core::Engine;
use oa_gateway_stomp::{StompAdapter, StompConfig};
use oa_gateway_testing::stomp::{start_mini_broker_mtls, start_stomp_adapter_with};
use oa_gateway_testing::tls::{client_tls_with_client_cert, issue, self_signed, test_ca};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn an_adapter_with_a_certificate_from_the_trusted_ca_connects() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let client_ca = test_ca();
    let broker = start_mini_broker_mtls(&certs, &client_ca).await;

    let client_certs = issue(&client_ca, &["stomp-adapter"]);
    let tls = client_tls_with_client_cert(&certs, "127.0.0.1", &client_certs);

    // start_stomp_adapter_with panics on a ready timeout, so reaching this
    // line at all proves the mTLS handshake and CONNECT/CONNECTED succeeded.
    let shutdown = start_stomp_adapter_with(
        engine,
        "stomp-mtls-test",
        broker.addr,
        vec!["demo".into()],
        Duration::ZERO,
        |config| config.tls = Some(tls),
    )
    .await;

    shutdown.cancel();
    broker.shutdown();
}

#[tokio::test]
async fn an_adapter_with_no_client_certificate_is_refused() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let client_ca = test_ca();
    let broker = start_mini_broker_mtls(&certs, &client_ca).await;

    // Trusts the broker fine, but presents no client certificate at all.
    let tls = oa_gateway_testing::tls::client_tls(&certs, "127.0.0.1");
    let adapter = Arc::new(StompAdapter::new(
        "stomp-mtls-no-cert",
        StompConfig {
            broker: broker.addr,
            reconnect: false,
            tls: Some(tls),
            ..StompConfig::default()
        },
    ));

    // TLS 1.3's client-cert rejection can surface on the next read/write
    // rather than from the handshake future itself, so this only asserts
    // that connecting fails, not which internal step reports it.
    tokio::time::timeout(
        Duration::from_secs(2),
        adapter.serve(engine, CancellationToken::new()),
    )
    .await
    .expect("adapter should fail rather than hang")
    .expect_err("a broker requiring a client certificate should refuse a client with none");

    broker.shutdown();
}

#[tokio::test]
async fn an_adapter_with_a_certificate_from_an_untrusted_ca_is_refused() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let client_ca = test_ca();
    let broker = start_mini_broker_mtls(&certs, &client_ca).await;

    let other_ca = test_ca();
    let untrusted_client_certs = issue(&other_ca, &["stomp-adapter"]);
    let tls = client_tls_with_client_cert(&certs, "127.0.0.1", &untrusted_client_certs);
    let adapter = Arc::new(StompAdapter::new(
        "stomp-mtls-untrusted-cert",
        StompConfig {
            broker: broker.addr,
            reconnect: false,
            tls: Some(tls),
            ..StompConfig::default()
        },
    ));

    tokio::time::timeout(
        Duration::from_secs(2),
        adapter.serve(engine, CancellationToken::new()),
    )
    .await
    .expect("adapter should fail rather than hang")
    .expect_err("a client certificate from an untrusted CA should be refused");

    broker.shutdown();
}
