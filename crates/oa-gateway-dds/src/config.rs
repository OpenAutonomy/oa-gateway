//! Runtime settings for a DDS session. This is not the host TOML section.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use oa_gateway_adapter::OnPanic;
use oa_gateway_uci::validate::Mode as ValidateMode;
use oa_gateway_uci::Schema;

/// Largest DDS sample accepted from the domain, in bytes, before it is
/// unwrapped or converted.
///
/// Matches OWP's default frame limit. DDS has no handshake of its own to
/// cap a sample's size the way a WebSocket frame limit does, so this is
/// a plain length check instead of a transport-level setting.
pub const DEFAULT_MAX_SAMPLE_SIZE: usize = 16 * 1024 * 1024;

/// Which [`crate::DdsProvider`] the host should construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DdsProviderKind {
    /// Pure-Rust rustdds. The only value in this build.
    #[default]
    Rustdds,
}

impl fmt::Display for DdsProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rustdds => "rustdds",
        })
    }
}

impl FromStr for DdsProviderKind {
    type Err = String;

    /// # Errors
    ///
    /// Returns a message if `s` is not `rustdds`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "rustdds" => Ok(Self::Rustdds),
            other => Err(format!("unknown dds provider '{other}'; expected rustdds")),
        }
    }
}

/// Runtime settings for a DDS session.
///
/// This is not the host TOML section. `qos` is already a path the host
/// has checked exists. There is no `enabled` flag here — spawning is
/// the host's decision. [`Self::provider`] is a closed enum; the only
/// value in this build is [`DdsProviderKind::Rustdds`].
#[derive(Debug, Clone)]
pub struct DdsConfig {
    /// Which provider to construct. Unknown names are refused before
    /// this struct is built.
    pub provider: DdsProviderKind,
    /// DDS domain id the participant joins.
    pub domain_id: u16,
    /// QoS file the provider interprets. rustdds parses a documented
    /// DDS-XML subset; a later FFI provider may pass the same path to
    /// its own loader.
    pub qos: PathBuf,
    /// Engine topic names and DDS topic names, bridged both ways.
    pub topics: Vec<String>,
    /// Peel A-GRA Rx/Tx wrappers on inbound samples and publish the
    /// wrapper plus the inner message.
    pub unwrap_ma_payloads: bool,
    /// Skip outbound samples that originated on this adapter.
    pub suppress_echo: bool,
    /// Panic while joined to the domain: abort `run`, or treat it as a
    /// failed session.
    pub on_panic: OnPanic,
    /// Rejoin the domain after the session ends or panics.
    pub reconnect: bool,
    /// Sleep between rejoin attempts.
    pub reconnect_delay: Duration,
    /// Compiled UCI schema used to check inbound samples. `None` skips
    /// the check: there is nothing to check against.
    pub schema: Option<Arc<Schema>>,
    /// What to do about an inbound sample that does not follow
    /// [`Self::schema`]. Has no effect without one.
    pub validate: ValidateMode,
    /// Largest inbound sample accepted, in bytes, before it is unwrapped
    /// or converted. An oversized sample is dropped and logged.
    pub max_sample_size: usize,
}

impl Default for DdsConfig {
    fn default() -> Self {
        Self {
            provider: DdsProviderKind::Rustdds,
            domain_id: 0,
            qos: PathBuf::from("config/dds-qos.xml"),
            topics: vec!["demo".into()],
            unwrap_ma_payloads: true,
            suppress_echo: true,
            on_panic: OnPanic::Abort,
            reconnect: false,
            reconnect_delay: Duration::from_secs(1),
            schema: None,
            validate: ValidateMode::default(),
            max_sample_size: DEFAULT_MAX_SAMPLE_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_reads_back() {
        assert_eq!(
            DdsProviderKind::Rustdds
                .to_string()
                .parse::<DdsProviderKind>()
                .unwrap(),
            DdsProviderKind::Rustdds
        );
        let err = DdsProviderKind::from_str("cyclone").unwrap_err();
        assert!(err.contains("rustdds"), "{err}");
    }
}
