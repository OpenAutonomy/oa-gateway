//! Accept loop and WebSocket handshake. Sessions live in `session`.
//!
//! This module refuses extra connections instead of queueing them, so a
//! caller learns immediately and the accept loop keeps draining.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use oa_gateway_adapter::tls::{MaybeTlsStream, ServerTls};
use oa_gateway_adapter::AdapterError;
use oa_gateway_core::{AdapterId, Engine};
use oa_gateway_uci::Schema;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::OwpConfig;
use crate::session::{self, Session};

/// Budget for a client to complete the TLS handshake, when TLS is
/// configured. Without a limit, a peer that opens TCP and never speaks TLS
/// would hold a connection permit forever.
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// OWP/WebSocket server adapter.
///
/// [`Self::new`] does not bind. [`Self::serve`] takes a listener the
/// host already bound. A compiled UCI schema is optional and is
/// attached with [`Self::with_schema`].
pub struct OwpAdapter {
    id: AdapterId,
    config: OwpConfig,
    conn_seq: AtomicU64,
    schema: Option<Arc<Schema>>,
    tls: Option<ServerTls>,
    /// One permit per allowed connection, held for the life of the session.
    connections: Arc<Semaphore>,
    /// Set while connections are being refused, so saturation is logged on the
    /// way in and on the way out instead of once per rejected connection.
    at_capacity: AtomicBool,
}

impl OwpAdapter {
    /// Builds an adapter that is not yet listening.
    ///
    /// The connection semaphore is sized from
    /// [`OwpConfig::max_connections`]. No schema is attached until
    /// [`Self::with_schema`], and the listener is plaintext until
    /// [`Self::with_tls`].
    #[must_use]
    pub fn new(id: impl Into<AdapterId>, config: OwpConfig) -> Self {
        let connections = Arc::new(Semaphore::new(config.max_connections));
        Self {
            id: id.into(),
            config,
            conn_seq: AtomicU64::new(1),
            schema: None,
            tls: None,
            connections,
            at_capacity: AtomicBool::new(false),
        }
    }

    /// Supply the UCI schema used to convert between OMS JSON and UCI XML.
    ///
    /// Without one the adapter still routes, but it cannot convert: XML payloads
    /// keep their topic as the type hint and are forwarded verbatim. A schema is
    /// mandatory for [`OwpConfig::xml_baseline`], which the host enforces at
    /// startup so the failure surfaces before any traffic arrives.
    #[must_use]
    pub fn with_schema(mut self, schema: Arc<Schema>) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Terminate TLS on every accepted connection.
    ///
    /// Without this the listener stays plaintext, which is unchanged
    /// behavior for a deployment that configures no certificate.
    #[must_use]
    pub fn with_tls(mut self, tls: ServerTls) -> Self {
        self.tls = Some(tls);
        self
    }

    #[must_use]
    pub fn id(&self) -> &AdapterId {
        &self.id
    }

    #[must_use]
    pub fn config(&self) -> &OwpConfig {
        &self.config
    }

    /// Accepts connections on `listener` until `shutdown` is cancelled.
    ///
    /// A failed `accept` is logged and the loop continues. At the
    /// connection limit the TCP stream is dropped immediately; the
    /// saturation warning is logged once on the way in and once on the
    /// way out, not once per refused peer.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Io`] if the local address of `listener`
    /// cannot be read. Handshake and session failures do not fail
    /// `serve`.
    pub async fn serve(
        self: Arc<Self>,
        listener: TcpListener,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let local = listener.local_addr()?;
        info!(%local, adapter = %self.id, tls = self.tls.is_some(), "owp listening");

        loop {
            tokio::select! {
                () = shutdown.cancelled() => {
                    info!(adapter = %self.id, "owp shutting down");
                    return Ok(());
                }
                accepted = listener.accept() => {
                    let (stream, peer) = match accepted {
                        Ok(v) => v,
                        Err(err) => {
                            warn!(error = %err, "accept failed");
                            continue;
                        }
                    };
                    // Refuse rather than queue: a caller that cannot get a slot
                    // learns immediately, and the accept loop keeps draining so
                    // the backlog does not become the queue instead.
                    let Ok(permit) = Arc::clone(&self.connections).try_acquire_owned() else {
                        if !self.at_capacity.swap(true, Ordering::Relaxed) {
                            warn!(
                                adapter = %self.id,
                                limit = self.config.max_connections,
                                "at the connection limit, refusing new connections"
                            );
                        }
                        drop(stream);
                        continue;
                    };
                    if self.at_capacity.swap(false, Ordering::Relaxed) {
                        info!(adapter = %self.id, "below the connection limit, accepting again");
                    }
                    let conn_id = self.conn_seq.fetch_add(1, Ordering::Relaxed);
                    let this = Arc::clone(&self);
                    let engine = Arc::clone(&engine);
                    let shutdown = shutdown.clone();
                    tokio::spawn(async move {
                        if let Err(err) = this.handle_connection(stream, peer, conn_id, engine, shutdown).await {
                            debug!(%peer, conn_id, error = %err, "owp connection ended");
                        }
                        drop(permit);
                    });
                }
            }
        }
    }

    /// Handshakes one TCP stream and runs its OWP session.
    ///
    /// A failed WebSocket handshake is logged and returns [`Ok`], so a
    /// bad client does not take the accept loop down. Oversized frames
    /// are rejected by the WebSocket cap before the payload is fully
    /// buffered.
    async fn handle_connection(
        &self,
        stream: TcpStream,
        peer: SocketAddr,
        conn_id: u64,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let stream: MaybeTlsStream<TcpStream> = match &self.tls {
            Some(tls) => match timeout(TLS_HANDSHAKE_TIMEOUT, tls.accept(stream)).await {
                Ok(Ok(stream)) => stream,
                Ok(Err(err)) => {
                    debug!(%peer, error = %err, "tls handshake failed");
                    return Ok(());
                }
                Err(_) => {
                    debug!(%peer, "tls handshake timed out");
                    return Ok(());
                }
            },
            None => MaybeTlsStream::Plain(stream),
        };

        // A frame over the cap is a protocol error the library reports on read,
        // which ends the session — the same outcome as an unparseable STOMP
        // frame, and it happens before the payload is fully buffered.
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(self.config.max_frame_size))
            .max_frame_size(Some(self.config.max_frame_size));
        let ws =
            match accept_hdr_async_with_config(stream, check_subprotocol, Some(ws_config)).await {
                Ok(ws) => ws,
                Err(err) => {
                    debug!(%peer, error = %err, "websocket handshake failed");
                    return Ok(());
                }
            };
        session::run(Session {
            adapter_id: self.id.clone(),
            config: self.config.clone(),
            schema: self.schema.clone(),
            conn_id,
            ws,
            engine,
            shutdown,
        })
        .await
    }
}

#[allow(clippy::result_large_err)]
/// Requires the `owp` WebSocket subprotocol (case-insensitive).
///
/// Missing or unmatched `Sec-WebSocket-Protocol` is `400`. On success
/// the response echoes `owp`.
fn check_subprotocol(req: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    let has_owp = req
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| {
            s.split(',')
                .map(str::trim)
                .any(|p| p.eq_ignore_ascii_case("owp"))
        });
    if !has_owp {
        let mut err = ErrorResponse::new(Some("missing owp subprotocol".into()));
        *err.status_mut() = StatusCode::BAD_REQUEST;
        return Err(err);
    }
    response
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_static("owp"));
    Ok(response)
}
