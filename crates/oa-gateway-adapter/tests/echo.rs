//! The crate docs show `Echo`. This test is the part that used to hide
//! under `#` in that example: a Ping on `demo` must produce a Pong.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oa_gateway_adapter::{Adapter, AdapterError};
use oa_gateway_core::{
    AdapterId, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Echo {
    id: AdapterId,
}

#[async_trait]
impl Adapter for Echo {
    fn id(&self) -> &AdapterId {
        &self.id
    }

    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let (tx, mut rx) = mpsc::channel::<Delivery>(DEFAULT_CHANNEL_CAPACITY);
        engine
            .subscribe(
                self.id.clone(),
                SubId::new("echo-1"),
                RouteKey::typed("demo", "Ping"),
                tx,
            )
            .await
            .map_err(|err| AdapterError::failed(&self.id, err.to_string()))?;

        loop {
            tokio::select! {
                () = shutdown.cancelled() => {
                    engine.drop_adapter(self.id.clone()).await;
                    return Ok(());
                }
                delivery = rx.recv() => {
                    let Some(delivery) = delivery else { return Ok(()) };
                    let reply = Envelope::new(
                        RouteKey::typed("demo", "Pong"),
                        delivery.envelope.payload,
                    );
                    engine.publish(reply).await;
                }
            }
        }
    }
}

#[tokio::test]
async fn a_ping_on_demo_is_answered_with_a_pong() {
    let engine = Arc::new(Engine::new());
    let shutdown = CancellationToken::new();

    let (obs_tx, mut obs_rx) = mpsc::channel::<Delivery>(8);
    engine
        .subscribe(
            "observer",
            SubId::new("obs-1"),
            RouteKey::typed("demo", "Pong"),
            obs_tx,
        )
        .await
        .unwrap();

    let echo = Arc::new(Echo { id: "echo".into() });
    tokio::spawn(Arc::clone(&echo).run(Arc::clone(&engine), shutdown.clone()));

    tokio::time::timeout(Duration::from_secs(5), async {
        while engine.subscription_count().await < 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("echo subscribed");

    engine
        .publish(Envelope::new(
            RouteKey::typed("demo", "Ping"),
            b"hi".to_vec(),
        ))
        .await;

    let pong = tokio::time::timeout(Duration::from_secs(5), obs_rx.recv())
        .await
        .expect("pong timed out")
        .expect("channel closed");
    assert_eq!(pong.envelope.payload.as_ref(), b"hi");
    shutdown.cancel();
}
