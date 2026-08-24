//! Builds each enabled adapter and spawns its `run` task.
//!
//! The host talks to adapters only through [`oa_gateway_core::Engine`]. This
//! module owns construction and task lifetime; protocol work stays in the
//! adapter crates.

mod dds;
mod loopback;
mod owp;
mod stomp;

use std::sync::Arc;

use oa_gateway_core::Engine;
use oa_gateway_uci::{Schema, ValidateMode};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::addr::resolve_addr;
use crate::config::Config;

/// Resolves every listen/connect address, then spawns the adapters that are on.
///
/// Addresses are resolved first so a bad one fails cleanly instead of leaving
/// the earlier adapters already running. An adapter that later fails is
/// logged inside its task; the handle still completes so shutdown can join
/// it.
///
/// `schema` and `validate` are handed to OWP and DDS. STOMP and loopback
/// do not convert or check payloads.
///
/// # Errors
///
/// Returns an error if an enabled adapter's address cannot be resolved, if
/// `owp.on_panic`, `dds.on_panic`, or `stomp.on_panic` is not `abort` or
/// `reconnect`, or if every adapter is disabled.
pub(crate) async fn start(
    config: &Config,
    engine: Arc<Engine>,
    schema: Option<Arc<Schema>>,
    validate: ValidateMode,
    shutdown: CancellationToken,
) -> Result<Vec<JoinHandle<()>>, String> {
    let owp_bind = if config.owp.enabled {
        Some(resolve_addr("owp.bind", &config.owp.bind).await?)
    } else {
        None
    };
    let stomp_broker = if config.stomp.enabled {
        Some(resolve_addr("stomp.broker", &config.stomp.broker).await?)
    } else {
        None
    };

    let mut handles = Vec::new();

    if config.loopback.enabled {
        handles.push(loopback::start(
            &config.loopback,
            Arc::clone(&engine),
            shutdown.clone(),
        ));
    }

    if let Some(bind) = owp_bind {
        handles.push(owp::start(
            &config.owp,
            bind,
            schema.as_ref(),
            validate,
            Arc::clone(&engine),
            shutdown.clone(),
        )?);
    }

    if let Some(broker) = stomp_broker {
        handles.push(stomp::start(
            &config.stomp,
            broker,
            Arc::clone(&engine),
            shutdown.clone(),
        )?);
    }

    if config.dds.enabled {
        handles.push(dds::start(
            &config.dds,
            schema.as_ref(),
            validate,
            engine,
            shutdown,
        )?);
    }

    if handles.is_empty() {
        return Err(
            "no adapters enabled. Add a [loopback], [owp], [stomp], or [dds] section.".into(),
        );
    }

    Ok(handles)
}
