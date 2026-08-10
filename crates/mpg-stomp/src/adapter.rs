//! ActiveMQ STOMP client adapter. Protocol control stays here; data crosses the engine.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mpg_adapter::{Adapter, AdapterError};
use mpg_agra::{unwrap as unwrap_ma, wrapper_kind};
use mpg_core::{AdapterId, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::client::{
    connect, disconnect_frame, send_frame, subscribe_frame, FrameReader, FrameWriter,
};
use crate::codec::Frame;
use crate::config::StompConfig;
use crate::dest::{
    inbound_route, sniff_content_type, sniff_type_hint, DestinationMap, HDR_ID, HDR_ORIGIN,
    HDR_STOMP_DEST, HDR_TOPIC, HDR_TYPE_HINT,
};

pub struct StompAdapter {
    id: AdapterId,
    config: StompConfig,
}

impl StompAdapter {
    #[must_use]
    pub fn new(id: impl Into<AdapterId>, config: StompConfig) -> Self {
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
    pub fn config(&self) -> &StompConfig {
        &self.config
    }

    pub async fn serve(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        self.serve_inner(engine, shutdown, None).await
    }

    /// Same as [`Self::serve`] but signals `ready` after the first CONNECTED + subscriptions.
    pub async fn serve_ready(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
        ready: oneshot::Sender<()>,
    ) -> Result<(), AdapterError> {
        self.serve_inner(engine, shutdown, Some(ready)).await
    }

    async fn serve_inner(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
        mut ready: Option<oneshot::Sender<()>>,
    ) -> Result<(), AdapterError> {
        loop {
            if shutdown.is_cancelled() {
                return Ok(());
            }
            match self.session(&engine, &shutdown, ready.take()).await {
                Ok(()) => {
                    if !self.config.reconnect || shutdown.is_cancelled() {
                        return Ok(());
                    }
                    warn!(adapter = %self.id, "stomp session ended, reconnecting");
                }
                Err(err) => {
                    if !self.config.reconnect || shutdown.is_cancelled() {
                        return Err(err);
                    }
                    warn!(adapter = %self.id, error = %err, "stomp session failed, retrying");
                }
            }
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }

    async fn session(
        &self,
        engine: &Arc<Engine>,
        shutdown: &CancellationToken,
        ready: Option<oneshot::Sender<()>>,
    ) -> Result<(), AdapterError> {
        let map = DestinationMap::new(self.config.destination_prefix.clone());
        let (mut reader, mut writer) = connect(&self.config).await.map_err(|err| {
            AdapterError::failed(&self.id, format!("connect {}: {err}", self.config.broker))
        })?;

        engine.drop_adapter(self.id.clone()).await;

        let (eng_tx, mut eng_rx) = mpsc::channel::<Delivery>(DEFAULT_CHANNEL_CAPACITY);
        for (i, topic) in self.config.topics.iter().enumerate() {
            let dest = map.to_stomp(topic);
            let sid = format!("sub-{i}");
            writer
                .send(&subscribe_frame(&sid, &dest))
                .await
                .map_err(|err| {
                    AdapterError::failed(&self.id, format!("SUBSCRIBE {dest}: {err}"))
                })?;
            engine
                .subscribe(
                    self.id.clone(),
                    SubId::new(format!("stomp-out-{i}")),
                    RouteKey::topic(topic.clone()),
                    eng_tx.clone(),
                )
                .await
                .map_err(|err| AdapterError::failed(&self.id, err.to_string()))?;
        }
        drop(eng_tx);

        info!(
            adapter = %self.id,
            broker = %self.config.broker,
            topics = ?self.config.topics,
            "stomp connected"
        );
        if let Some(tx) = ready {
            let _ = tx.send(());
        }

        let result = self
            .drive_session(
                &mut reader,
                &mut writer,
                engine,
                shutdown,
                &map,
                &mut eng_rx,
            )
            .await;

        let _ = writer.send(&disconnect_frame()).await;
        engine.drop_adapter(self.id.clone()).await;
        result
    }

    async fn drive_session(
        &self,
        reader: &mut FrameReader,
        writer: &mut FrameWriter,
        engine: &Arc<Engine>,
        shutdown: &CancellationToken,
        map: &DestinationMap,
        eng_rx: &mut mpsc::Receiver<Delivery>,
    ) -> Result<(), AdapterError> {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!(adapter = %self.id, "stomp shutting down");
                    return Ok(());
                }
                incoming = reader.recv() => {
                    match incoming {
                        Ok(Some(frame)) => {
                            if let Err(err) = self.handle_frame(engine, map, frame).await {
                                warn!(adapter = %self.id, error = %err, "dropping inbound stomp frame");
                            }
                        }
                        Ok(None) => {
                            return Err(AdapterError::failed(&self.id, "broker closed the connection"));
                        }
                        Err(err) => {
                            return Err(AdapterError::failed(&self.id, err.to_string()));
                        }
                    }
                }
                delivery = eng_rx.recv() => {
                    let Some(delivery) = delivery else { return Ok(()); };
                    if let Err(err) = forward_outbound(self, writer, map, delivery).await {
                        return Err(AdapterError::failed(&self.id, err));
                    }
                }
            }
        }
    }

    async fn handle_frame(
        &self,
        engine: &Arc<Engine>,
        map: &DestinationMap,
        frame: Frame,
    ) -> Result<(), String> {
        match frame.command.as_str() {
            "MESSAGE" => {
                inbound_publish(self, engine, map, &frame).await?;
            }
            "ERROR" => {
                let msg = frame
                    .header("message")
                    .unwrap_or_else(|| std::str::from_utf8(&frame.body).unwrap_or("broker ERROR"));
                return Err(format!("broker ERROR: {msg}"));
            }
            "RECEIPT" => {}
            other => {
                debug!(command = other, "ignoring stomp frame");
            }
        }
        Ok(())
    }
}

async fn inbound_publish(
    adapter: &StompAdapter,
    engine: &Arc<Engine>,
    map: &DestinationMap,
    frame: &Frame,
) -> Result<(), String> {
    let dest = frame
        .header("destination")
        .ok_or_else(|| "MESSAGE missing destination".to_string())?;
    let topic = frame
        .header(HDR_TOPIC)
        .map(str::to_owned)
        .or_else(|| map.from_stomp(dest))
        .ok_or_else(|| format!("destination {dest} is outside {}", map.prefix()))?;

    let type_hint = frame
        .header(HDR_TYPE_HINT)
        .map(str::to_owned)
        .or_else(|| sniff_type_hint(&frame.body));
    let content_type = sniff_content_type(&frame.body, frame.header("content-type"));
    let route = inbound_route(&topic, type_hint);

    let stamp = |mut env: Envelope| {
        env.headers
            .insert(HDR_ORIGIN.into(), adapter.id.as_str().to_owned());
        env.headers.insert(HDR_STOMP_DEST.into(), dest.to_owned());
        if let Some(mid) = frame.header("message-id") {
            env.headers
                .insert("stomp.message-id".into(), mid.to_owned());
        }
        env
    };

    if adapter.config.unwrap_ma_payloads && wrapper_kind(&frame.body).is_some() {
        let peeled = unwrap_ma(&topic, &frame.body).map_err(|e| e.to_string())?;
        engine.publish(stamp(peeled.wrapper)).await;
        engine.publish(stamp(peeled.inner)).await;
        return Ok(());
    }

    let envelope = Envelope::new(route, bytes::Bytes::copy_from_slice(&frame.body))
        .with_content_type(content_type);
    engine.publish(stamp(envelope)).await;
    Ok(())
}

async fn forward_outbound(
    adapter: &StompAdapter,
    writer: &mut FrameWriter,
    map: &DestinationMap,
    delivery: Delivery,
) -> Result<(), String> {
    if delivery
        .envelope
        .headers
        .get(HDR_ORIGIN)
        .map(String::as_str)
        == Some(adapter.id.as_str())
    {
        return Ok(());
    }

    let dest = map.to_stomp(&delivery.envelope.route.topic);
    let mut headers = vec![
        (
            "content-type".into(),
            delivery.envelope.content_type.as_str().to_owned(),
        ),
        (HDR_ORIGIN.into(), adapter.id.as_str().to_owned()),
        (HDR_TOPIC.into(), delivery.envelope.route.topic.clone()),
        (HDR_ID.into(), delivery.envelope.id.to_string()),
    ];
    if let Some(hint) = &delivery.envelope.route.type_hint {
        headers.push((HDR_TYPE_HINT.into(), hint.clone()));
    }
    for (k, v) in &delivery.envelope.headers {
        if k == HDR_ORIGIN || k == HDR_STOMP_DEST || k.starts_with("stomp.") {
            continue;
        }
        headers.push((k.clone(), v.clone()));
    }

    writer
        .send(&send_frame(
            &dest,
            headers,
            delivery.envelope.payload.to_vec(),
        ))
        .await
        .map_err(|e| e.to_string())
}

#[async_trait]
impl Adapter for StompAdapter {
    fn id(&self) -> &AdapterId {
        StompAdapter::id(self)
    }

    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        self.serve(engine, shutdown).await
    }
}
