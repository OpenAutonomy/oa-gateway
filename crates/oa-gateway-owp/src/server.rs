//! OWP/WebSocket adapter. Protocol control stays here; data crosses the engine.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use oa_gateway_adapter::AdapterError;
use oa_gateway_agra::{unwrap as unwrap_ma, wrapper_kind};
use oa_gateway_core::{
    AdapterId, ContentType, Delivery, Engine, Envelope, RouteKey, SubId, DEFAULT_CHANNEL_CAPACITY,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{accept_hdr_async, WebSocketStream};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::codec::{
    parse_client, type_hint_from_json, ClientOp, Identifiers, InfoPayload, InitPayload, OwpError,
    ServerOp,
};

const OWP_VERSION: &str = "1.0";

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
        }
    }
}

pub struct OwpAdapter {
    id: AdapterId,
    config: OwpConfig,
    conn_seq: AtomicU64,
}

impl OwpAdapter {
    #[must_use]
    pub fn new(id: impl Into<AdapterId>, config: OwpConfig) -> Self {
        Self {
            id: id.into(),
            config,
            conn_seq: AtomicU64::new(1),
        }
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
                    let conn_id = self.conn_seq.fetch_add(1, Ordering::Relaxed);
                    let this = Arc::clone(&self);
                    let engine = Arc::clone(&engine);
                    let shutdown = shutdown.clone();
                    tokio::spawn(async move {
                        if let Err(err) = this.handle_connection(stream, peer, conn_id, engine, shutdown).await {
                            debug!(%peer, conn_id, error = %err, "owp connection ended");
                        }
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
        let ws = match accept_hdr_async(stream, check_subprotocol).await {
            Ok(ws) => ws,
            Err(err) => {
                debug!(%peer, error = %err, "websocket handshake failed");
                return Ok(());
            }
        };
        run_session(Session {
            adapter_id: self.id.clone(),
            config: self.config.clone(),
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

async fn run_session(mut session: Session) -> Result<(), AdapterError> {
    let (out_tx, mut out_rx) = mpsc::channel::<ServerOp>(DEFAULT_CHANNEL_CAPACITY);
    let mut state = State::AwaitingInit;
    let mut subs: HashMap<String, LiveSub> = HashMap::new();

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
        ) => match publish_owp(session, service_id, topic, payload).await {
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
            let forwarder = tokio::spawn(async move {
                while let Some(delivery) = rx.recv().await {
                    let Ok(raw) = String::from_utf8(delivery.envelope.payload.to_vec()) else {
                        warn!("dropping non-utf8 payload destined for OWP");
                        continue;
                    };
                    let payload = if xml_baseline {
                        xml_to_oms_json(&raw)
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
            oa_gateway_uci::Message::from_xml(&payload, oa_gateway_uci::slice::v25())
                .map(|m| m.name)
                .unwrap_or_else(|_| topic.clone())
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
            toward_xml(env)?
        } else {
            env
        };
        session.engine.publish(stamp(env)).await;
    }
    Ok(())
}

fn toward_xml(mut env: Envelope) -> Result<Envelope, String> {
    if oa_gateway_uci::looks_like_xml(&env.payload) {
        env.content_type = ContentType::xml();
        return Ok(env);
    }
    let text = std::str::from_utf8(&env.payload).map_err(|e| e.to_string())?;
    let schema = oa_gateway_uci::slice::v25();
    let msg = oa_gateway_uci::Message::from_json(text, schema).map_err(|e| e.to_string())?;
    env.route.type_hint = Some(msg.name.clone());
    env.payload = bytes::Bytes::from(msg.to_xml(schema).map_err(|e| e.to_string())?);
    env.content_type = ContentType::xml();
    Ok(env)
}

fn xml_to_oms_json(raw: &str) -> String {
    if !oa_gateway_uci::looks_like_xml(raw.as_bytes()) {
        return raw.to_owned();
    }
    let schema = oa_gateway_uci::slice::v25();
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
