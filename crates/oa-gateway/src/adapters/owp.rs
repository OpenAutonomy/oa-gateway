//! Constructs and spawns the OWP/WebSocket adapter.
//!
//! `bind` is already resolved. This module maps the TOML section onto
//! [`oa_gateway_owp::OwpConfig`] and attaches a compiled UCI schema when
//! the host loaded one.

use std::net::SocketAddr;
use std::sync::Arc;

use oa_gateway_adapter::Adapter;
use oa_gateway_core::Engine;
use oa_gateway_owp::{OwpAdapter, OwpConfig};
use oa_gateway_uci::{Schema, ValidateMode};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::OwpSection;

/// Spawns [`OwpAdapter`] listening on `bind` and returns a handle to its
/// `run` task.
///
/// An empty `section.schema` becomes `None`, so INIT does not require a
/// protocol schema string. `system_uuid` is generated per process, not
/// read from the config. The compiled UCI `schema` is a separate object:
/// it drives JSON ↔ XML conversion and validation, and `validate` has no
/// effect without one.
///
/// A failure from `run` is logged and the task still finishes, so one
/// adapter cannot take the host down.
pub(crate) fn start(
    section: &OwpSection,
    bind: SocketAddr,
    schema: Option<&Arc<Schema>>,
    validate: ValidateMode,
    engine: Arc<Engine>,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    let mut adapter = OwpAdapter::new(
        section.id.clone(),
        OwpConfig {
            bind,
            server_id: section.server_id.clone(),
            system_label: section.system_label.clone(),
            schema: if section.schema.is_empty() {
                None
            } else {
                Some(section.schema.clone())
            },
            system_uuid: uuid::Uuid::new_v4().to_string(),
            unwrap_ma_payloads: section.unwrap_ma_payloads,
            xml_baseline: section.xml_baseline,
            max_frame_size: section.max_frame_size,
            max_connections: section.max_connections,
            max_subscriptions: section.max_subscriptions,
            validate,
        },
    );
    if let Some(schema) = schema {
        adapter = adapter.with_schema(Arc::clone(schema));
    }
    let adapter = Arc::new(adapter);
    info!(id = %adapter.id(), bind = %bind, "starting owp adapter");
    tokio::spawn(async move {
        if let Err(err) = adapter.run(engine, shutdown).await {
            error!(error = %err, "owp adapter failed");
        }
    })
}
