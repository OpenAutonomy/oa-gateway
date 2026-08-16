//! Constructs and spawns the STOMP client adapter.
//!
//! `broker` is already resolved. Empty login and passcode become `None` so
//! the CONNECT frame omits those headers instead of sending blanks.

use std::net::SocketAddr;
use std::sync::Arc;

use oa_gateway_adapter::Adapter;
use oa_gateway_core::Engine;
use oa_gateway_stomp::{StompAdapter, StompConfig};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::StompSection;

/// Spawns [`StompAdapter`] toward `broker` and returns a handle to its
/// `run` task.
///
/// Timeouts, echo skip, and panic policy come from `section`. A failure
/// from `run` is logged and the task still finishes, so one adapter
/// cannot take the host down.
///
/// # Errors
///
/// Returns an error if `on_panic` is not `abort` or `reconnect`.
pub(crate) fn start(
    section: &StompSection,
    broker: SocketAddr,
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
        },
    ));
    info!(id = %adapter.id(), %broker, "starting stomp adapter");
    Ok(tokio::spawn(async move {
        if let Err(err) = adapter.run(engine, shutdown).await {
            error!(error = %err, "stomp adapter failed");
        }
    }))
}
