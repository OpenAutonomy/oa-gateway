//! Constructs and spawns the DDS adapter.

use std::sync::Arc;

use oa_gateway_adapter::Adapter;
use oa_gateway_core::Engine;
use oa_gateway_dds::{DdsAdapter, DdsConfig};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::DdsSection;

/// Spawns [`DdsAdapter`] and returns a handle to its `run` task.
///
/// # Errors
///
/// Returns an error if `provider` is unknown or `qos` is missing.
pub(crate) fn start(
    section: &DdsSection,
    engine: Arc<Engine>,
    shutdown: CancellationToken,
) -> Result<JoinHandle<()>, String> {
    let qos = section.require_qos()?;
    let adapter = Arc::new(DdsAdapter::new(
        section.id.clone(),
        DdsConfig {
            provider: section.provider_kind()?,
            domain_id: section.domain_id,
            qos,
            topics: section.topics.clone(),
            unwrap_ma_payloads: section.unwrap_ma_payloads,
            suppress_echo: section.suppress_echo,
        },
    ));
    info!(id = %adapter.id(), domain = section.domain_id, "starting dds adapter");
    Ok(tokio::spawn(async move {
        if let Err(err) = adapter.run(engine, shutdown).await {
            error!(error = %err, "dds adapter failed");
        }
    }))
}
