//! Constructs and spawns the in-process loopback adapter.
//!
//! Loopback has no socket. It exists so tests and local demos can publish
//! and subscribe without standing up OWP or a broker.

use std::sync::Arc;

use oa_gateway_adapter::Adapter;
use oa_gateway_core::Engine;
use oa_gateway_loopback::Loopback;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::LoopbackSection;

/// Spawns [`Loopback`] on `engine` and returns a handle to its `run` task.
///
/// The engine is given twice: once so the adapter can subscribe as a
/// handle, and again to [`Adapter::run`]. A failure from `run` is logged
/// and the task still finishes, so one adapter cannot take the host down.
pub(crate) fn start(
    section: &LoopbackSection,
    engine: Arc<Engine>,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    let adapter = Arc::new(Loopback::new(engine.clone(), section.id.clone()));
    info!(id = %adapter.id(), "starting loopback adapter");
    tokio::spawn(async move {
        if let Err(err) = adapter.run(engine, shutdown).await {
            error!(error = %err, "loopback adapter failed");
        }
    })
}
