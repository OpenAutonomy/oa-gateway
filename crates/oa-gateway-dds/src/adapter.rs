//! Bridges configured engine topics onto DDS topics of the same name.
//!
//! Protocol I/O stays in the [`DdsProvider`](crate::DdsProvider). This
//! type owns engine subscribe/publish, A-GRA unwrap, and echo skip.
//! [`Engine::drop_adapter`] runs at session start and end.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use oa_gateway_adapter::{Adapter, AdapterError};
use oa_gateway_agra::{
    element_to_enum, unwrap as unwrap_ma, unwrapped_from_parts, wrapper_kind, WrapperKind,
    WrapperMeta,
};
use oa_gateway_core::{
    AdapterId, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::DdsConfig;
use crate::provider::{provider_for, DdsSession};
use crate::types::DdsSample;

/// DDS participant that bridges configured topics both ways.
///
/// Inbound samples are stamped with `oag.origin_adapter` so outbound
/// writes can refuse the echo when [`DdsConfig::suppress_echo`] is on.
/// The rustdds provider also drops samples whose writer shares this
/// participant's GUID prefix, because rustdds delivers local writes.
pub struct DdsAdapter {
    id: AdapterId,
    config: DdsConfig,
}

impl DdsAdapter {
    /// Builds an adapter that has not yet joined a domain.
    #[must_use]
    pub fn new(id: impl Into<AdapterId>, config: DdsConfig) -> Self {
        Self {
            id: id.into(),
            config,
        }
    }

    #[must_use]
    pub fn id(&self) -> &AdapterId {
        &self.id
    }

    #[must_use]
    pub fn config(&self) -> &DdsConfig {
        &self.config
    }

    /// Joins the domain, creates topics, and bridges until shutdown.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Failed`] if the provider cannot join or
    /// a topic cannot be created.
    pub async fn serve(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let provider = provider_for(self.config.provider);
        info!(
            adapter = %self.id,
            provider = provider.name(),
            domain = self.config.domain_id,
            "starting dds adapter"
        );
        let mut session = provider
            .join(self.config.domain_id, &self.config.qos)
            .map_err(|err| AdapterError::failed(&self.id, err.to_string()))?;
        for topic in &self.config.topics {
            session
                .create_topic(topic)
                .map_err(|err| AdapterError::failed(&self.id, err.to_string()))?;
        }

        engine.drop_adapter(self.id.clone()).await;
        let (tx, mut rx) = mpsc::channel::<Delivery>(DEFAULT_CHANNEL_CAPACITY);
        for (i, topic) in self.config.topics.iter().enumerate() {
            engine
                .subscribe(
                    self.id.clone(),
                    SubId::new(format!("dds-{i}")),
                    RouteKey::topic(topic.clone()),
                    tx.clone(),
                )
                .await
                .map_err(|err| AdapterError::failed(&self.id, err.to_string()))?;
        }
        drop(tx);

        let result = self
            .drive(&mut *session, engine.clone(), &mut rx, &shutdown)
            .await;
        engine.drop_adapter(self.id.clone()).await;
        result
    }

    async fn drive(
        &self,
        session: &mut dyn DdsSession,
        engine: Arc<Engine>,
        eng_rx: &mut mpsc::Receiver<Delivery>,
        shutdown: &CancellationToken,
    ) -> Result<(), AdapterError> {
        let mut tick = interval(Duration::from_millis(20));
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!(adapter = %self.id, "dds shutting down");
                    return Ok(());
                }
                delivery = eng_rx.recv() => {
                    let Some(delivery) = delivery else { return Ok(()); };
                    if let Err(err) = self.forward_outbound(session, delivery) {
                        warn!(adapter = %self.id, error = %err, "dropping outbound dds sample");
                    }
                }
                _ = tick.tick() => {
                    if let Err(err) = self.poll_inbound(session, &engine).await {
                        return Err(AdapterError::failed(&self.id, err));
                    }
                }
            }
        }
    }

    fn forward_outbound(
        &self,
        session: &mut dyn DdsSession,
        delivery: Delivery,
    ) -> Result<(), String> {
        if self.config.suppress_echo && delivery.envelope.is_echo_of(&self.id) {
            return Ok(());
        }
        let topic = delivery.envelope.route.topic.clone();
        if !self.config.topics.iter().any(|t| t == &topic) {
            return Ok(());
        }
        let sample = sample_from_envelope(&delivery.envelope)?;
        session.write(&topic, sample).map_err(|err| err.to_string())
    }

    async fn poll_inbound(
        &self,
        session: &mut dyn DdsSession,
        engine: &Arc<Engine>,
    ) -> Result<(), String> {
        for (topic, sample) in session.poll_inbound().map_err(|err| err.to_string())? {
            inbound_publish(self, engine, &topic, sample).await?;
        }
        Ok(())
    }
}

async fn inbound_publish(
    adapter: &DdsAdapter,
    engine: &Arc<Engine>,
    topic: &str,
    sample: DdsSample,
) -> Result<(), String> {
    let stamp = |env: Envelope| env.with_origin(&adapter.id);
    if adapter.config.unwrap_ma_payloads {
        let peeled = unwrapped_from_parts(topic, sample.meta, sample.encoded)
            .map_err(|err| err.to_string())?;
        log_drops(adapter, engine.publish(stamp(peeled.wrapper)).await);
        log_drops(adapter, engine.publish(stamp(peeled.inner)).await);
        return Ok(());
    }
    let env = Envelope::new(
        RouteKey::typed(topic, sample.meta.kind.element_name()),
        sample.encoded,
    );
    log_drops(adapter, engine.publish(stamp(env)).await);
    Ok(())
}

fn sample_from_envelope(envelope: &Envelope) -> Result<DdsSample, String> {
    if wrapper_kind(&envelope.payload).is_some() {
        let peeled =
            unwrap_ma(&envelope.route.topic, &envelope.payload).map_err(|err| err.to_string())?;
        return Ok(DdsSample {
            meta: peeled.meta,
            encoded: peeled.inner.payload,
        });
    }
    let hint = envelope
        .route
        .type_hint
        .clone()
        .unwrap_or_else(|| envelope.route.topic.clone());
    Ok(DdsSample {
        meta: WrapperMeta {
            kind: WrapperKind::Rx,
            message_type_enum: element_to_enum(&hint),
            originator_uuid: None,
            rx_payload_id: None,
            command_id: None,
            destination_routing: None,
        },
        encoded: Bytes::copy_from_slice(&envelope.payload),
    })
}

fn log_drops(adapter: &DdsAdapter, outcome: oa_gateway_core::PublishOutcome) {
    if outcome.dropped > 0 {
        warn!(
            adapter = %adapter.id,
            dropped = outcome.dropped,
            "dds publish dropped"
        );
    }
}

#[async_trait]
impl Adapter for DdsAdapter {
    fn id(&self) -> &AdapterId {
        DdsAdapter::id(self)
    }

    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        self.serve(engine, shutdown).await
    }
}
