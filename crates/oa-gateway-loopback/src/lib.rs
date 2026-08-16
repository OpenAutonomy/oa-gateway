//! In-process adapter: publish and subscribe without sockets.
//!
//! Multiple [`Loopback`] instances can share one [`Engine`]. They never
//! talk to each other — traffic only crosses through the engine. Used
//! by the host when `[loopback]` is enabled, and by tests that need a
//! peer without OWP or a broker.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use oa_gateway_adapter::{Adapter, AdapterError};
use oa_gateway_core::{
    AdapterId, Delivery, Engine, EngineError, Envelope, PublishOutcome, RouteKey, SubId,
    DEFAULT_CHANNEL_CAPACITY,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// In-process handle onto the engine.
///
/// Holds its own [`Engine`] so tests can [`Self::subscribe`] and
/// [`Self::publish`] without going through [`Adapter::run`]. Two
/// handles must not share an [`AdapterId`]: [`Self::shutdown`] drops
/// every subscription under that id.
pub struct Loopback {
    id: AdapterId,
    engine: Arc<Engine>,
    next_sub: AtomicU64,
}

impl Loopback {
    /// Binds this handle to `engine` under `id`.
    ///
    /// Does not subscribe and does not start [`Adapter::run`]. The
    /// engine is stored here; `run` ignores the engine the host passes
    /// in.
    #[must_use]
    pub fn new(engine: Arc<Engine>, id: impl Into<AdapterId>) -> Self {
        Self {
            id: id.into(),
            engine,
            next_sub: AtomicU64::new(1),
        }
    }

    /// The engine this handle was built with.
    #[must_use]
    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    /// Subscribes and returns a receiver of matching [`Envelope`]s.
    ///
    /// [`Delivery::sub_id`] is stripped; this handle assigns `lb-N`
    /// ids internally. Dropping the receiver stops the forwarder, but
    /// the engine subscription stays until [`Self::shutdown`]. Further
    /// publishes then count as dropped.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::DuplicateSub`] if an internal id collides,
    /// which does not happen on a single handle.
    pub async fn subscribe(
        &self,
        route: RouteKey,
    ) -> Result<mpsc::Receiver<Envelope>, EngineError> {
        let sub_id = SubId::new(format!(
            "lb-{}",
            self.next_sub.fetch_add(1, Ordering::Relaxed)
        ));
        let (tx, mut rx) = mpsc::channel::<Delivery>(DEFAULT_CHANNEL_CAPACITY);
        self.engine
            .subscribe(self.id.clone(), sub_id, route, tx)
            .await?;

        let (out_tx, out_rx) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
        tokio::spawn(async move {
            while let Some(delivery) = rx.recv().await {
                if out_tx.send(delivery.envelope).await.is_err() {
                    break;
                }
            }
        });
        Ok(out_rx)
    }

    /// Publishes through the shared engine. Same fan-out and drop rules
    /// as [`Engine::publish`].
    pub async fn publish(&self, envelope: Envelope) -> PublishOutcome {
        self.engine.publish(envelope).await
    }

    /// Drops every subscription this handle owns.
    ///
    /// Returns how many were removed. Does not cancel [`Adapter::run`];
    /// the host's cancellation token does that.
    pub async fn shutdown(&self) -> usize {
        self.engine.drop_adapter(self.id.clone()).await
    }
}

#[async_trait]
impl Adapter for Loopback {
    fn id(&self) -> &AdapterId {
        &self.id
    }

    /// Waits for `shutdown`, then drops this handle's subscriptions.
    ///
    /// There is no socket. Tests that only call [`Loopback::subscribe`]
    /// and [`Loopback::publish`] never need this.
    ///
    /// # Errors
    ///
    /// Does not fail. [`Ok`] after the token fires.
    async fn run(
        self: Arc<Self>,
        _engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        shutdown.cancelled().await;
        self.shutdown().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use tokio::time::{timeout, Duration};

    use super::*;

    async fn recv(rx: &mut mpsc::Receiver<Envelope>) -> Envelope {
        timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timeout")
            .expect("closed")
    }

    #[tokio::test]
    async fn two_loopbacks_cross_the_engine() {
        let engine = Arc::new(Engine::new());
        let a = Loopback::new(engine.clone(), "loop-a");
        let b = Loopback::new(engine.clone(), "loop-b");

        let mut rx = b.subscribe(RouteKey::typed("demo", "Ping")).await.unwrap();
        let sent = Envelope::new(RouteKey::typed("demo", "Ping"), Bytes::from_static(b"hi"))
            .with_header("src", "a");
        a.publish(sent.clone()).await;

        let got = recv(&mut rx).await;
        assert_eq!(got.payload, sent.payload);
        assert_eq!(got.headers.get("src").map(String::as_str), Some("a"));
    }

    #[tokio::test]
    async fn type_filter_and_wildcard() {
        let engine = Arc::new(Engine::new());
        let a = Loopback::new(engine.clone(), "loop-a");
        let b = Loopback::new(engine.clone(), "loop-b");

        let mut ping_rx = b.subscribe(RouteKey::typed("demo", "Ping")).await.unwrap();
        let mut wild_rx = b.subscribe(RouteKey::topic("demo")).await.unwrap();

        a.publish(Envelope::new(
            RouteKey::typed("demo", "Ping"),
            Bytes::from_static(b"ping"),
        ))
        .await;
        a.publish(Envelope::new(
            RouteKey::typed("demo", "Pong"),
            Bytes::from_static(b"pong"),
        ))
        .await;

        assert_eq!(recv(&mut ping_rx).await.payload.as_ref(), b"ping");
        match timeout(Duration::from_millis(50), ping_rx.recv()).await {
            Err(_) | Ok(None) => {}
            Ok(Some(env)) => panic!("Ping subscriber must not see {:?}", env.route),
        }

        let mut wild = vec![
            recv(&mut wild_rx).await.payload.to_vec(),
            recv(&mut wild_rx).await.payload.to_vec(),
        ];
        wild.sort();
        assert_eq!(wild, [b"ping".to_vec(), b"pong".to_vec()]);
    }

    #[tokio::test]
    async fn shutdown_unsubscribes() {
        let engine = Arc::new(Engine::new());
        let a = Loopback::new(engine.clone(), "loop-a");
        let b = Loopback::new(engine.clone(), "loop-b");
        let mut rx = b.subscribe(RouteKey::typed("demo", "Ping")).await.unwrap();
        assert_eq!(b.shutdown().await, 1);
        a.publish(Envelope::new(
            RouteKey::typed("demo", "Ping"),
            Bytes::from_static(b"x"),
        ))
        .await;
        match timeout(Duration::from_millis(50), rx.recv()).await {
            Err(_) | Ok(None) => {}
            Ok(Some(env)) => panic!("shutdown must drop subscriptions, got {:?}", env.route),
        }
    }
}
