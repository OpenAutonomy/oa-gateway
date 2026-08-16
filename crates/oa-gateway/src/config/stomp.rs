//! `[stomp]` section: the STOMP client toward an ActiveMQ (or other) broker.
//!
//! Off when the section is omitted, so a config that never mentions it
//! does not try to connect. A present `[stomp]` table turns it on
//! unless `enabled = false`. Java CAL/OpenWire peers share the same
//! destinations when the topic names match.

use serde::Deserialize;

use super::default_true;

/// STOMP adapter settings.
///
/// `broker` stays a string so a hostname can be resolved at startup.
/// Empty `login` and `passcode` mean omit those CONNECT headers, not send
/// blanks.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StompSection {
    /// Whether to spawn the adapter. `true` when the section is present.
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// Engine adapter id. Defaults to `"stomp"`.
    #[serde(default = "default_stomp_id")]
    pub(crate) id: String,
    /// Broker address, `host:port`. Defaults to `127.0.0.1:61613`.
    #[serde(default = "default_stomp_broker")]
    pub(crate) broker: String,
    /// STOMP `host` header. ActiveMQ Classic typically wants `"/"`.
    #[serde(default = "default_stomp_host")]
    pub(crate) host: String,
    /// CONNECT login. Empty (the default) omits the header.
    #[serde(default)]
    pub(crate) login: String,
    /// CONNECT passcode. Empty (the default) omits the header.
    #[serde(default)]
    pub(crate) passcode: String,
    /// Prefix prepended to each topic to form a STOMP destination.
    /// Defaults to `"/topic/"`.
    #[serde(default = "default_stomp_prefix")]
    pub(crate) destination_prefix: String,
    /// Engine topic names, and the suffixes of the STOMP destinations
    /// bridged both ways. Defaults to `["demo"]`.
    #[serde(default = "default_stomp_topics")]
    pub(crate) topics: Vec<String>,
    /// Peel A-GRA Rx/Tx hex wrappers on inbound frames and fan out
    /// wrapper plus inner. Defaults to `true`.
    #[serde(default = "default_true")]
    pub(crate) unwrap_ma_payloads: bool,
    /// Retry the broker after a dropped session. Defaults to `true`.
    #[serde(default = "default_true")]
    pub(crate) reconnect: bool,
    /// Seconds to wait between reconnect attempts. Defaults to `1`.
    #[serde(default = "default_stomp_reconnect_delay_secs")]
    pub(crate) reconnect_delay_secs: u64,
    /// Seconds for TCP connect and for the CONNECTED wait, each.
    /// Defaults to `5`.
    #[serde(default = "default_stomp_connect_timeout_secs")]
    pub(crate) connect_timeout_secs: u64,
    /// Skip outbound SEND when the envelope came from this adapter.
    /// Defaults to `true`.
    #[serde(default = "default_true")]
    pub(crate) suppress_echo: bool,
    /// `abort` or `reconnect` when a session task panics. Defaults to
    /// `"abort"`.
    #[serde(default = "default_stomp_on_panic")]
    pub(crate) on_panic: String,
    /// Largest frame accepted from the broker, in bytes. Defaults to the
    /// adapter crate's limit.
    #[serde(default = "default_stomp_max_frame_size")]
    pub(crate) max_frame_size: usize,
}

impl Default for StompSection {
    fn default() -> Self {
        Self {
            enabled: false,
            id: default_stomp_id(),
            broker: default_stomp_broker(),
            host: default_stomp_host(),
            login: String::new(),
            passcode: String::new(),
            destination_prefix: default_stomp_prefix(),
            topics: default_stomp_topics(),
            unwrap_ma_payloads: true,
            reconnect: true,
            reconnect_delay_secs: default_stomp_reconnect_delay_secs(),
            connect_timeout_secs: default_stomp_connect_timeout_secs(),
            suppress_echo: true,
            on_panic: default_stomp_on_panic(),
            max_frame_size: default_stomp_max_frame_size(),
        }
    }
}

impl StompSection {
    /// Parses [`Self::on_panic`] into [`oa_gateway_stomp::OnPanic`].
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not `abort` or `reconnect`.
    pub(crate) fn on_panic_mode(&self) -> Result<oa_gateway_stomp::OnPanic, String> {
        self.on_panic
            .parse()
            .map_err(|err| format!("stomp.on_panic: {err}"))
    }
}

fn default_stomp_id() -> String {
    "stomp".into()
}
fn default_stomp_broker() -> String {
    "127.0.0.1:61613".into()
}
fn default_stomp_host() -> String {
    "/".into()
}
fn default_stomp_prefix() -> String {
    "/topic/".into()
}
fn default_stomp_topics() -> Vec<String> {
    vec!["demo".into()]
}
fn default_stomp_max_frame_size() -> usize {
    oa_gateway_stomp::DEFAULT_MAX_FRAME_SIZE
}
fn default_stomp_reconnect_delay_secs() -> u64 {
    1
}
fn default_stomp_connect_timeout_secs() -> u64 {
    5
}
fn default_stomp_on_panic() -> String {
    oa_gateway_stomp::OnPanic::default().to_string()
}
