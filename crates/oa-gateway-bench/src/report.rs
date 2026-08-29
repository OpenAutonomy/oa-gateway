//! Histogram, engine counters, text summary, and JSON file.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use oa_gateway_core::Engine;
use serde::Serialize;

/// One finished run.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Report {
    pub scenario: String,
    pub git_sha: String,
    pub rustc: String,
    pub profile: String,
    pub started_unix: u64,
    pub flags: BTreeMap<String, serde_json::Value>,
    pub sent: u64,
    pub received: u64,
    pub dropped: u64,
    pub errors: u64,
    pub unmatched: u64,
    pub duration_secs: f64,
    pub sent_per_sec: f64,
    pub received_per_sec: f64,
    pub bytes_per_sec: f64,
    pub latency_ns: Option<Latency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_latency_ns: Option<Latency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineSnapshot>,
}

/// Percentiles over one sample set.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Latency {
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub max: u64,
    pub mean: u64,
    pub count: u64,
}

/// Process-lifetime engine counters at the end of a run.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EngineSnapshot {
    pub published: u64,
    pub delivered: u64,
    pub dropped: u64,
}

impl Report {
    /// Fills metadata and throughput from the raw counters.
    pub(crate) fn finish(
        mut self,
        samples: Vec<u64>,
        ack_samples: Option<Vec<u64>>,
        payload_bytes: u64,
    ) -> Self {
        let wall = self.duration_secs.max(f64::EPSILON);
        self.sent_per_sec = self.sent as f64 / wall;
        self.received_per_sec = self.received as f64 / wall;
        self.bytes_per_sec = self.received as f64 * payload_bytes as f64 / wall;
        self.latency_ns = summarize(samples);
        self.ack_latency_ns = ack_samples.and_then(summarize);
        self
    }

    /// Writes the human summary to stdout and optionally the JSON file.
    ///
    /// # Errors
    ///
    /// Returns a message if `--json` cannot be written.
    pub(crate) fn emit(&self, json_path: Option<&Path>) -> Result<(), String> {
        println!("{}", self.summary());
        if let Some(path) = json_path {
            let text = serde_json::to_string_pretty(self)
                .map_err(|err| format!("cannot serialize report: {err}"))?;
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)
                        .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
                }
            }
            fs::write(path, text)
                .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
        }
        Ok(())
    }

    fn summary(&self) -> String {
        let mut lines = vec![
            format!("scenario: {}", self.scenario),
            format!(
                "duration: {:.2}s  sent: {}  received: {}  dropped: {}  errors: {}  unmatched: {}",
                self.duration_secs,
                self.sent,
                self.received,
                self.dropped,
                self.errors,
                self.unmatched
            ),
            format!(
                "throughput: {:.0} sent/s  {:.0} recv/s  {:.0} B/s",
                self.sent_per_sec, self.received_per_sec, self.bytes_per_sec
            ),
        ];
        if let Some(lat) = &self.latency_ns {
            lines.push(format!(
                "latency: p50={} p90={} p99={} max={} mean={} (n={})",
                fmt_ns(lat.p50),
                fmt_ns(lat.p90),
                fmt_ns(lat.p99),
                fmt_ns(lat.max),
                fmt_ns(lat.mean),
                lat.count
            ));
        } else {
            lines.push("latency: no samples".into());
        }
        if let Some(lat) = &self.ack_latency_ns {
            lines.push(format!(
                "ack latency: p50={} p90={} p99={} max={} mean={} (n={})",
                fmt_ns(lat.p50),
                fmt_ns(lat.p90),
                fmt_ns(lat.p99),
                fmt_ns(lat.max),
                fmt_ns(lat.mean),
                lat.count
            ));
        }
        if let Some(eng) = &self.engine {
            lines.push(format!(
                "engine: published={} delivered={} dropped={}",
                eng.published, eng.delivered, eng.dropped
            ));
        }
        lines.join("\n")
    }
}

/// Empty report with process metadata filled in.
#[must_use]
pub(crate) fn blank(
    scenario: impl Into<String>,
    flags: BTreeMap<String, serde_json::Value>,
) -> Report {
    Report {
        scenario: scenario.into(),
        git_sha: git_sha(),
        rustc: rustc_version(),
        profile: if cfg!(debug_assertions) {
            "debug".into()
        } else {
            "release".into()
        },
        started_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        flags,
        sent: 0,
        received: 0,
        dropped: 0,
        errors: 0,
        unmatched: 0,
        duration_secs: 0.0,
        sent_per_sec: 0.0,
        received_per_sec: 0.0,
        bytes_per_sec: 0.0,
        latency_ns: None,
        ack_latency_ns: None,
        engine: None,
    }
}

/// Snapshot of [`Engine::stats`].
#[must_use]
pub(crate) fn engine_snapshot(engine: &Engine) -> EngineSnapshot {
    let stats = engine.stats();
    EngineSnapshot {
        published: stats.published(),
        delivered: stats.delivered(),
        dropped: stats.dropped(),
    }
}

/// Sorted-sample percentiles. `None` when there are no samples.
#[must_use]
pub(crate) fn summarize(mut samples: Vec<u64>) -> Option<Latency> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    let count = samples.len() as u64;
    let sum: u128 = samples.iter().copied().map(u128::from).sum();
    Some(Latency {
        p50: percentile(&samples, 50.0),
        p90: percentile(&samples, 90.0),
        p99: percentile(&samples, 99.0),
        max: *samples.last().expect("non-empty"),
        mean: u64::try_from(sum / u128::from(count)).unwrap_or(u64::MAX),
        count,
    })
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.2}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}µs", ns as f64 / 1_000.0)
    } else {
        format!("{ns}ns")
    }
}

fn git_sha() -> String {
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        return sha;
    }
    if let Ok(sha) = std::env::var("CI_COMMIT_SHA") {
        return sha;
    }
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map_or_else(|| "unknown".into(), |s| s.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_on_a_known_set() {
        let lat = summarize(vec![1, 2, 3, 4, 5]).unwrap();
        assert_eq!(lat.p50, 3);
        assert_eq!(lat.max, 5);
        assert_eq!(lat.count, 5);
        assert_eq!(lat.mean, 3);
    }

    #[test]
    fn empty_is_none() {
        assert!(summarize(Vec::new()).is_none());
    }
}
