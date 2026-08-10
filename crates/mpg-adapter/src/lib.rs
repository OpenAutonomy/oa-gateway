//! Adapter contract. Protocol plugins implement [`Adapter`] and talk only to
//! [`mpg_core::Engine`] — never to each other.
//!
//! An adapter owns one side of the gateway: its socket, its framing, its
//! handshake, and any schema translation. What it hands the engine is an
//! [`Envelope`](mpg_core::Envelope) addressed by a [`RouteKey`](mpg_core::RouteKey),
//! with the payload left opaque. See `docs/writing-an-adapter.md` for the full
//! walkthrough; the shape is:
//!
//! ```
//! # use std::sync::Arc;
//! use async_trait::async_trait;
//! use mpg_adapter::{Adapter, AdapterError};
//! use mpg_core::{
//!     AdapterId, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
//! };
//! use tokio::sync::mpsc;
//! use tokio_util::sync::CancellationToken;
//!
//! /// Stands in for a real protocol: answers every `Ping` on `demo` with a `Pong`.
//! struct Echo {
//!     id: AdapterId,
//! }
//!
//! #[async_trait]
//! impl Adapter for Echo {
//!     fn id(&self) -> &AdapterId {
//!         &self.id
//!     }
//!
//!     async fn run(
//!         self: Arc<Self>,
//!         engine: Arc<Engine>,
//!         shutdown: CancellationToken,
//!     ) -> Result<(), AdapterError> {
//!         // Deliveries arrive on a channel this adapter owns.
//!         let (tx, mut rx) = mpsc::channel::<Delivery>(DEFAULT_CHANNEL_CAPACITY);
//!         engine
//!             .subscribe(
//!                 self.id.clone(),
//!                 SubId::new("echo-1"),
//!                 RouteKey::typed("demo", "Ping"),
//!                 tx,
//!             )
//!             .await
//!             .map_err(|err| AdapterError::failed(&self.id, err.to_string()))?;
//!
//!         loop {
//!             tokio::select! {
//!                 // Always leave the engine clean on the way out.
//!                 _ = shutdown.cancelled() => {
//!                     engine.drop_adapter(self.id.clone()).await;
//!                     return Ok(());
//!                 }
//!                 delivery = rx.recv() => {
//!                     let Some(delivery) = delivery else { return Ok(()) };
//!                     let reply = Envelope::new(
//!                         RouteKey::typed("demo", "Pong"),
//!                         delivery.envelope.payload,
//!                     );
//!                     engine.publish(reply).await;
//!                 }
//!             }
//!         }
//!     }
//! }
//! # #[tokio::main]
//! # async fn main() {
//! #     use std::time::Duration;
//! #     let engine = Arc::new(Engine::new());
//! #     let shutdown = CancellationToken::new();
//! #
//! #     let (obs_tx, mut obs_rx) = mpsc::channel::<Delivery>(8);
//! #     engine
//! #         .subscribe("observer", SubId::new("obs-1"), RouteKey::typed("demo", "Pong"), obs_tx)
//! #         .await
//! #         .unwrap();
//! #
//! #     let echo = Arc::new(Echo { id: "echo".into() });
//! #     tokio::spawn(Arc::clone(&echo).run(Arc::clone(&engine), shutdown.clone()));
//! #
//! #     tokio::time::timeout(Duration::from_secs(5), async {
//! #         while engine.subscription_count().await < 2 {
//! #             tokio::time::sleep(Duration::from_millis(1)).await;
//! #         }
//! #     })
//! #     .await
//! #     .expect("echo subscribed");
//! #
//! #     engine
//! #         .publish(Envelope::new(RouteKey::typed("demo", "Ping"), b"hi".to_vec()))
//! #         .await;
//! #
//! #     let pong = tokio::time::timeout(Duration::from_secs(5), obs_rx.recv())
//! #         .await
//! #         .expect("pong timed out")
//! #         .expect("channel closed");
//! #     assert_eq!(pong.envelope.payload.as_ref(), b"hi");
//! #     shutdown.cancel();
//! # }
//! ```

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
