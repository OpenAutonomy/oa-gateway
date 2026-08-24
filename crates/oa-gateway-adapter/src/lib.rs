//! Adapter contract. Protocol plugins implement [`Adapter`] and talk only to
//! [`oa_gateway_core::Engine`] — never to each other.
//!
//! An adapter owns one side of the gateway: its socket, its framing, its
//! handshake, and any schema translation. What it hands the engine is an
//! [`Envelope`](oa_gateway_core::Envelope) addressed by a [`RouteKey`](oa_gateway_core::RouteKey),
//! with the payload left opaque. See `docs/writing-an-adapter.md` for the full
//! walkthrough; the shape is:
//!
//! ```
//! # use std::sync::Arc;
//! use async_trait::async_trait;
//! use oa_gateway_adapter::{Adapter, AdapterError};
//! use oa_gateway_core::{
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
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use oa_gateway_core::{AdapterId, Engine};
use tokio_util::sync::CancellationToken;

mod supervise;
pub use supervise::{after_join, AfterSession, OnPanic};

/// Fatal failure of one adapter. The host logs it and leaves the others running.
///
/// This is not a per-message error. A bad payload is handled inside `run`
/// (drop, nack, or reply) and does not become [`AdapterError`].
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// The adapter's `run` loop cannot continue.
    ///
    /// `adapter` is the id the host logs. `message` is what an operator
    /// should see: a protocol error, a session it will not retry, a
    /// schema it cannot load.
    #[error("adapter {adapter} failed: {message}")]
    Failed { adapter: String, message: String },
    /// A listen or connect socket failed.
    ///
    /// Kept as [`std::io::Error`] so bind/connect can use `?` without
    /// wrapping. Not used for protocol-level failures; those are
    /// [`Self::Failed`].
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl AdapterError {
    /// Builds a [`Self::Failed`] from an adapter id and an operator-facing
    /// message.
    #[must_use]
    pub fn failed(adapter: impl std::fmt::Display, message: impl Into<String>) -> Self {
        Self::Failed {
            adapter: adapter.to_string(),
            message: message.into(),
        }
    }
}

/// A protocol plugin that owns its I/O loop and maps native frames onto
/// envelopes.
///
/// The engine is the only shared state. Adapters never call each other.
/// Each one creates the `mpsc` channel it hands to
/// [`Engine::subscribe`](oa_gateway_core::Engine::subscribe) and reads
/// the matching receiver. Call
/// [`Engine::drop_adapter`](oa_gateway_core::Engine::drop_adapter) on
/// the way out, or subscriptions keep matching and silently discarding
/// messages.
#[async_trait]
pub trait Adapter: Send + Sync + 'static {
    /// Stable id for this adapter instance.
    ///
    /// The host logs it. Bridging adapters also stamp
    /// `oag.origin_adapter` with it so they can refuse their own echo.
    fn id(&self) -> &AdapterId;

    /// Runs until `shutdown` is cancelled, or until this adapter cannot
    /// continue.
    ///
    /// This is the whole lifetime: accept or connect, read frames,
    /// publish envelopes, and return. Subscribe only after the
    /// transport is up, so deliveries are not queued with nowhere to
    /// go. Observe `shutdown` in the same loop that reads the
    /// transport.
    ///
    /// # Errors
    ///
    /// Returning [`Err`] is fatal for this adapter only. The host logs
    /// it and does not restart `run`. [`Ok`] is the shutdown path.
    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError>;
}
