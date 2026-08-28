//! Mutual TLS on the OWP listener: a client presenting a certificate issued
//! by the trusted CA connects, one presenting none or one from a different
//! CA is refused — and, as with encryption-only TLS, a bad handshake does
//! not take the accept loop down for the next, well-behaved client.

use std::sync::Arc;

use oa_gateway_core::Engine;
use oa_gateway_testing::owp::{connect_tls, handshake, start_owp_mtls_with};
use oa_gateway_testing::tls::{
    client_tls, client_tls_with_client_cert, issue, self_signed, test_ca,
};

#[tokio::test]
async fn a_client_with_a_certificate_from_the_trusted_ca_connects() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let client_ca = test_ca();
    let (url, shutdown) = start_owp_mtls_with(engine, &certs, &client_ca, |_| {}).await;

    let client_certs = issue(&client_ca, &["client-1"]);
    let tls = client_tls_with_client_cert(&certs, "127.0.0.1", &client_certs);
    let mut ws = connect_tls(&url, tls).await.expect("mtls handshake");
    handshake(&mut ws).await;

    shutdown.cancel();
}

#[tokio::test]
async fn a_client_with_no_certificate_is_refused() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let client_ca = test_ca();
    let (url, shutdown) = start_owp_mtls_with(engine, &certs, &client_ca, |_| {}).await;

    // Trusts the server fine, but presents no client certificate at all.
    let no_client_cert = client_tls(&certs, "127.0.0.1");
    assert!(
        connect_tls(&url, no_client_cert).await.is_err(),
        "a client with no certificate should be refused once client_ca is configured"
    );

    // The accept loop is still serving clients with a valid certificate.
    let client_certs = issue(&client_ca, &["client-1"]);
    let tls = client_tls_with_client_cert(&certs, "127.0.0.1", &client_certs);
    let mut ws = connect_tls(&url, tls)
        .await
        .expect("listener should still accept a client with a valid certificate");
    handshake(&mut ws).await;

    shutdown.cancel();
}

#[tokio::test]
async fn a_client_with_a_certificate_from_an_untrusted_ca_is_refused() {
    let engine = Arc::new(Engine::new());
    let certs = self_signed(&["127.0.0.1"]);
    let client_ca = test_ca();
    let (url, shutdown) = start_owp_mtls_with(engine, &certs, &client_ca, |_| {}).await;

    let other_ca = test_ca();
    let untrusted_client_certs = issue(&other_ca, &["client-1"]);
    let tls = client_tls_with_client_cert(&certs, "127.0.0.1", &untrusted_client_certs);
    assert!(
        connect_tls(&url, tls).await.is_err(),
        "a client certificate from an untrusted CA should be refused"
    );

    // The accept loop is still serving clients with a valid certificate.
    let client_certs = issue(&client_ca, &["client-1"]);
    let tls = client_tls_with_client_cert(&certs, "127.0.0.1", &client_certs);
    let mut ws = connect_tls(&url, tls)
        .await
        .expect("listener should still accept a client with a valid certificate");
    handshake(&mut ws).await;

    shutdown.cancel();
}
