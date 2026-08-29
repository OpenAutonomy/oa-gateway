//! Listen address, INIT identity, and resource limits.
//!
//! `schema` here is the OWP protocol version string on INIT, not the UCI
//! XSD the host compiles. `xml_baseline` is what needs that XSD.

use std::net::SocketAddr;
use std::time::Duration;

use oa_gateway_adapter::OnPanic;
use oa_gateway_uci::validate::Mode as ValidateMode;

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

/// Seconds from an accepted WebSocket to a successful INIT before the
/// session is closed. Without it a peer can hold a connection slot forever
/// by connecting and never speaking OWP.
pub const DEFAULT_INIT_TIMEOUT_SECS: u64 = 30;

/// Seconds with no frame in either direction on an active session before it
/// is closed. Generous, since a subscriber to a low-rate topic is legitimately
/// quiet on the inbound side and the server's own MSG frames keep it alive.
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 600;

/// Settings for one OWP listener.
///
/// [`Self::schema`] is the INIT protocol version, not a UCI catalog.
/// [`Self::system_uuid`] is minted per process by the host, not read
/// from TOML.
#[derive(Debug, Clone)]
pub struct OwpConfig {
    /// Address already resolved by the host.
    pub bind: SocketAddr,
    /// Server identity sent on INFO.
    pub server_id: String,
    /// Human-readable label sent on INFO.
    pub system_label: String,
    /// When set, INIT.schema must match exactly. `None` skips the check.
    pub schema: Option<String>,
    /// System UUID sent on INFO. Generated per process, not configured.
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
    /// Deadline from an accepted WebSocket to a successful INIT. `None`
    /// disables it. Measured from the handshake, not reset by traffic: a
    /// peer that connects and never INITs is closed when this elapses.
    pub init_timeout: Option<Duration>,
    /// Longest gap with no frame in either direction on an active session.
    /// `None` disables it. Any client frame and any server frame resets it,
    /// so an active publisher or subscriber is never closed for being idle.
    pub idle_timeout: Option<Duration>,
    /// Exact `Origin` header values accepted at the WebSocket handshake.
    /// Empty (the default) accepts any origin, including none, which is the
    /// behavior before this list existed. When non-empty, a handshake whose
    /// `Origin` is not listed verbatim is refused with `403`.
    pub allowed_origins: Vec<String>,
    /// What to do about a payload that does not follow the loaded schema.
    /// Has no effect without one: there is nothing to check against.
    pub validate: ValidateMode,
    /// Panic in the accept loop: abort `run`, or treat it as a failed
    /// session.
    pub on_panic: OnPanic,
    /// Rebind and accept again after the accept loop ends or panics.
    pub reconnect: bool,
    /// Sleep between rebind attempts.
    pub reconnect_delay: Duration,
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
            init_timeout: Some(Duration::from_secs(DEFAULT_INIT_TIMEOUT_SECS)),
            idle_timeout: Some(Duration::from_secs(DEFAULT_IDLE_TIMEOUT_SECS)),
            allowed_origins: Vec::new(),
            validate: ValidateMode::default(),
            on_panic: OnPanic::Abort,
            reconnect: false,
            reconnect_delay: Duration::from_secs(1),
        }
    }
}
