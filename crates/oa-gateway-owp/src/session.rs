//! One WebSocket session: INIT, then PUB / SUB / UNSUB until close.
//!
//! Binary frames are refused and the socket is closed. A parse error on
//! a text frame is `-ERR Illegal-Argument` and the session stays up.
//! A failed INIT closes it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use oa_gateway_adapter::tls::MaybeTlsStream;
use oa_gateway_adapter::AdapterError;
use oa_gateway_agra::{unwrap as unwrap_ma, wrapper_kind, xml_root_local_name};
use oa_gateway_core::{
    AdapterId, ContentType, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
};
use oa_gateway_uci::validate::{summarize, Mode as ValidateMode};
use oa_gateway_uci::Schema;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::codec::{
    parse_client, type_hint_from_json, ClientOp, Identifiers, InfoPayload, InitPayload, OwpError,
    ServerOp,
};
use crate::config::OwpConfig;
use crate::convert::{toward_xml, violations_of, xml_to_oms_json};

/// Protocol version this server speaks. INIT must list it.
const OWP_VERSION: &str = "1.0";

/// One accepted WebSocket and the engine handle it publishes through.
///
/// `conn_id` is unique per accept on this adapter. Engine subscription
/// ids are `{conn_id}:{sid}` so two connections can reuse a client sid.
pub(crate) struct Session {
    pub(crate) adapter_id: AdapterId,
    pub(crate) config: OwpConfig,
    pub(crate) schema: Option<Arc<Schema>>,
    pub(crate) conn_id: u64,
    pub(crate) ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pub(crate) engine: Arc<Engine>,
    pub(crate) shutdown: CancellationToken,
}

enum State {
    /// First client frame must be INIT.
    AwaitingInit,
    /// Handshake succeeded. `verbose` defaults to true when INIT omits it.
    Active { verbose: bool, service_id: String },
}

/// One client SUB: the engine key and the task that forwards MSG frames.
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

/// Unroutable publishes and invalid payloads, tracked separately.
#[derive(Default)]
struct RouteWarnings {
    unroutable: SeenRoutes,
    invalid: SeenRoutes,
}

impl SeenRoutes {
    /// Distinct routes named in the log before this tracker goes quiet.
    const CAP: usize = 64;

    /// Records `route` if it is new and under the cap.
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

/// End the WebSocket after an `-ERR` that the spec treats as fatal.
enum Fatal {
    Close,
}

/// Runs `session` until shutdown, close, or a fatal protocol error.
///
/// On the way out every live SUB is aborted and unsubscribed. This does
/// not call [`Engine::drop_adapter`]: other connections on the same
/// adapter must keep their subscriptions.
///
/// # Errors
///
/// Does not fail. Handshake already succeeded; I/O errors end the loop
/// as [`Ok`].
pub(crate) async fn run(mut session: Session) -> Result<(), AdapterError> {
    let (out_tx, mut out_rx) = mpsc::channel::<ServerOp>(DEFAULT_CHANNEL_CAPACITY);
    let mut state = State::AwaitingInit;
    let mut subs: HashMap<String, LiveSub> = HashMap::new();
    let mut warnings = RouteWarnings::default();

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
                            &mut warnings,
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

/// Dispatches one client text frame.
///
/// Unknown or malformed frames stay on the connection. INIT in the
/// wrong state, or a rejected INIT, returns [`Fatal::Close`].
///
/// `SUB` `group` is accepted by the codec and ignored here.
async fn handle_text(
    session: &Session,
    state: &mut State,
    subs: &mut HashMap<String, LiveSub>,
    warnings: &mut RouteWarnings,
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

    match state {
        State::AwaitingInit => match op {
            ClientOp::Init(init) => handle_init(session, state, out_tx, init).await,
            _ => {
                send(
                    out_tx,
                    err_op(
                        OwpError::IllegalState,
                        Some("INIT must be the first operation"),
                    ),
                )
                .await;
                Err(Fatal::Close)
            }
        },
        State::Active {
            verbose,
            service_id,
        } => match op {
            ClientOp::Init(_) => {
                send(
                    out_tx,
                    err_op(OwpError::IllegalState, Some("duplicate INIT")),
                )
                .await;
                Err(Fatal::Close)
            }
            ClientOp::Pub { topic, payload } => {
                handle_pub(
                    session, warnings, out_tx, *verbose, service_id, topic, payload,
                )
                .await
            }
            ClientOp::Sub {
                sid,
                message_name,
                topic,
                group: _,
            } => handle_sub(session, subs, out_tx, *verbose, sid, message_name, topic).await,
            ClientOp::Unsub { sid } => handle_unsub(session, subs, out_tx, *verbose, sid).await,
        },
    }
}

/// Negotiates INIT. Success sends INFO (and +OK when verbose).
async fn handle_init(
    session: &Session,
    state: &mut State,
    out_tx: &mpsc::Sender<ServerOp>,
    init: InitPayload,
) -> Result<(), Fatal> {
    match negotiate(&session.config, &init) {
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
            Ok(())
        }
        Err(error) => {
            send(out_tx, err_op(error, None)).await;
            Err(Fatal::Close)
        }
    }
}

/// Publishes onto the engine. A bad payload is `-ERR Invalid-Message`
/// and the session stays up.
async fn handle_pub(
    session: &Session,
    warnings: &mut RouteWarnings,
    out_tx: &mpsc::Sender<ServerOp>,
    verbose: bool,
    service_id: &str,
    topic: String,
    payload: String,
) -> Result<(), Fatal> {
    match publish_owp(session, warnings, service_id, topic, payload).await {
        Ok(()) => {
            if verbose {
                send(out_tx, ServerOp::Ok).await;
            }
        }
        Err(err) => {
            send(out_tx, err_op(OwpError::InvalidMessage, Some(&err))).await;
        }
    }
    Ok(())
}

/// Subscribes the engine and spawns a forwarder that writes MSG frames.
///
/// Duplicate `sid` or the per-connection cap is `-ERR` without closing.
/// The protocol has no resource-limit code; the cap uses Illegal-State.
async fn handle_sub(
    session: &Session,
    subs: &mut HashMap<String, LiveSub>,
    out_tx: &mpsc::Sender<ServerOp>,
    verbose: bool,
    sid: String,
    message_name: String,
    topic: String,
) -> Result<(), Fatal> {
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
        let mut logged_conversion_invalid = false;
        while let Some(delivery) = rx.recv().await {
            let Ok(raw) = String::from_utf8(delivery.envelope.payload.to_vec()) else {
                warn!("dropping non-utf8 payload destined for OWP");
                continue;
            };

            // What arrived off the bus, before any conversion of ours,
            // so a violation is attributed to the producer.
            let violations = violations_of(raw.as_bytes(), schema.as_deref(), validate);
            match handle_violations(
                &violations,
                validate,
                &adapter_id,
                &local_sid,
                &forward_tx,
                &mut logged_invalid,
                ViolationWording::PRODUCER,
            )
            .await
            {
                Verdict::Drop => continue,
                Verdict::Closed => break,
                Verdict::Forward => {}
            }
            let payload = if xml_baseline {
                match xml_to_oms_json(&raw, schema.as_deref()) {
                    Ok(json) => {
                        // Conversion is a no-op when the bus payload was not XML
                        // to begin with (xml_to_oms_json returns it unchanged),
                        // in which case re-checking here would just repeat the
                        // producer check above and misattribute its violation to
                        // conversion. Only worth checking when bytes may have
                        // actually changed: conversion is best-effort, and a bug
                        // in it can produce JSON that no longer follows the
                        // schema even though the bus payload did.
                        if oa_gateway_uci::looks_like_xml(raw.as_bytes()) {
                            let violations =
                                violations_of(json.as_bytes(), schema.as_deref(), validate);
                            match handle_violations(
                                &violations,
                                validate,
                                &adapter_id,
                                &local_sid,
                                &forward_tx,
                                &mut logged_conversion_invalid,
                                ViolationWording::CONVERSION,
                            )
                            .await
                            {
                                Verdict::Drop => continue,
                                Verdict::Closed => break,
                                Verdict::Forward => {}
                            }
                        }
                        json
                    }
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
                        let details =
                            format!("delivery on {local_sid} could not be converted: {err}");
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
    if verbose {
        send(out_tx, ServerOp::Ok).await;
    }
    Ok(())
}

/// Aborts the forwarder and unsubscribes. Unknown `sid` is `-ERR`.
async fn handle_unsub(
    session: &Session,
    subs: &mut HashMap<String, LiveSub>,
    out_tx: &mpsc::Sender<ServerOp>,
    verbose: bool,
    sid: String,
) -> Result<(), Fatal> {
    if let Some(live) = subs.remove(&sid) {
        live.forwarder.abort();
        let _ = session
            .engine
            .unsubscribe(session.adapter_id.clone(), live.engine_sub)
            .await;
        if verbose {
            send(out_tx, ServerOp::Ok).await;
        }
    } else {
        send(
            out_tx,
            err_op(OwpError::IllegalArgument, Some("unknown sid")),
        )
        .await;
    }
    Ok(())
}

/// Maps one PUB onto one or two engine envelopes and publishes them.
///
/// A-GRA unwrap, XML baseline conversion, and validation all run before
/// any publish, so a wrapper and its inner are all-or-nothing. Each
/// envelope is stamped with `owp.service_id` and `owp.conn_id`.
///
/// # Errors
///
/// Returns a message for unwrap, type-hint, conversion, or reject-mode
/// validation failures. Unroutable publishes are warned, not errors.
async fn publish_owp(
    session: &Session,
    warnings: &mut RouteWarnings,
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
            match warnings.invalid.report(&env.route) {
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
        let outcome = session.engine.publish(env).await;
        if outcome.dropped > 0 {
            warn!(
                adapter = %session.adapter_id,
                service = %service_id,
                route = %route,
                matched = outcome.matched,
                delivered = outcome.delivered,
                dropped = outcome.dropped,
                "engine dropped deliveries on this publish"
            );
        }
        if outcome.matched == 0 {
            match warnings.unroutable.report(&route) {
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

/// Checks INIT.versions contains [`OWP_VERSION`] and INIT.schema matches
/// when [`OwpConfig::schema`] is set.
///
/// # Errors
///
/// Returns [`OwpError::UnsupportedVersion`] or
/// [`OwpError::UnsupportedSchema`].
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

/// Builds INFO. The service UUID is v5 of the client's `service_id`.
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

/// What the caller of [`handle_violations`] should do with the delivery.
enum Verdict {
    /// No violations, or `Warn` mode: forward the payload as usual.
    Forward,
    /// `Reject` mode found violations: the caller should drop this delivery
    /// (`continue` its loop) after the error has already been sent.
    Drop,
    /// The client's channel is closed: the caller should stop the forwarder.
    Closed,
}

/// The wording that differs between a violation found in what the producer
/// put on the bus and one found in what `xml_baseline` conversion produced.
struct ViolationWording {
    /// Noun phrase completing "delivery on {sid} ...: {summary}".
    detail: &'static str,
    reject_log: &'static str,
    warn_log: &'static str,
}

impl ViolationWording {
    const PRODUCER: Self = Self {
        detail: "does not follow the UCI schema",
        reject_log: "dropping a delivery that does not follow the UCI schema; later ones on \
                      this subscription are not logged",
        warn_log: "delivered payload does not follow the UCI schema; forwarding it anyway, \
                    and later ones on this subscription are not logged",
    };
    const CONVERSION: Self = Self {
        detail: "converted to a payload that does not follow the UCI schema",
        reject_log: "dropping a delivery that converted to a payload not following the UCI \
                      schema; later ones on this subscription are not logged",
        warn_log: "converted payload does not follow the UCI schema; forwarding it anyway, \
                    and later ones on this subscription are not logged",
    };
}

/// Applies `validate`'s policy to `violations`: in `Reject` mode, sends a
/// `-ERR` and reports [`Verdict::Drop`]; in `Warn` mode, only logs. Either
/// way the warning is logged once per subscription, via `logged`.
async fn handle_violations(
    violations: &[oa_gateway_uci::validate::Violation],
    validate: ValidateMode,
    adapter_id: &impl std::fmt::Display,
    local_sid: &str,
    forward_tx: &mpsc::Sender<ServerOp>,
    logged: &mut bool,
    wording: ViolationWording,
) -> Verdict {
    if violations.is_empty() {
        return Verdict::Forward;
    }
    let summary = summarize(violations);
    if validate == ValidateMode::Reject {
        if !*logged {
            *logged = true;
            warn!(adapter = %adapter_id, sid = %local_sid, violations = %summary, "{}", wording.reject_log);
        }
        let details = format!("delivery on {local_sid} {}: {summary}", wording.detail);
        if forward_tx
            .send(err_op(OwpError::InvalidMessage, Some(&details)))
            .await
            .is_err()
        {
            return Verdict::Closed;
        }
        return Verdict::Drop;
    }
    if !*logged {
        *logged = true;
        warn!(adapter = %adapter_id, sid = %local_sid, violations = %summary, "{}", wording.warn_log);
    }
    Verdict::Forward
}

/// Builds a `-ERR` frame. `details` is the rest of the line when set.
fn err_op(error: OwpError, details: Option<&str>) -> ServerOp {
    ServerOp::Err {
        error,
        details: details.map(str::to_owned),
    }
}

/// Queues `op` for the writer. A full or closed channel is ignored so
/// a slow client cannot stall the read loop.
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
}
