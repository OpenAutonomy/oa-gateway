//! OWP/WebSocket adapter. Protocol control stays here; data crosses the engine.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use oa_gateway_adapter::AdapterError;
use oa_gateway_agra::{unwrap as unwrap_ma, wrapper_kind, xml_root_local_name};
use oa_gateway_core::{
    AdapterId, ContentType, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
};
use oa_gateway_uci::validate::{summarize, Mode as ValidateMode, Violation};
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
    /// What to do about a payload that does not follow the loaded schema.
    /// Has no effect without one: there is nothing to check against.
    pub validate: ValidateMode,
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
            validate: ValidateMode::default(),
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

/// Routes this connection has already been warned about.
///
/// A client that publishes into thin air, or sends a payload the schema does not
/// permit, rarely does it once, so reporting every message would bury the log.
/// Reporting also stops past [`SeenRoutes::CAP`] distinct routes, so a client
/// cycling through topics cannot turn a warning into unbounded memory or log
/// volume. One tracker per kind of warning, so a noisy route of one kind does not
/// hide the other.
#[derive(Default)]
struct SeenRoutes {
    seen: HashSet<RouteKey>,
}

impl SeenRoutes {
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
    let mut unroutable = SeenRoutes::default();
    let mut invalid = SeenRoutes::default();

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
                            &mut invalid,
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
    unroutable: &mut SeenRoutes,
    invalid: &mut SeenRoutes,
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
        ) => match publish_owp(session, unroutable, invalid, service_id, topic, payload).await {
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
            let adapter_id = session.adapter_id.clone();
            let validate = session.config.validate;
            let forwarder = tokio::spawn(async move {
                // The client is told about every dropped delivery, since each is
                // a message it will not receive; the log is told once, since the
                // cause is the same for every payload on this route.
                let mut logged = false;
                let mut logged_invalid = false;
                while let Some(delivery) = rx.recv().await {
                    let Ok(raw) = String::from_utf8(delivery.envelope.payload.to_vec()) else {
                        warn!("dropping non-utf8 payload destined for OWP");
                        continue;
                    };

                    // What arrived off the bus, before any conversion of ours,
                    // so a violation is attributed to the producer.
                    let violations = violations_of(raw.as_bytes(), schema.as_deref(), validate);
                    if !violations.is_empty() {
                        let summary = summarize(&violations);
                        if validate == ValidateMode::Reject {
                            if !logged_invalid {
                                logged_invalid = true;
                                warn!(
                                    adapter = %adapter_id,
                                    sid = %local_sid,
                                    violations = %summary,
                                    "dropping a delivery that does not follow the UCI schema; \
                                     later ones on this subscription are not logged"
                                );
                            }
                            let details = format!(
                                "delivery on {local_sid} does not follow the UCI schema: {summary}"
                            );
                            if forward_tx
                                .send(err_op(OwpError::InvalidMessage, Some(&details)))
                                .await
                                .is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        if !logged_invalid {
                            logged_invalid = true;
                            warn!(
                                adapter = %adapter_id,
                                sid = %local_sid,
                                violations = %summary,
                                "delivered payload does not follow the UCI schema; forwarding it \
                                 anyway, and later ones on this subscription are not logged"
                            );
                        }
                    }
                    let payload = if xml_baseline {
                        match xml_to_oms_json(&raw, schema.as_deref()) {
                            Ok(json) => json,
                            Err(err) => {
                                if !logged {
                                    logged = true;
                                    warn!(
                                        adapter = %adapter_id,
                                        sid = %local_sid,
                                        error = %err,
                                        "dropping a delivery that will not convert to JSON; \
                                         later failures on this subscription are not logged"
                                    );
                                }
                                // Forwarding the XML instead would hand the client
                                // a format it did not subscribe for, and it has no
                                // way to tell that from an ordinary payload.
                                let details = format!(
                                    "delivery on {local_sid} could not be converted: {err}"
                                );
                                if forward_tx
                                    .send(err_op(OwpError::InvalidMessage, Some(&details)))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                continue;
                            }
                        }
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
    unroutable: &mut SeenRoutes,
    invalid: &mut SeenRoutes,
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
            // The name is in the document, so no schema is needed to read it,
            // and this is how the STOMP edge names an XML payload too. The topic
            // used to stand in when the schema could not parse the payload,
            // which routed the message under a key no subscriber of that type
            // would match and reported +OK regardless.
            xml_root_local_name(&payload)
                .ok_or_else(|| "XML payload has no element to take a type from".to_string())?
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

    // Convert and check everything before publishing any of it: an A-GRA wrapper
    // and the message inside it are one publish as far as the client is
    // concerned, and half of one is worse than neither.
    let mut ready = Vec::with_capacity(outgoing.len());
    for env in outgoing {
        let env = if session.config.xml_baseline {
            let schema = session.schema.as_deref().ok_or_else(|| {
                "owp.xml_baseline is enabled but no UCI schema is loaded".to_string()
            })?;
            toward_xml(env, schema)?
        } else {
            env
        };

        let violations = violations_of(
            &env.payload,
            session.schema.as_deref(),
            session.config.validate,
        );
        if !violations.is_empty() {
            let summary = summarize(&violations);
            if session.config.validate == ValidateMode::Reject {
                return Err(format!("payload does not follow the UCI schema: {summary}"));
            }
            match invalid.report(&env.route) {
                Report::Skip => {}
                report => {
                    warn!(
                        adapter = %session.adapter_id,
                        service = %service_id,
                        route = %env.route,
                        violations = %summary,
                        "published payload does not follow the UCI schema; carrying it anyway"
                    );
                    cap_notice(report, &session.adapter_id, service_id);
                }
            }
        }

        ready.push(stamp(env));
    }

    for env in ready {
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
                    cap_notice(report, &session.adapter_id, service_id);
                }
            }
        }
    }
    Ok(())
}

/// Say when a tracker has stopped naming routes, so a quiet log is not mistaken
/// for a quiet client.
fn cap_notice(report: Report, adapter: &AdapterId, service_id: &str) {
    if matches!(report, Report::Final) {
        warn!(
            adapter = %adapter,
            service = %service_id,
            "reached the reporting limit for this connection; further routes of this \
             kind will not be named"
        );
    }
}

/// Everything in `payload` that `schema` does not permit.
///
/// Parses on its own account rather than reusing the conversion's parse. The two
/// run on different paths — conversion only when the XML baseline is on, this
/// whenever a schema is loaded and validation is not off — and one obvious check
/// is worth more than one saved parse. A payload that will not parse at all is a
/// conversion failure, reported where conversion happens, so nothing is said
/// about it twice.
fn violations_of(payload: &[u8], schema: Option<&Schema>, mode: ValidateMode) -> Vec<Violation> {
    if !mode.is_on() {
        return Vec::new();
    }
    let Some(schema) = schema else {
        return Vec::new();
    };
    let Ok(text) = std::str::from_utf8(payload) else {
        return Vec::new();
    };
    let parsed = if oa_gateway_uci::looks_like_xml(payload) {
        oa_gateway_uci::Message::from_xml(text, schema)
    } else {
        oa_gateway_uci::Message::from_json(text, schema)
    };
    parsed.map(|m| m.violations(schema)).unwrap_or_default()
}

fn toward_xml(mut env: Envelope, schema: &Schema) -> Result<Envelope, String> {
    if oa_gateway_uci::looks_like_xml(&env.payload) {
        env.content_type = ContentType::xml();
        return Ok(env);
    }
    let text = std::str::from_utf8(&env.payload).map_err(|e| e.to_string())?;
    let text = transcode_wrapper_inner(text, schema, true)?;
    let msg = oa_gateway_uci::Message::from_json(&text, schema).map_err(|e| e.to_string())?;
    env.route.type_hint = Some(msg.name.clone());
    env.payload = bytes::Bytes::from(msg.to_xml(schema).map_err(|e| e.to_string())?);
    env.content_type = ContentType::xml();
    Ok(env)
}

/// Convert a payload the engine carried in XML into the OMS JSON a client
/// subscribed for.
///
/// A payload that is not XML is already what the client asked for and passes
/// through. Anything else is either converted or refused: the caller drops the
/// delivery and says so, rather than forwarding a document in a format the
/// client has no way to distinguish from an expected one.
fn xml_to_oms_json(raw: &str, schema: Option<&Schema>) -> Result<String, String> {
    if !oa_gateway_uci::looks_like_xml(raw.as_bytes()) {
        return Ok(raw.to_owned());
    }
    // The host refuses to start with xml_baseline and no schema, so this is a
    // guard against an embedding that skips that check, not a reachable path.
    let schema = schema.ok_or("no UCI schema is loaded")?;
    let json = oa_gateway_uci::Message::from_xml(raw, schema)
        .and_then(|m| m.to_json(schema))
        .map_err(|e| e.to_string())?;
    transcode_wrapper_inner(&json, schema, false)
}

/// A-GRA `EncodedPayload` is opaque hex. OWP clients put OMS JSON in it; MA
/// parses XML. `xml_baseline` has to convert the inner the same way it
/// converts the wrapper, or MA logs `expected XML` and drops the path.
fn transcode_wrapper_inner(text: &str, schema: &Schema, want_xml: bool) -> Result<String, String> {
    let mut root: serde_json::Value =
        serde_json::from_str(text).map_err(|e| e.to_string())?;
    let obj = match root.as_object_mut() {
        Some(obj) if obj.len() == 1 => obj,
        _ => return Ok(text.to_owned()),
    };
    let name = obj.keys().next().cloned().unwrap_or_default();
    if name != oa_gateway_agra::RX_ELEMENT && name != oa_gateway_agra::TX_ELEMENT {
        return Ok(text.to_owned());
    }
    let Some(encoded) = obj
        .get_mut(&name)
        .and_then(|body| body.get_mut("MessageData"))
        .and_then(|data| data.get_mut("EncodedPayload"))
    else {
        return Ok(text.to_owned());
    };
    let Some(hex) = encoded.as_str() else {
        return Ok(text.to_owned());
    };
    let inner_bytes = decode_hex(hex)?;
    let inner_text = std::str::from_utf8(&inner_bytes).map_err(|e| e.to_string())?;
    let inner_is_xml = inner_text.trim_start().starts_with('<');
    if want_xml == inner_is_xml {
        return Ok(text.to_owned());
    }
    let converted = if want_xml {
        let msg = oa_gateway_uci::Message::from_json(inner_text, schema).map_err(|e| e.to_string())?;
        msg.to_xml(schema).map_err(|e| e.to_string())?
    } else {
        let msg = oa_gateway_uci::Message::from_xml(inner_text, schema).map_err(|e| e.to_string())?;
        msg.to_json(schema).map_err(|e| e.to_string())?
    };
    *encoded = serde_json::Value::String(encode_hex_upper(converted.as_bytes()));
    serde_json::to_string(&root).map_err(|e| e.to_string())
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if digits.len() % 2 != 0 {
        return Err("EncodedPayload is not hexBinary: odd number of digits".into());
    }
    let mut out = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks_exact(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        other => Err(format!(
            "EncodedPayload is not hexBinary: invalid character {:?}",
            char::from(other)
        )),
    }
}

fn encode_hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
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
        let mut unroutable = SeenRoutes::default();
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
        let mut unroutable = SeenRoutes::default();
        for i in 0..SeenRoutes::CAP - 1 {
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
        assert_eq!(unroutable.seen.len(), SeenRoutes::CAP);
    }

    fn hex_of(text: &str) -> String {
        encode_hex_upper(text.as_bytes())
    }

    #[test]
    fn xml_baseline_converts_json_encoded_payload_to_xml() {
        let inner = r#"{"PositionReport":{"MessageData":{"n":1}}}"#;
        let wrapper = format!(
            r#"{{"MA_RxDataPayload":{{"MessageData":{{"EncodedPayload":"{}","MessageType":"POSITION_REPORT"}}}}}}"#,
            hex_of(inner)
        );
        let schema = oa_gateway_uci::slice::v25();
        let converted = transcode_wrapper_inner(&wrapper, schema, true).unwrap();
        let hex = serde_json::from_str::<serde_json::Value>(&converted)
            .unwrap()
            .pointer("/MA_RxDataPayload/MessageData/EncodedPayload")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_owned();
        let inner_xml = String::from_utf8(decode_hex(&hex).unwrap()).unwrap();
        assert!(
            inner_xml.trim_start().starts_with('<'),
            "inner should be XML, got {inner_xml}"
        );
        assert!(inner_xml.contains("PositionReport"), "{inner_xml}");
    }

    #[test]
    fn xml_baseline_leaves_xml_encoded_payload_alone() {
        let inner = "<PositionReport><MessageData><n>1</n></MessageData></PositionReport>";
        let wrapper = format!(
            r#"{{"MA_RxDataPayload":{{"MessageData":{{"EncodedPayload":"{}"}}}}}}"#,
            hex_of(inner)
        );
        let schema = oa_gateway_uci::slice::v25();
        let converted = transcode_wrapper_inner(&wrapper, schema, true).unwrap();
        assert_eq!(converted, wrapper);
    }
}
