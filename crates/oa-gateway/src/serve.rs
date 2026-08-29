//! Runs the host until Ctrl-C, then cancels every adapter and waits for them.
//!
//! Startup is sequenced so a bad config, schema, or address fails before
//! anything accepts traffic. Shutdown is cooperative: adapters observe the
//! cancellation token in their own `run` loops.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oa_gateway_core::Engine;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::adapters;
use crate::config;
use crate::schema;
use crate::tls;

/// Loads `config_path`, starts the enabled adapters, and blocks until Ctrl-C.
///
/// Schema compilation and address resolution happen before any adapter
/// task is spawned, so a bad input exits cleanly instead of leaving an
/// earlier adapter already listening.
///
/// On Ctrl-C the cancellation token is fired and every adapter task is
/// joined. An adapter that failed earlier has already been logged; a join
/// error here is ignored so one panicked task cannot block the rest of
/// shutdown.
///
/// # Errors
///
/// Returns an error if the config cannot be loaded, the schema or
/// validation mode is unusable, `stomp.on_panic` is not `abort` or
/// `reconnect`, `dds.provider` or `dds.qos` is unusable, no adapters
/// are enabled, an address cannot be resolved, or the process cannot
/// listen for Ctrl-C.
pub(crate) async fn serve(config_path: &Path) -> Result<(), String> {
    init_tracing();

    let config = config::load(config_path)?;

    // Compile the schema and read any TLS material before anything starts
    // listening, for the same reason addresses are resolved up front: a bad
    // input should fail cleanly rather than after adapters are already
    // accepting traffic.
    let schema = schema::load(&config)?;
    let validate = config.uci.validate_mode()?;
    if schema.is_some() {
        info!(mode = %validate, "schema validation");
    }
    let host_tls = tls::load(&config)?;

    let engine = Arc::new(Engine::new());
    let shutdown = CancellationToken::new();
    let mut handles = adapters::start(
        &config,
        engine.clone(),
        schema,
        validate,
        host_tls,
        shutdown.clone(),
    )
    .await?;
    if let Some(ticker) = stats_ticker(
        Arc::clone(&engine),
        config.engine.stats_interval_secs,
        shutdown.clone(),
    ) {
        handles.push(ticker);
    }

    info!("oa-gateway running — Ctrl-C to stop");
    tokio::signal::ctrl_c()
        .await
        .map_err(|err| format!("cannot listen for Ctrl-C: {err}"))?;
    info!("shutdown requested");
    shutdown.cancel();
    for handle in handles {
        let _ = handle.await;
    }
    Ok(())
}

/// Installs the process-wide tracing subscriber.
///
/// Honors `RUST_LOG` when set; otherwise filters at `info`, which is the
/// level the usage text documents.
/// Logs [`Engine`] counters on `interval_secs`. `0` starts nothing.
fn stats_ticker(
    engine: Arc<Engine>,
    interval_secs: u64,
    shutdown: CancellationToken,
) -> Option<JoinHandle<()>> {
    if interval_secs == 0 {
        return None;
    }
    let interval = Duration::from_secs(interval_secs);
    Some(tokio::spawn(async move {
        let mut last_dropped = 0u64;
        loop {
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(interval) => {
                    let stats = engine.stats();
                    let dropped = stats.dropped();
                    info!(
                        published = stats.published(),
                        delivered = stats.delivered(),
                        dropped,
                        "engine stats"
                    );
                    if dropped > last_dropped {
                        warn!(
                            dropped,
                            since_last = dropped - last_dropped,
                            "engine dropped deliveries since the last sample"
                        );
                    }
                    last_dropped = dropped;
                }
            }
        }
    }))
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}
