//! Adapter contract. Protocol plugins implement [`Adapter`] and talk only to
//! [`mpg_core::Engine`] — never to each other.

use std::sync::Arc;

use async_trait::async_trait;
use mpg_core::{AdapterId, Engine};
use tokio_util::sync::CancellationToken;

/// Fatal adapter failure. The host logs this and continues with remaining adapters.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("adapter {adapter} failed: {message}")]
    Failed { adapter: String, message: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl AdapterError {
    #[must_use]
    pub fn failed(adapter: impl std::fmt::Display, message: impl Into<String>) -> Self {
        Self::Failed {
            adapter: adapter.to_string(),
            message: message.into(),
        }
    }
}

/// A protocol plugin that owns its I/O loop and maps native frames onto envelopes.
#[async_trait]
pub trait Adapter: Send + Sync + 'static {
    fn id(&self) -> &AdapterId;

    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError>;
}
