//! OWP 1.0 adapter: WebSocket text frames in, [`mpg_core::Envelope`] out.

mod codec;
mod server;

use std::sync::Arc;

use async_trait::async_trait;
use mpg_adapter::{Adapter, AdapterError};
use mpg_core::{AdapterId, Engine};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub use codec::{
    is_identifier, parse_client, parse_server, type_hint_from_json, ClientOp, InfoPayload,
    InitPayload, OwpError, ServerOp,
};
pub use server::{OwpAdapter, OwpConfig};

#[async_trait]
impl Adapter for OwpAdapter {
    fn id(&self) -> &AdapterId {
        OwpAdapter::id(self)
    }

    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let listener = TcpListener::bind(self.config().bind).await?;
        self.serve(listener, engine, shutdown).await
    }
}
