//! Starts the DDS adapter for in-process tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oa_gateway_core::Engine;
use oa_gateway_dds::{DdsAdapter, DdsConfig, DdsProviderKind};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

/// Shipped QoS file used by tests.
#[must_use]
pub fn shipped_qos_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/dds-qos.xml")
}

/// Starts [`DdsAdapter`] on `domain_id` and waits until it has joined.
///
/// Panics if `run` returns before the settle time.
pub async fn start_dds_adapter(
    engine: Arc<Engine>,
    id: impl Into<String>,
    domain_id: u16,
    topics: Vec<String>,
) -> CancellationToken {
    start_dds_adapter_with(engine, id, domain_id, topics, |_| {}).await
}

/// As [`start_dds_adapter`], with the config open for editing first.
///
/// The fixture schema ([`oa_gateway_uci::slice::v25`]) is always
/// attached, the same way [`crate::owp::start_owp_with`] attaches it —
/// `edit` only needs to set [`DdsConfig::validate`] to turn checking on.
/// Tests for `max_sample_size` set it far below the default, so reaching
/// it costs a few bytes rather than megabytes.
pub async fn start_dds_adapter_with(
    engine: Arc<Engine>,
    id: impl Into<String>,
    domain_id: u16,
    topics: Vec<String>,
    edit: impl FnOnce(&mut DdsConfig),
) -> CancellationToken {
    let shutdown = CancellationToken::new();
    let mut config = DdsConfig {
        provider: DdsProviderKind::Rustdds,
        domain_id,
        qos: shipped_qos_path(),
        topics,
        unwrap_ma_payloads: true,
        suppress_echo: true,
        schema: Some(Arc::new(oa_gateway_uci::slice::v25().clone())),
        ..DdsConfig::default()
    };
    edit(&mut config);
    let adapter = Arc::new(DdsAdapter::new(id.into(), config));
    let token = shutdown.clone();
    let handle = tokio::spawn(async move { adapter.serve(engine, token).await });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !handle.is_finished(),
        "dds adapter ended during startup: {:?}",
        timeout(Duration::from_millis(10), handle).await
    );
    shutdown
}
