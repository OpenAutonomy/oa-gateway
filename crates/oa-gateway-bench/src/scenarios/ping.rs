//! One message across OWP: INIT, SUB `Ping` on `demo`, PUB, wait for MSG.

use std::sync::Arc;

use oa_gateway_core::Engine;
use oa_gateway_owp::ServerOp;
use oa_gateway_testing::owp::start_owp;

use crate::cli::PingArgs;
use crate::owp_client;

const PING_SID: &str = "ping-1";
const PING_PUB: &str = r#"PUB demo {"Ping":{"n":1}}"#;

/// Runs the ping smoke.
///
/// Handshake schema is `002.5.0`. `--url` attaches to a running
/// gateway; without it, an in-process OWP adapter is started the same
/// way as the `owp` scenario.
///
/// # Errors
///
/// Returns a message if the socket path fails, the server refuses a
/// frame, or no `MSG` arrives.
pub(crate) async fn run(args: PingArgs) -> Result<(), String> {
    let (url, shutdown) = if let Some(url) = &args.url {
        (url.clone(), None)
    } else {
        let (url, token) = start_owp(Arc::new(Engine::new()), false).await;
        (url, Some(token))
    };

    let mut ws = owp_client::connect(&url).await?;
    owp_client::handshake(&mut ws, true).await?;
    owp_client::subscribe(&mut ws, PING_SID, "Ping", "demo", true).await?;
    owp_client::send_text(&mut ws, PING_PUB).await?;

    loop {
        match owp_client::recv_op(&mut ws).await? {
            ServerOp::Msg { sid, payload } => {
                if sid != PING_SID {
                    return Err(format!("expected MSG {PING_SID}, got sid {sid}"));
                }
                println!("received {payload}");
                break;
            }
            ServerOp::Ok => {}
            ServerOp::Err { error, details } => {
                return Err(match details {
                    Some(d) => format!("owp -ERR {error} {d}"),
                    None => format!("owp -ERR {error}"),
                });
            }
            other => return Err(format!("expected MSG after PUB, got {other}")),
        }
    }

    if let Some(token) = shutdown {
        token.cancel();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn attaches_to_in_process_owp() {
        let (url, shutdown) = start_owp(Arc::new(Engine::new()), false).await;
        run(PingArgs { url: Some(url) })
            .await
            .expect("ping against in-process OWP");
        shutdown.cancel();
    }
}
