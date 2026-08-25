//! Constructs and spawns the DDS adapter.

use std::sync::Arc;

use oa_gateway_adapter::Adapter;
use oa_gateway_core::Engine;
use oa_gateway_dds::{DdsAdapter, DdsConfig};
use oa_gateway_uci::{Schema, ValidateMode};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::DdsSection;

/// Spawns [`DdsAdapter`] and returns a handle to its `run` task.
///
/// `schema` and `validate` check inbound samples the same way they
/// check OWP traffic; `validate` has no effect without a `schema`.
///
/// # Errors
///
/// Returns an error if `provider` is unknown, `qos` is missing, or
/// `on_panic` is not `abort` or `reconnect`.
pub(crate) fn start(
    section: &DdsSection,
    schema: Option<&Arc<Schema>>,
    validate: ValidateMode,
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
            on_panic: section.on_panic_mode()?,
            reconnect: section.reconnect,
            reconnect_delay: std::time::Duration::from_secs(section.reconnect_delay_secs),
            schema: schema.cloned(),
            validate,
            max_sample_size: section.max_sample_size,
        },
    ));
    info!(id = %adapter.id(), domain = section.domain_id, "starting dds adapter");
    Ok(tokio::spawn(async move {
        if let Err(err) = adapter.run(engine, shutdown).await {
            error!(error = %err, "dds adapter failed");
        }
    }))
}
