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
            max_frame_size: default_owp_max_frame_size(),
            max_connections: default_owp_max_connections(),
            max_subscriptions: default_owp_max_subscriptions(),
        }
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
