//! ActiveMQ STOMP client adapter. Protocol control stays here; data crosses the engine.
//!
//! This is a client, not a server. The retry loop sits outside one
//! broker session. Each session runs on a child task so a panic is a
//! join error. [`StompConfig::on_panic`] chooses abort or reconnect.
//! The host does not restart a finished `run`.

use std::sync::Arc;

use async_trait::async_trait;
use oa_gateway_adapter::{Adapter, AdapterError};
use oa_gateway_agra::{unwrap as unwrap_ma, wrapper_kind};
use oa_gateway_core::{
    AdapterId, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::client::{
    connect, disconnect_frame, send_frame, subscribe_frame, FrameReader, FrameWriter,
};
use crate::codec::Frame;
use crate::config::{OnPanic, StompConfig};
use crate::dest::{
    inbound_route, sniff_content_type, sniff_type_hint, DestinationMap, HDR_ID, HDR_ORIGIN,
    HDR_STOMP_DEST, HDR_TOPIC, HDR_TYPE_HINT,
};

/// STOMP client that bridges configured topics both ways.
///
/// Inbound MESSAGE frames are stamped with `oag.origin_adapter` so
/// outbound SEND can refuse the echo when
/// [`StompConfig::suppress_echo`] is on. [`Engine::drop_adapter`] runs
/// at session start and end so a reconnect does not keep stale
/// subscriptions.
pub struct StompAdapter {
    id: AdapterId,
    config: StompConfig,
}

impl StompAdapter {
    /// Builds an adapter that is not yet connected.
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

    /// Connects, subscribes, and bridges until `shutdown` or a fatal
    /// session error with reconnect off.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Failed`] if connect, SUBSCRIBE, or the
    /// session fails and [`StompConfig::reconnect`] is false; if a
    /// session panics and [`StompConfig::on_panic`] is `abort`; or if
    /// shutdown has already fired.
    pub async fn serve(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        self.serve_inner(engine, shutdown, None).await
    }

    /// Same as [`Self::serve`], then signals `ready` after the first
    /// CONNECTED and engine subscriptions.
    ///
    /// Used by tests that must not publish before the bridge is up.
    /// Later reconnects do not signal again.
    ///
    /// # Errors
    ///
    /// Same as [`Self::serve`].
    pub async fn serve_ready(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
        ready: oneshot::Sender<()>,
    ) -> Result<(), AdapterError> {
        self.serve_inner(engine, shutdown, Some(ready)).await
    }

    /// Retry loop. Delay comes from [`StompConfig::reconnect_delay`].
    /// `ready` is taken on the first session only.
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
            let adapter = Arc::clone(&self);
            let engine = Arc::clone(&engine);
            let token = shutdown.clone();
            let ready = ready.take();
            let joined =
                tokio::spawn(async move { adapter.session(&engine, &token, ready).await }).await;
            match after_join(
                joined,
                self.config.reconnect,
                self.config.on_panic,
                &self.id,
            ) {
                AfterSession::ReturnOk => return Ok(()),
                AfterSession::ReturnErr(err) => return Err(err),
                AfterSession::Retry { message } => {
                    if shutdown.is_cancelled() {
                        return Ok(());
                    }
                    warn!(adapter = %self.id, "{message}");
                }
            }
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(self.config.reconnect_delay) => {}
            }
        }
    }

    /// One CONNECT through DISCONNECT.
    ///
    /// Drops this adapter's engine subscriptions before SUBSCRIBE so a
    /// previous session cannot leave keys behind. Engine subscribe uses
    /// [`RouteKey::topic`] (wildcard) for each configured topic.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Failed`] if the broker cannot be reached,
    /// SUBSCRIBE fails, or [`Self::drive_session`] fails.
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

    /// Reads broker frames and engine deliveries until shutdown or a
    /// broken connection.
    ///
    /// A clean broker close is an error so reconnect can fire. A
    /// dropped engine channel is [`Ok`].
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Failed`] if the broker closes, a read
    /// fails, or an outbound SEND cannot be written.
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

    /// Handles one inbound frame. MESSAGE is published; ERROR fails the
    /// session; RECEIPT is ignored.
    ///
    /// # Errors
    ///
    /// Returns a message for a broker ERROR or a MESSAGE that cannot be
    /// mapped onto an envelope.
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

/// Publishes a MESSAGE onto the engine.
///
/// Topic comes from `oag.topic` or by stripping the destination prefix.
/// Type hint comes from `oag.type_hint` or by sniffing the body. The
/// envelope is stamped with this adapter's id so the outbound path can
/// skip it. A-GRA wrappers publish wrapper and inner as two envelopes.
///
/// # Errors
///
/// Returns a message if `destination` is missing, outside the prefix,
/// or unwrap fails.
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

    let stamp = |env: Envelope| {
        let mut env = env.with_origin(&adapter.id);
        env.headers.insert(HDR_STOMP_DEST.into(), dest.to_owned());
        if let Some(mid) = frame.header("message-id") {
            env.headers
                .insert("stomp.message-id".into(), mid.to_owned());
        }
        env
    };

    if adapter.config.unwrap_ma_payloads && wrapper_kind(&frame.body).is_some() {
        let peeled = unwrap_ma(&topic, &frame.body).map_err(|e| e.to_string())?;
        log_drops(adapter, engine.publish(stamp(peeled.wrapper)).await);
        log_drops(adapter, engine.publish(stamp(peeled.inner)).await);
        return Ok(());
    }

    let envelope = Envelope::new(route, bytes::Bytes::copy_from_slice(&frame.body))
        .with_content_type(content_type);
    log_drops(adapter, engine.publish(stamp(envelope)).await);
    Ok(())
}

/// Writes a SEND for one engine delivery.
///
/// Skips envelopes whose `oag.origin_adapter` is this adapter, so a
/// MESSAGE just taken from the broker is not sent back. `stomp.*`
/// headers and `oag.origin_adapter` are not copied onto the wire.
///
/// # Errors
///
/// Returns a message if the SEND cannot be written.
async fn forward_outbound(
    adapter: &StompAdapter,
    writer: &mut FrameWriter,
    map: &DestinationMap,
    delivery: Delivery,
) -> Result<(), String> {
    if adapter.config.suppress_echo && delivery.envelope.is_echo_of(&adapter.id) {
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

    /// Same as [`StompAdapter::serve`].
    ///
    /// # Errors
    ///
    /// See [`StompAdapter::serve`].
    async fn run(
        self: Arc<Self>,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        self.serve(engine, shutdown).await
    }
}

fn log_drops(adapter: &StompAdapter, outcome: oa_gateway_core::PublishOutcome) {
    if outcome.dropped > 0 {
        warn!(
            adapter = %adapter.id,
            matched = outcome.matched,
            delivered = outcome.delivered,
            dropped = outcome.dropped,
            "engine dropped deliveries on this publish"
        );
    }
}

/// What the retry loop does after a session task ends.
#[derive(Debug)]
enum AfterSession {
    ReturnOk,
    ReturnErr(AdapterError),
    Retry { message: String },
}

/// Maps a session join result onto abort, return, or retry.
fn after_join(
    joined: Result<Result<(), AdapterError>, tokio::task::JoinError>,
    reconnect: bool,
    on_panic: OnPanic,
    adapter: &AdapterId,
) -> AfterSession {
    match joined {
        Ok(Ok(())) => {
            if reconnect {
                AfterSession::Retry {
                    message: "stomp session ended, reconnecting".into(),
                }
            } else {
                AfterSession::ReturnOk
            }
        }
        Ok(Err(err)) => {
            if reconnect {
                AfterSession::Retry {
                    message: format!("stomp session failed, retrying: {err}"),
                }
            } else {
                AfterSession::ReturnErr(err)
            }
        }
        Err(join) if join.is_panic() => {
            error!(adapter = %adapter, "stomp session panicked");
            match (on_panic, reconnect) {
                (OnPanic::Abort, _) | (OnPanic::Reconnect, false) => {
                    AfterSession::ReturnErr(AdapterError::failed(adapter, "session panicked"))
                }
                (OnPanic::Reconnect, true) => AfterSession::Retry {
                    message: "stomp session panicked, retrying".into(),
                },
            }
        }
        Err(_) => AfterSession::ReturnOk,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn panic_aborts_even_when_reconnect_is_on() {
        let joined = tokio::spawn(async { panic!("session boom") }).await;
        match after_join(joined, true, OnPanic::Abort, &AdapterId::new("stomp")) {
            AfterSession::ReturnErr(err) => {
                assert!(err.to_string().contains("session panicked"), "{err}");
            }
            other => panic!("expected abort, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn panic_retries_when_on_panic_is_reconnect() {
        let joined = tokio::spawn(async { panic!("session boom") }).await;
        match after_join(joined, true, OnPanic::Reconnect, &AdapterId::new("stomp")) {
            AfterSession::Retry { message } => {
                assert!(message.contains("panicked"), "{message}");
            }
            other => panic!("expected retry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn panic_reconnect_still_stops_when_reconnect_is_off() {
        let joined = tokio::spawn(async { panic!("session boom") }).await;
        match after_join(joined, false, OnPanic::Reconnect, &AdapterId::new("stomp")) {
            AfterSession::ReturnErr(_) => {}
            other => panic!("expected err, got {other:?}"),
        }
    }
}
