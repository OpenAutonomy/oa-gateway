//! Host process: load config, start adapters, wait for Ctrl-C.
//!
//! This binary owns process lifetime only. Protocol work lives in adapter
//! crates; routing lives in [`oa_gateway_core`]. A failed adapter is logged
//! and left down — it does not take the others with it.

mod adapters;
mod addr;
mod cli;
mod config;
mod schema;
mod serve;
mod tls;

use cli::{parse_args, Cli, USAGE};
use serve::serve;

/// Starts the tokio runtime and exits 1 if the host cannot run.
///
/// Failures are printed to stderr as `oa-gateway: …` so a service or
/// container log line names the process without a stack trace for
/// operator mistakes (missing config, bad address).
#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("oa-gateway: {message}");
        std::process::exit(1);
    }
}

/// Dispatches on the command line: help, version, or serve a config.
///
/// # Errors
///
/// Returns a message for a bad command line, or any failure from [`serve()`]
/// when a config path was given.
async fn run() -> Result<(), String> {
    match parse_args(std::env::args().skip(1))? {
        Cli::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Cli::Version => {
            println!("oa-gateway {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Cli::Run(path) => serve(&path).await,
    }
}
