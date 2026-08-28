//! Reads the TLS material named in the config before any adapter listens.
//!
//! Doing this here, once, keeps a missing file or a mismatched cert/key pair
//! from being discovered only once a client tries to connect.

use oa_gateway_adapter::tls::{client_tls, server_tls, ClientTls, ServerTls};

use crate::addr::host_part;
use crate::config::Config;

/// TLS material the host built before any adapter touched a socket.
#[derive(Debug)]
pub(crate) struct HostTls {
    pub(crate) owp: Option<ServerTls>,
    pub(crate) stomp: Option<ClientTls>,
}

/// Reads the certificates and keys named in `config`, if any.
///
/// A disabled adapter's TLS settings are not read, matching how
/// [`crate::adapters::start`] only resolves addresses for enabled adapters.
///
/// # Errors
///
/// Returns an error if `owp.tls_cert`/`owp.tls_key` is set without its
/// pair, if a cert/key/CA file cannot be read, if a certificate does not
/// parse or does not match its key, or if `stomp.tls_server_name` (or the
/// host part of `stomp.broker`, when that is empty) is not a usable DNS
/// name or IP address.
pub(crate) fn load(config: &Config) -> Result<HostTls, String> {
    let owp = if config.owp.enabled {
        let cert = non_empty_path(&config.owp.tls_cert);
        let key = non_empty_path(&config.owp.tls_key);
        server_tls("owp.tls", cert, key, None)?
    } else {
        None
    };
    let stomp = if config.stomp.enabled && config.stomp.tls {
        let ca = non_empty_path(&config.stomp.tls_ca);
        let name = if config.stomp.tls_server_name.is_empty() {
            host_part(&config.stomp.broker)
        } else {
            &config.stomp.tls_server_name
        };
        Some(client_tls("stomp.tls", ca, name, None, None)?)
    } else {
        None
    };
    Ok(HostTls { owp, stomp })
}

fn non_empty_path(value: &str) -> Option<&std::path::Path> {
    if value.is_empty() {
        None
    } else {
        Some(std::path::Path::new(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tls_configured_leaves_owp_plaintext() {
        let config: Config = toml::from_str("[owp]\nenabled = true\n").unwrap();
        assert!(load(&config).unwrap().owp.is_none());
    }

    #[test]
    fn a_disabled_owp_adapter_is_not_checked_for_tls() {
        // enabled = false with only tls_cert set would otherwise be refused;
        // a disabled adapter's TLS settings should not even be read.
        let config: Config =
            toml::from_str("[owp]\nenabled = false\ntls_cert = \"definitely/not/here.pem\"\n")
                .unwrap();
        assert!(load(&config).unwrap().owp.is_none());
    }

    #[test]
    fn a_cert_without_a_key_is_refused_at_startup() {
        let config: Config =
            toml::from_str("[owp]\nenabled = true\ntls_cert = \"definitely/not/here.pem\"\n")
                .unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("owp.tls_key"), "{err}");
    }

    #[test]
    fn an_unreadable_cert_path_names_the_file() {
        let config: Config = toml::from_str(
            "[owp]\nenabled = true\ntls_cert = \"definitely/not/here.pem\"\ntls_key = \"definitely/not/here-key.pem\"\n",
        )
        .unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("definitely/not/here.pem"), "{err}");
    }

    #[test]
    fn a_matching_cert_and_key_are_loaded() {
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

        let dir = std::env::temp_dir().join("oa-gateway-tls-test");
        std::fs::create_dir_all(&dir).unwrap();
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();

        let config: Config = toml::from_str(&format!(
            "[owp]\nenabled = true\ntls_cert = {:?}\ntls_key = {:?}\n",
            cert_path.display().to_string(),
            key_path.display().to_string(),
        ))
        .unwrap();
        assert!(load(&config).unwrap().owp.is_some());

        std::fs::remove_file(&cert_path).ok();
        std::fs::remove_file(&key_path).ok();
    }

    #[test]
    fn no_stomp_tls_configured_leaves_it_plaintext() {
        let config: Config = toml::from_str("[stomp]\nenabled = true\n").unwrap();
        assert!(load(&config).unwrap().stomp.is_none());
    }

    #[test]
    fn a_disabled_stomp_adapter_is_not_checked_for_tls() {
        let config: Config = toml::from_str(
            "[stomp]\nenabled = false\ntls = true\ntls_ca = \"definitely/not/here.pem\"\n",
        )
        .unwrap();
        assert!(load(&config).unwrap().stomp.is_none());
    }

    #[test]
    fn stomp_tls_off_ignores_a_configured_ca() {
        // tls = false is the switch; a leftover tls_ca must not be checked.
        let config: Config = toml::from_str(
            "[stomp]\nenabled = true\ntls = false\ntls_ca = \"definitely/not/here.pem\"\n",
        )
        .unwrap();
        assert!(load(&config).unwrap().stomp.is_none());
    }

    #[test]
    fn an_unreadable_stomp_ca_path_names_the_file() {
        let config: Config = toml::from_str(
            "[stomp]\nenabled = true\ntls = true\ntls_ca = \"definitely/not/here.pem\"\n",
        )
        .unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("stomp.tls_ca"), "{err}");
        assert!(err.contains("definitely/not/here.pem"), "{err}");
    }

    #[test]
    fn stomp_tls_server_name_defaults_to_the_broker_host_not_the_stomp_host_header() {
        // An unparseable broker host surfaces in the error, proving it is
        // what got used as the default server name — and `host = "/"`
        // (the STOMP protocol header, not a hostname) is set alongside it
        // to confirm that is not what was parsed instead.
        let config: Config = toml::from_str(
            "[stomp]\nenabled = true\ntls = true\nbroker = \"not a hostname!:61612\"\nhost = \"/\"\n",
        )
        .unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("not a hostname!"), "{err}");
    }

    #[test]
    fn an_explicit_stomp_server_name_overrides_the_broker_host() {
        let config: Config = toml::from_str(
            "[stomp]\nenabled = true\ntls = true\nbroker = \"127.0.0.1:61612\"\ntls_server_name = \"not a hostname!\"\n",
        )
        .unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("stomp.tls_server_name"), "{err}");
    }
}
