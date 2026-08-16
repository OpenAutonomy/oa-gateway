//! Resolves configured `host:port` strings to [`std::net::SocketAddr`].
//!
//! Adapters take a resolved address, not a string, so a bad name fails at
//! startup instead of when the first connection is attempted.

use std::net::SocketAddr;

use tracing::info;

/// Resolves a configured address string to a socket address.
///
/// Accepts a literal address or a `host:port` that needs a lookup, so
/// `localhost:9000` works as naturally as `127.0.0.1:9000`.
///
/// When a name offers both families, IPv4 wins. `localhost` resolves to
/// `::1` first on some hosts, and binding there alone would refuse the
/// documented `127.0.0.1:9000` clients.
///
/// `key` is the config field name used in error messages and the
/// resolution log line, not part of the lookup.
///
/// # Errors
///
/// Returns an error if `value` is not a usable address, the lookup fails,
/// or the name resolves to no addresses.
pub(crate) async fn resolve_addr(key: &str, value: &str) -> Result<SocketAddr, String> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let candidates: Vec<SocketAddr> = tokio::net::lookup_host(value)
        .await
        .map_err(|err| {
            format!("{key} = \"{value}\" is not a usable address ({err}). Expected host:port, such as 127.0.0.1:9000.")
        })?
        .collect();
    let addr = candidates
        .iter()
        .find(|addr| addr.is_ipv4())
        .or_else(|| candidates.first())
        .copied()
        .ok_or_else(|| format!("{key} = \"{value}\" resolved to no addresses"))?;
    info!(%key, value, %addr, "resolved address");
    Ok(addr)
}
