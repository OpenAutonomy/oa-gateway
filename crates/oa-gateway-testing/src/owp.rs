//! In-process OWP server and a WebSocket client for it.
//!
//! [`start_owp`] binds an ephemeral port and serves
//! [`oa_gateway_owp::OwpAdapter`] with the UCI fixture schema
//! ([`oa_gateway_uci::slice::v25`]), not a compiled XSD. That is
//! enough for the messages in [`crate::fixtures`]. Helpers panic on
//! timeout or a surprising frame; they are not a public client API.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use oa_gateway_core::Engine;
use oa_gateway_owp::{parse_server, OwpAdapter, OwpConfig, ServerOp};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::sync::CancellationToken;

/// Client WebSocket after the `owp` subprotocol handshake.
pub type OwpWs = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Starts an OWP adapter on an ephemeral port, wired to the fixture
/// schema.
///
/// The fixture schema stands in for a real UCI schema, which the
/// gateway otherwise loads from the published XSD at startup. It
/// covers the message types the fixtures use, so `xml_baseline`
/// conversion works without a local copy of the standard. INIT
/// `schema` is `002.5.0` so [`handshake`] matches.
///
/// Returns a `ws://127.0.0.1:{port}/` URL and a token that stops the
/// accept loop. Panics if the port cannot be bound.
pub async fn start_owp(engine: Arc<Engine>, xml_baseline: bool) -> (String, CancellationToken) {
    start_owp_with(engine, |config| config.xml_baseline = xml_baseline).await
}

/// As [`start_owp`], with the config open for editing first.
///
/// Tests for the resource limits set them far below their defaults, so
/// that reaching one costs a few frames instead of megabytes. The
/// listener is already bound; `edit` must not change
/// [`OwpConfig::bind`] to a different address.
pub async fn start_owp_with(
    engine: Arc<Engine>,
    edit: impl FnOnce(&mut OwpConfig),
) -> (String, CancellationToken) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = CancellationToken::new();

    let mut config = OwpConfig {
        bind: addr,
        server_id: "oa-gateway-test".into(),
        system_label: "test".into(),
        schema: Some("002.5.0".into()),
        system_uuid: "11111111-1111-4111-8111-111111111111".into(),
        ..OwpConfig::default()
    };
    edit(&mut config);

    let adapter = Arc::new(
        OwpAdapter::new("owp-test", config)
            .with_schema(Arc::new(oa_gateway_uci::slice::v25().clone())),
    );
    let token = shutdown.clone();
    tokio::spawn(async move {
        adapter.serve(listener, engine, token).await.unwrap();
    });
    (format!("ws://{addr}/"), shutdown)
}

/// Opens a WebSocket to `url` with `Sec-WebSocket-Protocol: owp`.
///
/// Panics if the handshake fails. The adapter refuses a socket that
/// omits this subprotocol.
pub async fn connect(url: &str) -> OwpWs {
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", "owp".parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws
}

/// Sends one OWP text frame. Panics if the socket is closed.
pub async fn send_text(ws: &mut OwpWs, frame: &str) {
    ws.send(Message::Text(frame.to_owned().into()))
        .await
        .unwrap();
}

/// Next text frame, answering WebSocket pings.
///
/// Waits up to two seconds. Panics on timeout, a close, or any frame
/// that is not text or ping.
pub async fn recv_text(ws: &mut OwpWs) -> String {
    loop {
        let msg = timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("owp recv timeout")
            .expect("closed")
            .unwrap();
        match msg {
            Message::Text(t) => return t.to_string(),
            Message::Ping(p) => {
                ws.send(Message::Pong(p)).await.unwrap();
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }
}

/// INIT 1.0 / schema `002.5.0` / `verbose`, then expects `+OK` and
/// `INFO`.
///
/// Matches [`start_owp`]'s configured schema. Panics if either reply
/// is missing or the wrong opcode.
pub async fn handshake(ws: &mut OwpWs) {
    send_text(
        ws,
        r#"INIT {"versions":["1.0"],"schema":"002.5.0","service_id":"web-app","verbose":true}"#,
    )
    .await;
    match parse_server(&recv_text(ws).await).unwrap() {
        ServerOp::Ok => {}
        other => panic!("expected +OK, got {other}"),
    }
    match parse_server(&recv_text(ws).await).unwrap() {
        ServerOp::Info(_) => {}
        other => panic!("expected INFO, got {other}"),
    }
}
