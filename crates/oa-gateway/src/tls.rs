//! Reads the TLS material named in the config before any adapter listens.
//!
//! Doing this here, once, keeps a missing file or a mismatched cert/key pair
//! from being discovered only once a client tries to connect.

use oa_gateway_adapter::tls::{server_tls, ServerTls};

use crate::config::Config;

/// TLS material the host built before any adapter touched a socket.
#[derive(Debug)]
pub(crate) struct HostTls {
    pub(crate) owp: Option<ServerTls>,
}

/// Reads the certificates and keys named in `config`, if any.
///
/// A disabled adapter's TLS settings are not read, matching how
/// [`crate::adapters::start`] only resolves addresses for enabled adapters.
///
/// # Errors
///
/// Returns an error if `owp.tls_cert`/`owp.tls_key` is set without its
/// pair, if either file cannot be read, or if the certificate and key do
/// not parse or do not match each other.
pub(crate) fn load(config: &Config) -> Result<HostTls, String> {
    let owp = if config.owp.enabled {
        let cert = non_empty_path(&config.owp.tls_cert);
        let key = non_empty_path(&config.owp.tls_key);
        server_tls("owp.tls", cert, key)?
    } else {
        None
    };
    Ok(HostTls { owp })
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
}
