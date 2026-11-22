//! Constructs and spawns the OWP/WebSocket adapter.
//!
//! `bind` is already resolved. This module maps the TOML section onto
//! [`oa_gateway_owp::OwpConfig`] and attaches a compiled libxml2 validator
//! when the host loaded one.

use std::net::SocketAddr;
use std::sync::Arc;

use oa_gateway_adapter::tls::ServerTls;
use oa_gateway_adapter::{Adapter, ValidateMode};
use oa_gateway_core::Engine;
use oa_gateway_owp::{OwpAdapter, OwpConfig};
use oa_gateway_xsd_libxml::XsdLibxmlValidator;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::OwpSection;

/// Spawns [`OwpAdapter`] listening on `bind` and returns a handle to its
/// `run` task.
///
/// An empty `section.schema` becomes `None`, so INIT does not require a
/// protocol schema string. `system_uuid` is generated per process, not
/// read from the config. `validate` has no effect without a compiled
/// `xsd_validator`.
///
/// A failure from `run` is logged and the task still finishes, so one
/// adapter cannot take the host down.
///
/// # Errors
///
/// Returns an error if `on_panic` is not `abort` or `reconnect`.
#[allow(clippy::too_many_arguments)] // every parameter is an independent, host-owned input; bundling them would just move the count into a struct literal at the one call site.
pub(crate) fn start(
    section: &OwpSection,
    bind: SocketAddr,
    xsd_validator: Option<&Arc<XsdLibxmlValidator>>,
    validate: ValidateMode,
    tls: Option<ServerTls>,
    engine: Arc<Engine>,
    shutdown: CancellationToken,
) -> Result<JoinHandle<()>, String> {
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
            max_frame_size: section.max_frame_size,
            max_connections: section.max_connections,
            max_subscriptions: section.max_subscriptions,
            init_timeout: (section.init_timeout_secs > 0)
                .then(|| std::time::Duration::from_secs(section.init_timeout_secs)),
            idle_timeout: (section.idle_timeout_secs > 0)
                .then(|| std::time::Duration::from_secs(section.idle_timeout_secs)),
            allowed_origins: section.allowed_origins.clone(),
            validate,
            on_panic: section.on_panic_mode()?,
            reconnect: section.reconnect,
            reconnect_delay: std::time::Duration::from_secs(section.reconnect_delay_secs),
        },
    );
    if let Some(validator) = xsd_validator {
        adapter = adapter.with_xsd_validator(Arc::clone(validator));
    }
    let tls_on = tls.is_some();
    if let Some(tls) = tls {
        adapter = adapter.with_tls(tls);
    }
    let adapter = Arc::new(adapter);
    info!(id = %adapter.id(), bind = %bind, tls = tls_on, "starting owp adapter");
    Ok(tokio::spawn(async move {
        if let Err(err) = adapter.run(engine, shutdown).await {
            error!(error = %err, "owp adapter failed");
        }
    }))
}
