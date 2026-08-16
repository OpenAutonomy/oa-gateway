//! `[engine]` section: process-wide router reporting.
//!
//! The engine itself has no config. This section only controls how the
//! host talks about it.

use serde::Deserialize;

/// Host-side engine reporting.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EngineSection {
    /// Seconds between `EngineStats` log lines. `0` disables the ticker.
    /// Defaults to `30`.
    #[serde(default = "default_stats_interval_secs")]
    pub(crate) stats_interval_secs: u64,
}

impl Default for EngineSection {
    fn default() -> Self {
        Self {
            stats_interval_secs: default_stats_interval_secs(),
        }
    }
}

fn default_stats_interval_secs() -> u64 {
    30
}
