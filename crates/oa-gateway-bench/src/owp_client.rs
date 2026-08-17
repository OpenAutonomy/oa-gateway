//! Non-panicking OWP client for the hot path.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use oa_gateway_owp::{parse_server, ServerOp};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Client WebSocket after the `owp` subprotocol handshake.
pub(crate) type OwpWs = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens a WebSocket to `url` with `Sec-WebSocket-Protocol: owp`.
///
/// # Errors
///
/// Returns a message if the handshake fails.
pub(crate) async fn connect(url: &str) -> Result<OwpWs, String> {
    let mut req = url
        .into_client_request()
        .map_err(|err| format!("owp request: {err}"))?;
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "owp".parse().expect("static header"),
    );
    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|err| format!("owp connect {url}: {err}"))?;
    Ok(ws)
}

/// INIT 1.0 / schema `002.5.0`, then expects `+OK` (when verbose) and `INFO`.
///
/// # Errors
///
/// Returns a message if the server refuses the handshake or times out.
pub(crate) async fn handshake(ws: &mut OwpWs, verbose: bool) -> Result<(), String> {
    let init = format!(
        r#"INIT {{"versions":["1.0"],"schema":"002.5.0","service_id":"bench","verbose":{verbose}}}"#
    );
    send_text(ws, &init).await?;
    if verbose {
        expect_ok(ws, "INIT").await?;
    }
    match recv_op(ws).await? {
        ServerOp::Info(_) => Ok(()),
        other => Err(format!("expected INFO after INIT, got {other}")),
    }
}

/// Sends one OWP text frame.
pub(crate) async fn send_text(ws: &mut OwpWs, frame: &str) -> Result<(), String> {
    ws.send(Message::Text(frame.to_owned().into()))
        .await
        .map_err(|err| format!("owp send: {err}"))
}

/// Next server opcode, answering WebSocket pings.
pub(crate) async fn recv_op(ws: &mut OwpWs) -> Result<ServerOp, String> {
    let text = recv_text(ws).await?;
    parse_server(&text).map_err(|err| format!("owp parse: {err}"))
}

/// Next text frame, answering WebSocket pings.
pub(crate) async fn recv_text(ws: &mut OwpWs) -> Result<String, String> {
    loop {
        let msg = timeout(Duration::from_secs(5), ws.next())
            .await
            .map_err(|_| "owp recv timeout".to_owned())?
            .ok_or_else(|| "owp socket closed".to_owned())?
            .map_err(|err| format!("owp recv: {err}"))?;
        match msg {
            Message::Text(t) => return Ok(t.to_string()),
            Message::Ping(p) => {
                ws.send(Message::Pong(p))
                    .await
                    .map_err(|err| format!("owp pong: {err}"))?;
            }
            Message::Pong(_) => {}
            other => return Err(format!("unexpected websocket frame {other:?}")),
        }
    }
}

/// `SUB` and, when the server is verbose, the following `+OK`.
pub(crate) async fn subscribe(
    ws: &mut OwpWs,
    sid: &str,
    message_name: &str,
    topic: &str,
    verbose: bool,
) -> Result<(), String> {
    send_text(ws, &format!("SUB {sid} {message_name} {topic}")).await?;
    if verbose {
        expect_ok(ws, "SUB").await?;
    }
    Ok(())
}

async fn expect_ok(ws: &mut OwpWs, after: &str) -> Result<(), String> {
    match recv_op(ws).await? {
        ServerOp::Ok => Ok(()),
        other => Err(format!("expected +OK after {after}, got {other}")),
    }
}
