//! OWP/WebSocket adapter. Protocol control stays here; data crosses the engine.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use oa_gateway_adapter::AdapterError;
use oa_gateway_agra::{unwrap as unwrap_ma, wrapper_kind};
use oa_gateway_core::{
    AdapterId, ContentType, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
};
use oa_gateway_uci::Schema;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_hdr_async_with_config, WebSocketStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::codec::{
    parse_client, type_hint_from_json, ClientOp, Identifiers, InfoPayload, InitPayload, OwpError,
    ServerOp,
};

const OWP_VERSION: &str = "1.0";

/// Largest OWP frame accepted from a client, in bytes.
///
/// Matches the STOMP adapter's default so both edges of the gateway agree on
/// what counts as too big, and replaces the WebSocket library's far larger
/// default, which was the only ceiling before.
pub const DEFAULT_MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Concurrent connections accepted before further ones are refused.
pub const DEFAULT_MAX_CONNECTIONS: usize = 256;

/// Subscriptions allowed on one connection.
///
/// Comfortably above the number of messages in the UCI catalog, so a client
/// subscribing to every type in the standard still fits.
pub const DEFAULT_MAX_SUBSCRIPTIONS: usize = 1024;

#[derive(Debug, Clone)]
pub struct OwpConfig {
    pub bind: SocketAddr,
    pub server_id: String,
    pub system_label: String,
    /// When set, INIT.schema must match exactly.
    pub schema: Option<String>,
    pub system_uuid: String,
    /// Peel A-GRA Rx/Tx hex wrappers on PUB and fan out wrapper + inner.
    pub unwrap_ma_payloads: bool,
    /// Convert OMS JSON ↔ UCI XML at the socket. Engine / ASB see XML.
    pub xml_baseline: bool,
    /// Largest frame accepted from a client. Oversized frames end the session.
    pub max_frame_size: usize,
    /// Connections served at once. Further ones are closed on accept.
    pub max_connections: usize,
    /// Subscriptions one connection may hold.
    pub max_subscriptions: usize,
}

impl Default for OwpConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:9000".parse().expect("static addr"),
            server_id: "oa-gateway-0".into(),
            system_label: "OA-Gateway Prototype".into(),
            schema: Some("002.5.0".into()),
            system_uuid: uuid::Uuid::new_v4().to_string(),
            unwrap_ma_payloads: true,
            xml_baseline: false,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_subscriptions: DEFAULT_MAX_SUBSCRIPTIONS,
        }
    }
}

pub struct OwpAdapter {
    id: AdapterId,
    config: OwpConfig,
    conn_seq: AtomicU64,
    schema: Option<Arc<Schema>>,
    /// One permit per allowed connection, held for the life of the session.
    connections: Arc<Semaphore>,
    /// Set while connections are being refused, so saturation is logged on the
    /// way in and on the way out instead of once per rejected connection.
    at_capacity: AtomicBool,
}

impl OwpAdapter {
    #[must_use]
    pub fn new(id: impl Into<AdapterId>, config: OwpConfig) -> Self {
        let connections = Arc::new(Semaphore::new(config.max_connections));
        Self {
            id: id.into(),
            config,
            conn_seq: AtomicU64::new(1),
            schema: None,
            connections,
            at_capacity: AtomicBool::new(false),
        }
    }

    /// Supply the UCI schema used to convert between OMS JSON and UCI XML.
    ///
    /// Without one the adapter still routes, but it cannot convert: XML payloads
    /// keep their topic as the type hint and are forwarded verbatim. A schema is
    /// mandatory for [`OwpConfig::xml_baseline`], which the host enforces at
    /// startup so the failure surfaces before any traffic arrives.
    #[must_use]
    pub fn with_schema(mut self, schema: Arc<Schema>) -> Self {
        self.schema = Some(schema);
        self
    }

    #[must_use]
    pub fn id(&self) -> &AdapterId {
        &self.id
    }

    #[must_use]
    pub fn config(&self) -> &OwpConfig {
        &self.config
    }

    pub async fn serve(
        self: Arc<Self>,
        listener: TcpListener,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        let local = listener.local_addr()?;
        info!(%local, adapter = %self.id, "owp listening");

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!(adapter = %self.id, "owp shutting down");
                    return Ok(());
                }
                accepted = listener.accept() => {
                    let (stream, peer) = match accepted {
                        Ok(v) => v,
                        Err(err) => {
                            warn!(error = %err, "accept failed");
                            continue;
                        }
                    };
                    // Refuse rather than queue: a caller that cannot get a slot
                    // learns immediately, and the accept loop keeps draining so
                    // the backlog does not become the queue instead.
                    let Ok(permit) = Arc::clone(&self.connections).try_acquire_owned() else {
                        if !self.at_capacity.swap(true, Ordering::Relaxed) {
                            warn!(
                                adapter = %self.id,
                                limit = self.config.max_connections,
                                "at the connection limit, refusing new connections"
                            );
                        }
                        drop(stream);
                        continue;
                    };
                    if self.at_capacity.swap(false, Ordering::Relaxed) {
                        info!(adapter = %self.id, "below the connection limit, accepting again");
                    }
                    let conn_id = self.conn_seq.fetch_add(1, Ordering::Relaxed);
                    let this = Arc::clone(&self);
                    let engine = Arc::clone(&engine);
                    let shutdown = shutdown.clone();
                    tokio::spawn(async move {
                        if let Err(err) = this.handle_connection(stream, peer, conn_id, engine, shutdown).await {
                            debug!(%peer, conn_id, error = %err, "owp connection ended");
                        }
                        drop(permit);
                    });
                }
            }
        }
    }

    async fn handle_connection(
        &self,
        stream: TcpStream,
        peer: SocketAddr,
        conn_id: u64,
        engine: Arc<Engine>,
        shutdown: CancellationToken,
    ) -> Result<(), AdapterError> {
        // A frame over the cap is a protocol error the library reports on read,
        // which ends the session — the same outcome as an unparseable STOMP
        // frame, and it happens before the payload is fully buffered.
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(self.config.max_frame_size))
            .max_frame_size(Some(self.config.max_frame_size));
        let ws =
            match accept_hdr_async_with_config(stream, check_subprotocol, Some(ws_config)).await {
                Ok(ws) => ws,
                Err(err) => {
                    debug!(%peer, error = %err, "websocket handshake failed");
                    return Ok(());
                }
            };
        run_session(Session {
            adapter_id: self.id.clone(),
            config: self.config.clone(),
            schema: self.schema.clone(),
            conn_id,
            ws,
            engine,
            shutdown,
        })
        .await
    }
}

#[allow(clippy::result_large_err)]
fn check_subprotocol(req: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
    let has_owp = req
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| {
            s.split(',')
                .map(str::trim)
                .any(|p| p.eq_ignore_ascii_case("owp"))
        });
    if !has_owp {
        let mut err = ErrorResponse::new(Some("missing owp subprotocol".into()));
        *err.status_mut() = StatusCode::BAD_REQUEST;
        return Err(err);
    }
    response
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_static("owp"));
    Ok(response)
}

struct Session {
    adapter_id: AdapterId,
    config: OwpConfig,
    schema: Option<Arc<Schema>>,
    conn_id: u64,
    ws: WebSocketStream<TcpStream>,
    engine: Arc<Engine>,
    shutdown: CancellationToken,
}

enum State {
    AwaitingInit,
    Active { verbose: bool, service_id: String },
}

struct LiveSub {
    engine_sub: SubId,
    forwarder: JoinHandle<()>,
}

/// Whether an unroutable publish is worth reporting yet.
enum Report {
    /// Not seen on this connection before.
    New,
    /// As [`Report::New`], and the last one this connection will report.
    Final,
    /// Already reported, or reporting has stopped.
    Skip,
}

/// Routes this connection has been warned about publishing into thin air.
///
/// A client publishing where nothing is subscribed rarely does it once, so
/// reporting every message would bury the log. Reporting also stops past
/// [`Unroutable::CAP`] distinct routes, so a client cycling through topics
/// cannot turn the warning into unbounded memory or log volume.
#[derive(Default)]
struct Unroutable {
    seen: HashSet<RouteKey>,
}

impl Unroutable {
    const CAP: usize = 64;

    fn report(&mut self, route: &RouteKey) -> Report {
        if self.seen.len() >= Self::CAP || self.seen.contains(route) {
            return Report::Skip;
        }
        self.seen.insert(route.clone());
        if self.seen.len() == Self::CAP {
            Report::Final
        } else {
            Report::New
        }
    }
}

async fn run_session(mut session: Session) -> Result<(), AdapterError> {
    let (out_tx, mut out_rx) = mpsc::channel::<ServerOp>(DEFAULT_CHANNEL_CAPACITY);
    let mut state = State::AwaitingInit;
    let mut subs: HashMap<String, LiveSub> = HashMap::new();
    let mut unroutable = Unroutable::default();

    loop {
        tokio::select! {
            _ = session.shutdown.cancelled() => break,
            outgoing = out_rx.recv() => {
                let Some(op) = outgoing else { break };
                if session.ws.send(Message::Text(op.to_string().into())).await.is_err() {
                    break;
                }
            }
            incoming = session.ws.next() => {
                let Some(frame) = incoming else { break };
                let msg = match frame {
                    Ok(m) => m,
                    Err(_) => break,
                };
                match msg {
                    Message::Text(text) => {
                        if handle_text(
                            &session,
                            &mut state,
                            &mut subs,
                            &mut unroutable,
                            &out_tx,
                            text.as_str(),
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    Message::Ping(data) => {
                        let _ = session.ws.send(Message::Pong(data)).await;
                    }
                    Message::Pong(_) | Message::Frame(_) => {}
                    Message::Binary(_) => {
                        let _ = out_tx
                            .send(err_op(OwpError::IllegalOperation, Some("binary frames are not allowed")))
                            .await;
                        break;
                    }
                    Message::Close(_) => break,
                }
            }
        }
    }

    for (_, live) in subs.drain() {
        live.forwarder.abort();
        let _ = session
            .engine
            .unsubscribe(session.adapter_id.clone(), live.engine_sub)
            .await;
    }
    Ok(())
}

enum Fatal {
    Close,
}

async fn handle_text(
    session: &Session,
    state: &mut State,
    subs: &mut HashMap<String, LiveSub>,
    unroutable: &mut Unroutable,
    out_tx: &mpsc::Sender<ServerOp>,
    text: &str,
) -> Result<(), Fatal> {
    let op = match parse_client(text) {
        Ok(op) => op,
        Err(err) => {
            send(
                out_tx,
                err_op(OwpError::IllegalArgument, Some(err.to_string().as_str())),
            )
            .await;
            return Ok(());
        }
    };

    match (&*state, op) {
        (State::AwaitingInit, ClientOp::Init(init)) => match negotiate(&session.config, &init) {
            Ok(()) => {
                let verbose = init.verbose.unwrap_or(true);
                if verbose {
                    send(out_tx, ServerOp::Ok).await;
                }
                send(out_tx, ServerOp::Info(info_payload(&session.config, &init))).await;
                *state = State::Active {
                    verbose,
                    service_id: init.service_id,
                };
            }
            Err(error) => {
                send(out_tx, err_op(error, None)).await;
                return Err(Fatal::Close);
            }
        },
        (State::AwaitingInit, _) => {
            send(
                out_tx,
                err_op(
                    OwpError::IllegalState,
                    Some("INIT must be the first operation"),
                ),
            )
            .await;
            return Err(Fatal::Close);
        }
        (State::Active { .. }, ClientOp::Init(_)) => {
            send(
                out_tx,
                err_op(OwpError::IllegalState, Some("duplicate INIT")),
            )
            .await;
            return Err(Fatal::Close);
        }
        (
            State::Active {
                verbose,
                service_id,
            },
            ClientOp::Pub { topic, payload },
        ) => match publish_owp(session, unroutable, service_id, topic, payload).await {
            Ok(()) => {
                if *verbose {
                    send(out_tx, ServerOp::Ok).await;
                }
            }
            Err(err) => {
                send(out_tx, err_op(OwpError::InvalidMessage, Some(&err))).await;
            }
        },
        (
            State::Active {
                verbose,
                service_id: _,
            },
            ClientOp::Sub {
                sid,
                message_name,
                topic,
                group: _,
            },
        ) => {
            if subs.contains_key(&sid) {
                send(
                    out_tx,
                    err_op(OwpError::IllegalArgument, Some("duplicate sid")),
                )
                .await;
                return Ok(());
            }
            if subs.len() >= session.config.max_subscriptions {
                // Each subscription costs a channel, a task, and an engine index
                // entry keyed by client-supplied strings, so the count is bounded
                // per connection. The protocol has no resource-limit code, and
                // Illegal-State is the closest of the ones it defines.
                warn!(
                    adapter = %session.adapter_id,
                    conn_id = session.conn_id,
                    limit = session.config.max_subscriptions,
                    "subscription limit reached"
                );
                send(
                    out_tx,
                    err_op(OwpError::IllegalState, Some("subscription limit reached")),
                )
                .await;
                return Ok(());
            }
            let engine_sub = SubId::new(format!("{}:{sid}", session.conn_id));
            let (tx, mut rx) = mpsc::channel::<Delivery>(DEFAULT_CHANNEL_CAPACITY);
            if session
                .engine
                .subscribe(
                    session.adapter_id.clone(),
                    engine_sub.clone(),
                    RouteKey::typed(topic, message_name),
                    tx,
                )
                .await
                .is_err()
            {
                send(
                    out_tx,
                    err_op(OwpError::InternalError, Some("subscribe failed")),
                )
                .await;
                return Ok(());
            }
            let forward_tx = out_tx.clone();
            let local_sid = sid.clone();
            let xml_baseline = session.config.xml_baseline;
            let schema = session.schema.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(delivery) = rx.recv().await {
                    let Ok(raw) = String::from_utf8(delivery.envelope.payload.to_vec()) else {
                        warn!("dropping non-utf8 payload destined for OWP");
                        continue;
                    };
                    let payload = if xml_baseline {
                        xml_to_oms_json(&raw, schema.as_deref())
                    } else {
                        raw
                    };
                    if forward_tx
                        .send(ServerOp::Msg {
                            sid: local_sid.clone(),
                            payload,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
            subs.insert(
                sid,
                LiveSub {
                    engine_sub,
                    forwarder,
                },
            );
            if *verbose {
                send(out_tx, ServerOp::Ok).await;
            }
        }
        (State::Active { verbose, .. }, ClientOp::Unsub { sid }) => {
            if let Some(live) = subs.remove(&sid) {
                live.forwarder.abort();
                let _ = session
                    .engine
                    .unsubscribe(session.adapter_id.clone(), live.engine_sub)
                    .await;
                if *verbose {
                    send(out_tx, ServerOp::Ok).await;
                }
            } else {
                send(
                    out_tx,
                    err_op(OwpError::IllegalArgument, Some("unknown sid")),
                )
                .await;
            }
        }
    }
    Ok(())
}

async fn publish_owp(
    session: &Session,
    unroutable: &mut Unroutable,
    service_id: &str,
    topic: String,
    payload: String,
) -> Result<(), String> {
    let stamp = |mut env: Envelope| {
        env.headers
            .insert("owp.service_id".into(), service_id.to_owned());
        env.headers
            .insert("owp.conn_id".into(), session.conn_id.to_string());
        env
    };

    let mut outgoing = Vec::new();
    if session.config.unwrap_ma_payloads && wrapper_kind(payload.as_bytes()).is_some() {
        let peeled = unwrap_ma(&topic, payload.as_bytes()).map_err(|e| e.to_string())?;
        outgoing.push(peeled.wrapper);
        outgoing.push(peeled.inner);
    } else {
        let hint = if oa_gateway_uci::looks_like_xml(payload.as_bytes()) {
            // Without a schema the element name cannot be read reliably, so the
            // topic stands in — the same fallback used when conversion fails.
            session
                .schema
                .as_deref()
                .and_then(|schema| oa_gateway_uci::Message::from_xml(&payload, schema).ok())
                .map(|m| m.name)
                .unwrap_or_else(|| topic.clone())
        } else {
            type_hint_from_json(&payload).map_err(|e| e.to_string())?
        };
        let ct = if oa_gateway_uci::looks_like_xml(payload.as_bytes()) {
            ContentType::xml()
        } else {
            ContentType::json()
        };
        outgoing.push(
            Envelope::new(RouteKey::typed(topic, hint), payload.into_bytes()).with_content_type(ct),
        );
    }

    for env in outgoing {
        let env = if session.config.xml_baseline {
            let schema = session.schema.as_deref().ok_or_else(|| {
                "owp.xml_baseline is enabled but no UCI schema is loaded".to_string()
            })?;
            toward_xml(env, schema)?
        } else {
            env
        };
        let env = stamp(env);
        let route = env.route.clone();
        // A publish that matches nothing is legal pub/sub, but it is far more
        // often a topic the gateway was never configured to carry — a STOMP
        // topics list without this entry, say — and the client is told "+OK"
        // either way. Say so here rather than let the message vanish.
        if session.engine.publish(env).await.matched == 0 {
            match unroutable.report(&route) {
                Report::Skip => {}
                report => {
                    warn!(
                        adapter = %session.adapter_id,
                        service = %service_id,
                        route = %route,
                        "nothing is subscribed to this route, so the publish went nowhere"
                    );
                    if matches!(report, Report::Final) {
                        warn!(
                            adapter = %session.adapter_id,
                            service = %service_id,
                            "reached the unroutable route limit; no more will be reported \
                             on this connection"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn toward_xml(mut env: Envelope, schema: &Schema) -> Result<Envelope, String> {
    if oa_gateway_uci::looks_like_xml(&env.payload) {
        env.content_type = ContentType::xml();
        return Ok(env);
    }
    let text = std::str::from_utf8(&env.payload).map_err(|e| e.to_string())?;
    let msg = oa_gateway_uci::Message::from_json(text, schema).map_err(|e| e.to_string())?;
    env.route.type_hint = Some(msg.name.clone());
    env.payload = bytes::Bytes::from(msg.to_xml(schema).map_err(|e| e.to_string())?);
    env.content_type = ContentType::xml();
    Ok(env)
}

fn xml_to_oms_json(raw: &str, schema: Option<&Schema>) -> String {
    if !oa_gateway_uci::looks_like_xml(raw.as_bytes()) {
        return raw.to_owned();
    }
    let Some(schema) = schema else {
        return raw.to_owned();
    };
    match oa_gateway_uci::Message::from_xml(raw, schema).and_then(|m| m.to_json(schema)) {
        Ok(json) => json,
        Err(err) => {
            warn!(error = %err, "xml→json failed; forwarding XML to OWP client");
            raw.to_owned()
        }
    }
}

fn negotiate(config: &OwpConfig, init: &InitPayload) -> Result<(), OwpError> {
    if !init.versions.iter().any(|v| v == OWP_VERSION) {
        return Err(OwpError::UnsupportedVersion);
    }
    if let Some(expected) = &config.schema {
        if &init.schema != expected {
            return Err(OwpError::UnsupportedSchema);
        }
    }
    Ok(())
}

fn info_payload(config: &OwpConfig, init: &InitPayload) -> InfoPayload {
    let service = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, init.service_id.as_bytes());
    InfoPayload {
        version: OWP_VERSION.into(),
        server_id: config.server_id.clone(),
        uuids: Identifiers {
            system: config.system_uuid.clone(),
            service: service.to_string(),
            subsystem: None,
        },
        system_label: config.system_label.clone(),
    }
}

fn err_op(error: OwpError, details: Option<&str>) -> ServerOp {
    ServerOp::Err {
        error,
        details: details.map(str::to_owned),
    }
}

async fn send(tx: &mpsc::Sender<ServerOp>, op: ServerOp) {
    let _ = tx.send(op).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_is_reported_once() {
        let mut unroutable = Unroutable::default();
        let route = RouteKey::typed("SystemStatus", "SystemStatus");

        assert!(matches!(unroutable.report(&route), Report::New));
        assert!(matches!(unroutable.report(&route), Report::Skip));
        assert!(matches!(unroutable.report(&route), Report::Skip));

        // A different type on the same topic is a different route.
        let other = RouteKey::typed("SystemStatus", "PositionReport");
        assert!(matches!(unroutable.report(&other), Report::New));
    }

    #[test]
    fn reporting_stops_at_the_cap() {
        let mut unroutable = Unroutable::default();
        for i in 0..Unroutable::CAP - 1 {
            let route = RouteKey::typed(format!("topic-{i}"), "Ping");
            assert!(matches!(unroutable.report(&route), Report::New));
        }

        let last = RouteKey::typed("topic-last", "Ping");
        assert!(matches!(unroutable.report(&last), Report::Final));

        // Nothing further is reported or remembered, however many routes arrive.
        for i in 0..1000 {
            let route = RouteKey::typed(format!("flood-{i}"), "Ping");
            assert!(matches!(unroutable.report(&route), Report::Skip));
        }
        assert_eq!(unroutable.seen.len(), Unroutable::CAP);
    }
}
