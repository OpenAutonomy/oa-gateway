//! Resource limits on the OWP edge, exercised against a running adapter.
//!
//! Each limit is set far below its default so that reaching it costs a few
//! frames rather than megabytes. What matters is the behavior at the boundary:
//! the connection ends, or the client is told, rather than the gateway quietly
//! allocating whatever it was asked to.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use oa_gateway_core::Engine;
use oa_gateway_owp::{parse_server, OwpError, ServerOp};
use oa_gateway_testing::owp::{connect, handshake, recv_text, send_text, start_owp_with};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn a_frame_over_the_limit_ends_the_session() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| config.max_frame_size = 1024).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    // Well-formed, and far too large: the frame is refused for its size, not
    // for its content.
    let payload = "x".repeat(4096);
    let frame = format!(r#"PUB demo {{"Ping":{{"n":"{payload}"}}}}"#);
    // The send itself succeeds — the client has its own, larger limit — so the
    // rejection has to show up as the session ending.
    let _ = ws.send(Message::Text(frame.into())).await;

    let ended = timeout(Duration::from_secs(2), async {
        loop {
            match ws.next().await {
                None => return true,
                Some(Err(_)) => return true,
                Some(Ok(Message::Close(_))) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await
    .expect("adapter neither closed the session nor answered");
    assert!(ended, "oversized frame should end the session");

    shutdown.cancel();
}

#[tokio::test]
async fn a_frame_at_the_limit_still_works() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| config.max_frame_size = 1024).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    let prefix = r#"PUB demo {"Ping":{"n":""#;
    let suffix = r#""}}"#;
    let payload = "x".repeat(1024 - prefix.len() - suffix.len());
    let frame = format!("{prefix}{payload}{suffix}");
    assert_eq!(frame.len(), 1024);

    send_text(&mut ws, &frame).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK for a frame exactly at the limit, got {other}"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn subscriptions_past_the_limit_are_refused() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| config.max_subscriptions = 2).await;
    let mut ws = connect(&url).await;
    handshake(&mut ws).await;

    for i in 0..2 {
        send_text(&mut ws, &format!("SUB sub-{i} Ping demo")).await;
        match parse_server(&recv_text(&mut ws).await).unwrap() {
            ServerOp::Ok => {}
            other => panic!("expected +OK for subscription {i}, got {other}"),
        }
    }

    send_text(&mut ws, "SUB sub-2 Ping demo").await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Err { error, details } => {
            assert_eq!(error, OwpError::IllegalState);
            assert_eq!(details.as_deref(), Some("subscription limit reached"));
        }
        other => panic!("expected ERR past the subscription limit, got {other}"),
    }

    // The connection survives a refusal: the limit is not a reason to hang up,
    // and the subscriptions already established keep working.
    send_text(&mut ws, r#"PUB demo {"Ping":{"n":1}}"#).await;
    match parse_server(&recv_text(&mut ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected the session to continue after a refusal, got {other}"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn connections_past_the_limit_are_refused_then_allowed_again() {
    let engine = Arc::new(Engine::new());
    let (url, shutdown) = start_owp_with(engine, |config| config.max_connections = 1).await;

    let mut first = connect(&url).await;
    handshake(&mut first).await;

    // The adapter accepts the socket to keep the accept loop draining, then
    // closes it, so the client's handshake is what fails.
    assert!(
        try_connect(&url).await.is_err(),
        "second connection should be refused while the first holds the only slot"
    );

    drop(first);

    // The permit is released when the session task ends, which is not
    // synchronous with the client's drop.
    let mut recovered = false;
    for _ in 0..40 {
        if try_connect(&url).await.is_ok() {
            recovered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        recovered,
        "a slot should free up once the first connection ends"
    );

    shutdown.cancel();
}

/// [`connect`] panics on failure, which is what most tests want.
async fn try_connect(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut req = url.into_client_request()?;
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "owp".parse()?);
    let (ws, _) = timeout(
        Duration::from_secs(2),
        tokio_tungstenite::connect_async(req),
    )
    .await??;
    drop(ws);
    Ok(())
}
