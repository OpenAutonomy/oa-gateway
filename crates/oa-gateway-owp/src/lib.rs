//! OWP 1.0 adapter: WebSocket text frames in, [`oa_gateway_core::Envelope`] out.
//!
//! This crate is a server. Clients speak OWP over a text WebSocket; the
//! codec follows OMSC-SPC-013 and does not compile UCI. Conversion and
//! validation happen in the session when a schema and `xml_baseline`
//! are configured. The engine sees only envelopes.
//!
//! [`OwpAdapter::run`] binds [`OwpConfig::bind`] and accepts until
//! cancelled. A failed bind is fatal for this adapter only.

mod codec;
mod config;
mod convert;
mod server;
mod session;

use std::sync::Arc;

use async_trait::async_trait;
use oa_gateway_adapter::{after_join, Adapter, AdapterError, AfterSession};
use oa_gateway_core::{AdapterId, Engine};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub use codec::{
    is_identifier, parse_client, parse_server, type_hint_from_json, ClientOp, InfoPayload,
    InitPayload, OwpError, ServerOp,
};
pub use config::{
    OwpConfig, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_FRAME_SIZE, DEFAULT_MAX_SUBSCRIPTIONS,
};
pub use server::OwpAdapter;

#[async_trait]
impl Adapter for OwpAdapter {
    fn id(&self) -> &AdapterId {
        OwpAdapter::id(self)
    }

    /// Binds the configured address and accepts connections until
    /// `shutdown` is cancelled, retrying a bind failure, a session
    /// error, or a session panic per [`OwpConfig::on_panic`] and
    /// [`OwpConfig::reconnect`].
    ///
    /// Each attempt runs on its own child task, so a panic in the
    /// accept loop is a join error here rather than an unwind that
    /// would otherwise take the retry loop down with it.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Io`] if the address cannot be bound and
    /// [`OwpConfig::reconnect`] is off. Per-connection session errors
    /// are handled inside the accept loop and never reach here.
    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            let adapter = Arc::clone(&self);
            let eng = Arc::clone(&engine);
            let token = shutdown.clone();
            let joined =
                tokio::spawn(async move { adapter.bind_and_serve(eng, token).await }).await;
            match after_join(
                joined,
                self.config().reconnect,
                self.config().on_panic,
                self.id(),
            ) {
                AfterSession::ReturnOk => return Ok(()),
                AfterSession::ReturnErr(err) => return Err(err),
                AfterSession::Retry { message } => {
                    if shutdown.is_cancelled() {
                        return Ok(());
                    }
                    warn!(adapter = %self.id(), "{message}");
                }
            }
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(self.config().reconnect_delay) => {}
            }
        }
    }
}

impl OwpAdapter {
    /// One bind-and-accept session: the unit [`Adapter::run`]'s retry
    /// loop restarts on failure or panic.
    async fn bind_and_serve(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let listener = TcpListener::bind(self.config().bind).await?;
        self.serve(listener, engine, shutdown).await
    }
}
