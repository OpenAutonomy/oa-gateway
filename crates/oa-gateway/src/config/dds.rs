//! `[dds]` section: the DDS adapter toward a domain.
//!
//! Off when the section is omitted. A present `[dds]` table turns it
//! on unless `enabled = false`. `qos` is required when the section is
//! present.

use std::path::PathBuf;

use serde::Deserialize;

use super::default_true;

/// DDS adapter settings.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DdsSection {
    /// Whether to spawn the adapter. `true` when the section is present.
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// Engine adapter id. Defaults to `"dds"`.
    #[serde(default = "default_dds_id")]
    pub(crate) id: String,
    /// Provider name. Defaults to `"rustdds"`.
    #[serde(default = "default_dds_provider")]
    pub(crate) provider: String,
    /// DDS domain id. Defaults to `0`.
    #[serde(default)]
    pub(crate) domain_id: u16,
    /// QoS XML path. Required when the section is present.
    #[serde(default)]
    pub(crate) qos: PathBuf,
    /// Engine topic names and DDS topic names, bridged both ways.
    #[serde(default = "default_dds_topics")]
    pub(crate) topics: Vec<String>,
    /// Peel A-GRA Rx/Tx wrappers on inbound samples.
    #[serde(default = "default_true")]
    pub(crate) unwrap_ma_payloads: bool,
    /// Skip outbound samples that originated on this adapter.
    #[serde(default = "default_true")]
    pub(crate) suppress_echo: bool,
}

impl Default for DdsSection {
    fn default() -> Self {
        Self {
            enabled: false,
            id: default_dds_id(),
            provider: default_dds_provider(),
            domain_id: 0,
            qos: PathBuf::new(),
            topics: default_dds_topics(),
            unwrap_ma_payloads: true,
            suppress_echo: true,
        }
    }
}

impl DdsSection {
    /// Parses [`Self::provider`] into [`oa_gateway_dds::DdsProviderKind`].
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not a known provider.
    pub(crate) fn provider_kind(&self) -> Result<oa_gateway_dds::DdsProviderKind, String> {
        self.provider
            .parse()
            .map_err(|err| format!("dds.provider: {err}"))
    }

    /// # Errors
    ///
    /// Returns an error if `qos` is empty or the file is missing.
    pub(crate) fn require_qos(&self) -> Result<PathBuf, String> {
        if self.qos.as_os_str().is_empty() {
            return Err("dds.qos is required when [dds] is present".into());
        }
        if !self.qos.exists() {
            return Err(format!("dds.qos {} not found", self.qos.display()));
        }
        Ok(self.qos.clone())
    }
}

fn default_dds_id() -> String {
    "dds".into()
}
fn default_dds_provider() -> String {
    oa_gateway_dds::DdsProviderKind::default().to_string()
}
fn default_dds_topics() -> Vec<String> {
    vec!["demo".into()]
}
