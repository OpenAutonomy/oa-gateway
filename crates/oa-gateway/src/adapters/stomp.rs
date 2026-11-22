//! Constructs and spawns the STOMP client adapter.
//!
//! `broker` is already resolved. Empty login and passcode become `None` so
//! the CONNECT frame omits those headers instead of sending blanks.

use std::net::SocketAddr;
use std::sync::Arc;

use oa_gateway_adapter::tls::ClientTls;
use oa_gateway_adapter::Adapter;
use oa_gateway_core::Engine;
use oa_gateway_stomp::{StompAdapter, StompConfig};
use oa_gateway_uci::{Schema, ValidateMode};
use oa_gateway_xsd_libxml::XsdLibxmlValidator;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::StompSection;

/// Spawns [`StompAdapter`] toward `broker` and returns a handle to its
/// `run` task.
///
/// Timeouts, echo skip, and panic policy come from `section`. `schema`
/// and `validate` check both inbound MESSAGE and outbound SEND frames the
/// same way they check OWP and DDS traffic; `validate` has no effect
/// without a `schema`. A failure from `run` is logged and the task still
/// finishes, so one adapter cannot take the host down.
///
/// # Errors
///
/// Returns an error if `on_panic` is not `abort` or `reconnect`.
#[allow(clippy::too_many_arguments)] // every parameter is an independent, host-owned input; bundling them would just move the count into a struct literal at the one call site.
pub(crate) fn start(
    section: &StompSection,
    broker: SocketAddr,
    tls: Option<ClientTls>,
    schema: Option<&Arc<Schema>>,
    xsd_validator: Option<&Arc<XsdLibxmlValidator>>,
    validate: ValidateMode,
    engine: Arc<Engine>,
    shutdown: CancellationToken,
) -> Result<JoinHandle<()>, String> {
    let login = if section.login.is_empty() {
        None
    } else {
        Some(section.login.clone())
    };
    let passcode = if section.passcode.is_empty() {
        None
    } else {
        Some(section.passcode.clone())
    };
    let tls_on = tls.is_some();
    let adapter = Arc::new(StompAdapter::new(
        section.id.clone(),
        StompConfig {
            broker,
            host: section.host.clone(),
            login,
            passcode,
            destination_prefix: section.destination_prefix.clone(),
            topics: section.topics.clone(),
            unwrap_ma_payloads: section.unwrap_ma_payloads,
            reconnect: section.reconnect,
            reconnect_delay: std::time::Duration::from_secs(section.reconnect_delay_secs),
            connect_timeout: std::time::Duration::from_secs(section.connect_timeout_secs),
            suppress_echo: section.suppress_echo,
            on_panic: section.on_panic_mode()?,
            max_frame_size: section.max_frame_size,
            tls,
            schema: schema.cloned(),
            xsd_validator: xsd_validator.cloned(),
            validate,
        },
    ));
    info!(id = %adapter.id(), %broker, tls = tls_on, "starting stomp adapter");
    Ok(tokio::spawn(async move {
        if let Err(err) = adapter.run(engine, shutdown).await {
            error!(error = %err, "stomp adapter failed");
        }
    }))
}
