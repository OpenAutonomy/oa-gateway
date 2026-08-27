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

/// The host part of a configured `host:port`, without the port.
///
/// TLS server-name verification checks the name the operator configured,
/// not whatever [`resolve_addr`] resolved it to — reversing a resolved
/// [`SocketAddr`] back to a name would be both wrong (it is not
/// necessarily the name that was configured) and impossible for a
/// round-robin DNS record. Handles the bracketed IPv6 form: `[::1]:61613`
/// is `::1`.
pub(crate) fn host_part(value: &str) -> &str {
    if let Some(rest) = value.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &rest[..end];
        }
    }
    match value.rfind(':') {
        Some(idx) => &value[..idx],
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostname_and_port_split_on_the_last_colon() {
        assert_eq!(host_part("broker.example.com:61613"), "broker.example.com");
    }

    #[test]
    fn an_ipv4_literal_and_port_split_the_same_way() {
        assert_eq!(host_part("127.0.0.1:61613"), "127.0.0.1");
    }

    #[test]
    fn a_bracketed_ipv6_literal_loses_its_brackets_and_port() {
        assert_eq!(host_part("[::1]:61613"), "::1");
        assert_eq!(host_part("[2001:db8::1]:61613"), "2001:db8::1");
    }

    #[test]
    fn a_value_with_no_port_is_returned_whole() {
        assert_eq!(host_part("broker.example.com"), "broker.example.com");
    }
}
