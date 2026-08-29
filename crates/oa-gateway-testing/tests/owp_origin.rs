//! The `owp.allowed_origins` handshake check.
//!
//! Empty (the default) accepts any `Origin`. A non-empty list refuses a
//! handshake whose `Origin` is not one of its entries verbatim, a missing
//! `Origin` included, with `403`.

use std::sync::Arc;
use std::time::Duration;

use oa_gateway_core::Engine;
use oa_gateway_testing::owp::{handshake, start_owp_with, OwpWs};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Error;

const APP: &str = "https://app.example";
const EVIL: &str = "https://evil.example";

/// Opens a WebSocket to `url` with the `owp` subprotocol and, when given,
/// an `Origin` header.
async fn connect_with_origin(url: &str, origin: Option<&str>) -> Result<OwpWs, Error> {
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "owp".parse().unwrap());
    if let Some(value) = origin {
        req.headers_mut().insert("Origin", value.parse().unwrap());
    }
    let (ws, _) = timeout(
        Duration::from_secs(2),
        tokio_tungstenite::connect_async(req),
    )
    .await
    .expect("connect timed out")?;
    Ok(ws)
}

fn assert_status(err: &Error, want: StatusCode) {
    match err {
        Error::Http(response) => assert_eq!(response.status(), want, "{err:?}"),
        other => panic!("expected an HTTP {want} rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn no_allowlist_accepts_any_origin() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |_| {}).await;

    let mut ws = connect_with_origin(&url, Some(EVIL))
        .await
        .expect("an empty allowlist should accept any origin");
    handshake(&mut ws).await;

    shutdown.cancel();
}

#[tokio::test]
async fn allowlist_refuses_a_non_matching_or_missing_origin() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| {
        config.allowed_origins = vec![APP.to_owned()];
    })
    .await;

    assert_status(
        &connect_with_origin(&url, Some(EVIL)).await.unwrap_err(),
        StatusCode::FORBIDDEN,
    );
    assert_status(
        &connect_with_origin(&url, None).await.unwrap_err(),
        StatusCode::FORBIDDEN,
    );

    shutdown.cancel();
}

#[tokio::test]
async fn allowlist_accepts_a_listed_origin() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| {
        config.allowed_origins = vec!["https://other.example".to_owned(), APP.to_owned()];
    })
    .await;

    let mut ws = connect_with_origin(&url, Some(APP))
        .await
        .expect("a listed origin should connect");
    handshake(&mut ws).await;

    shutdown.cancel();
}
