//! Runtime settings for a DDS session. This is not the host TOML section.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

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
