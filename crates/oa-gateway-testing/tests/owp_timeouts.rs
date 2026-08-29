//! INIT and idle timeouts on the OWP edge.
//!
//! Both are set to a fraction of a second here so the tests are quick; the
//! shipped defaults are 30 s (`init_timeout`) and 600 s (`idle_timeout`).

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use oa_gateway_core::Engine;
use oa_gateway_owp::{parse_server, ServerOp};
use oa_gateway_testing::owp::{connect, handshake, recv_text, send_text, start_owp_with};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Reads until the socket closes (a `Close` frame, an error, or end of
/// stream), or times out waiting. `true` means it closed.
async fn wait_for_close(ws: &mut oa_gateway_testing::owp::OwpWs) -> bool {
    timeout(Duration::from_secs(2), async {
        loop {
            match ws.next().await {
                None | Some(Err(_) | Ok(Message::Close(_))) => return true,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .unwrap_or(false)
}

/// A socket that finishes the WebSocket handshake but never sends INIT is
/// hung up on once `init_timeout` elapses.
#[tokio::test]
async fn init_timeout_closes_a_connection_that_never_inits() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| {
        config.init_timeout = Some(Duration::from_millis(200));
        config.idle_timeout = None;
        config.max_connections = 1;
    })
    .await;

    let mut ws = connect(&url).await;
    assert!(
        wait_for_close(&mut ws).await,
        "a connection that never sends INIT should be closed"
    );
    drop(ws);

    // The connection slot is released when the timed-out session task ends,
    // so a fresh client can connect and complete INIT.
    let mut next = None;
    for _ in 0..40 {
        if let Ok(mut req) = url.as_str().into_client_request() {
            req.headers_mut()
                .insert("Sec-WebSocket-Protocol", "owp".parse().unwrap());
            if let Ok((ws, _)) = tokio_tungstenite::connect_async(req).await {
                next = Some(ws);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut next = next.expect("the connection slot was never released");
    handshake(&mut next).await;

    shutdown.cancel();
}

/// An active session that goes quiet in both directions is closed once
/// `idle_timeout` elapses.
#[tokio::test]
async fn idle_timeout_closes_an_inactive_session() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| {
        config.init_timeout = None;
        config.idle_timeout = Some(Duration::from_millis(200));
    })
    .await;

    let mut ws = connect(&url).await;
    handshake(&mut ws).await;
    assert!(
        wait_for_close(&mut ws).await,
        "an idle session should be closed once idle_timeout passes"
    );

    shutdown.cancel();
}

/// A session that keeps sending frames is never closed for being idle,
/// however long it runs past `idle_timeout`.
#[tokio::test]
async fn an_active_session_outlasts_the_idle_timeout() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| {
        config.init_timeout = None;
        config.idle_timeout = Some(Duration::from_millis(250));
    })
    .await;

    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    // Well under idle_timeout each time, for several times its length in total.
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(120)).await;
        send_text(&mut ws, r#"PUB demo {"Ping":{"n":1}}"#).await;
        match parse_server(&recv_text(&mut ws).await).unwrap() {
            ServerOp::Ok => {}
            other => panic!("an active session should still answer, got {other}"),
        }
    }

    shutdown.cancel();
}
