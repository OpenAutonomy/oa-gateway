//! Latency and throughput utility. Calls public gateway APIs only.

mod cli;
mod clock;
mod owp_client;
mod payload;
mod report;
mod scenarios;

use std::process::ExitCode;

use cli::{parse_args, Command, USAGE};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if err == USAGE.trim_end() {
                print!("{USAGE}");
                ExitCode::SUCCESS
            } else {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        }
    }
}

async fn run() -> Result<(), String> {
    let command = parse_args(std::env::args().skip(1))?;
    match command {
        Command::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Command::Engine(args) => scenarios::engine::run(args).await,
        Command::Loopback(args) => scenarios::loopback::run(args).await,
        Command::Owp(args) => scenarios::owp::run(args).await,
        Command::Uci(args) => scenarios::uci::run(args),
        Command::Ping(args) => scenarios::ping::run(args).await,
    }
}
