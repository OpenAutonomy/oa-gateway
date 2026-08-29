//! `[owp]` section: the OWP/WebSocket server.
//!
//! `schema` here is the protocol version string on INIT, not the UCI XSD
//! list in `[uci]`. An empty string disables the INIT check.

use serde::Deserialize;

use super::default_true;

/// OWP adapter settings.
///
/// Off when the section is omitted. A present `[owp]` table turns it
/// on unless `enabled = false`. `bind` stays a string so
/// `localhost:9000` can be resolved at startup the same way as
/// `127.0.0.1:9000`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OwpSection {
    /// Whether to spawn the adapter. `true` when the section is present.
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// Engine adapter id. Defaults to `"owp"`.
    #[serde(default = "default_owp_id")]
    pub(crate) id: String,
    /// Listen address, `host:port`. Defaults to `127.0.0.1:9000`.
    #[serde(default = "default_bind")]
    pub(crate) bind: String,
    /// Server identity sent on INIT. Defaults to `"oa-gateway-0"`.
    #[serde(default = "default_server_id")]
    pub(crate) server_id: String,
    /// Human-readable label sent on INIT.
    #[serde(default = "default_label")]
    pub(crate) system_label: String,
    /// Protocol schema string a client INIT must match exactly.
    ///
    /// This is not [`super::UciSection::schema`]. Empty means no check.
    /// Defaults to `"002.5.0"`.
    #[serde(default = "default_schema")]
    pub(crate) schema: String,
    /// Peel A-GRA Rx/Tx hex wrappers on PUB and fan out wrapper plus inner.
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub(crate) unwrap_ma_payloads: bool,
    /// Convert OMS JSON ↔ UCI XML at the socket so the engine and ASB see
    /// XML. Requires `[uci].schema`. Defaults to `false`.
    #[serde(default)]
    pub(crate) xml_baseline: bool,
    /// PEM certificate chain served to clients, leaf certificate first.
    /// Empty (the default) leaves the listener plaintext. Requires
    /// `tls_key`; setting one without the other is a startup error.
    #[serde(default)]
    pub(crate) tls_cert: String,
    /// PEM private key for `tls_cert`, in PKCS#8, PKCS#1, or SEC1 form.
    /// Empty (the default) leaves the listener plaintext. Requires
    /// `tls_cert`.
    #[serde(default)]
    pub(crate) tls_key: String,
    /// PEM bundle of certificate authorities a client certificate must
    /// chain to. Empty (the default) accepts a client with or without one.
    /// Requires `tls_cert`/`tls_key`; once set, a client that cannot
    /// present a certificate from this bundle is refused at the handshake.
    #[serde(default)]
    pub(crate) tls_client_ca: String,
    /// Largest frame accepted from a client, in bytes. Oversized frames
    /// end that session. Defaults to the adapter crate's limit.
    #[serde(default = "default_owp_max_frame_size")]
    pub(crate) max_frame_size: usize,
    /// Connections served at once. Further ones are closed on accept.
    #[serde(default = "default_owp_max_connections")]
    pub(crate) max_connections: usize,
    /// Subscriptions one connection may hold. A SUB past the limit is
    /// refused and the session continues.
    #[serde(default = "default_owp_max_subscriptions")]
    pub(crate) max_subscriptions: usize,
    /// Seconds from an accepted connection to a successful INIT before it
    /// is closed. `0` disables. Defaults to `30`.
    #[serde(default = "default_owp_init_timeout_secs")]
    pub(crate) init_timeout_secs: u64,
    /// Seconds with no frame in either direction on an active session
    /// before it is closed. `0` disables. Defaults to `600`.
    #[serde(default = "default_owp_idle_timeout_secs")]
    pub(crate) idle_timeout_secs: u64,
    /// Rebind and accept again after the accept loop ends or panics.
    /// Defaults to `false`: an existing deployment sees no behavior
    /// change until it opts in.
    #[serde(default)]
    pub(crate) reconnect: bool,
    /// Seconds to wait between rebind attempts. Defaults to `1`.
    #[serde(default = "default_owp_reconnect_delay_secs")]
    pub(crate) reconnect_delay_secs: u64,
    /// `abort` or `reconnect` when the accept loop panics. Defaults to
    /// `"abort"`.
    #[serde(default = "default_owp_on_panic")]
    pub(crate) on_panic: String,
}

impl Default for OwpSection {
    fn default() -> Self {
        Self {
            enabled: false,
            id: default_owp_id(),
            bind: default_bind(),
            server_id: default_server_id(),
            system_label: default_label(),
            schema: default_schema(),
            unwrap_ma_payloads: true,
            xml_baseline: false,
            tls_cert: String::new(),
            tls_key: String::new(),
            tls_client_ca: String::new(),
            max_frame_size: default_owp_max_frame_size(),
            max_connections: default_owp_max_connections(),
            max_subscriptions: default_owp_max_subscriptions(),
            init_timeout_secs: default_owp_init_timeout_secs(),
            idle_timeout_secs: default_owp_idle_timeout_secs(),
            reconnect: false,
            reconnect_delay_secs: default_owp_reconnect_delay_secs(),
            on_panic: default_owp_on_panic(),
        }
    }
}

impl OwpSection {
    /// Parses [`Self::on_panic`] into [`oa_gateway_adapter::OnPanic`].
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not `abort` or `reconnect`.
    pub(crate) fn on_panic_mode(&self) -> Result<oa_gateway_adapter::OnPanic, String> {
        self.on_panic
            .parse()
            .map_err(|err| format!("owp.on_panic: {err}"))
    }
}

fn default_owp_id() -> String {
    "owp".into()
}
fn default_bind() -> String {
    "127.0.0.1:9000".into()
}
fn default_server_id() -> String {
    "oa-gateway-0".into()
}
fn default_label() -> String {
    "OA-Gateway Prototype".into()
}
fn default_schema() -> String {
    "002.5.0".into()
}
fn default_owp_max_frame_size() -> usize {
    oa_gateway_owp::DEFAULT_MAX_FRAME_SIZE
}
fn default_owp_max_connections() -> usize {
    oa_gateway_owp::DEFAULT_MAX_CONNECTIONS
}
fn default_owp_max_subscriptions() -> usize {
    oa_gateway_owp::DEFAULT_MAX_SUBSCRIPTIONS
}
fn default_owp_init_timeout_secs() -> u64 {
    oa_gateway_owp::DEFAULT_INIT_TIMEOUT_SECS
}
fn default_owp_idle_timeout_secs() -> u64 {
    oa_gateway_owp::DEFAULT_IDLE_TIMEOUT_SECS
}
fn default_owp_reconnect_delay_secs() -> u64 {
    1
}
fn default_owp_on_panic() -> String {
    oa_gateway_adapter::OnPanic::default().to_string()
}
