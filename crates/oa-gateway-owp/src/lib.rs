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
use oa_gateway_adapter::{Adapter, AdapterError};
use oa_gateway_core::{AdapterId, Engine};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

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
    /// `shutdown` is cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Io`] if the address cannot be bound.
    /// Session errors are handled per connection and do not fail `run`.
    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let listener = TcpListener::bind(self.config().bind).await?;
        self.serve(listener, engine, shutdown).await
    }
}
