//! `[loopback]` section: the in-process adapter with no socket.

use serde::Deserialize;

use super::default_true;

/// Loopback adapter settings.
///
/// Off when the section is omitted. A present `[loopback]` table turns
/// it on unless `enabled = false`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoopbackSection {
    /// Whether to spawn the adapter. `true` when the section is present.
    #[serde(default = "default_true")]
    pub(crate) enabled: bool,
    /// Engine adapter id. Defaults to `"loopback"`.
    #[serde(default = "default_loopback_id")]
    pub(crate) id: String,
}

impl Default for LoopbackSection {
    fn default() -> Self {
        Self {
            enabled: false,
            id: default_loopback_id(),
        }
    }
}

fn default_loopback_id() -> String {
    "loopback".into()
}
