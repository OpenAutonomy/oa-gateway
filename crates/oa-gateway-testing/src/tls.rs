//! Self-signed certificates for TLS tests.
//!
//! Generated fresh per test run rather than checked into the repository:
//! nothing expires, and no key material — not even a throwaway one — lives
//! in a public repo's git history.

use oa_gateway_adapter::tls::{client_tls_from_pem, server_tls_from_pem, ClientTls, ServerTls};

/// A self-signed certificate and its key, in memory.
///
/// The certificate is its own authority, so trusting it as a peer's leaf
/// certificate and trusting it as a CA amount to the same thing here.
pub struct TestCerts {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Generates a self-signed certificate valid for `names` (hostnames or IP
/// addresses).
///
/// # Panics
///
/// Panics if certificate generation fails, which does not happen for a
/// well-formed name list.
#[must_use]
pub fn self_signed(names: &[&str]) -> TestCerts {
    let names: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(names).expect("self-signed cert generation");
    TestCerts {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
    }
}

/// A [`ServerTls`] presenting `certs`.
///
/// # Panics
///
/// Panics if `certs` does not parse, which does not happen for a pair
/// from [`self_signed`].
#[must_use]
pub fn server_tls(certs: &TestCerts) -> ServerTls {
    server_tls_from_pem(
        "test",
        certs.cert_pem.as_bytes(),
        certs.key_pem.as_bytes(),
        None,
    )
    .expect("test certificate should parse")
}

/// A [`ClientTls`] that trusts `certs` and checks the peer's certificate
/// against `server_name`.
///
/// # Panics
///
/// Panics if `certs` does not parse or `server_name` is not a usable DNS
/// name or IP address.
#[must_use]
pub fn client_tls(certs: &TestCerts, server_name: &str) -> ClientTls {
    client_tls_from_pem(
        "test",
        Some(certs.cert_pem.as_bytes()),
        server_name,
        None,
        None,
    )
    .expect("test certificate and server name should be usable")
}

/// A [`ClientTls`] that trusts a *different* self-signed authority than
/// whatever the server under test presents — for a test asserting that an
/// untrusted certificate is rejected.
///
/// # Panics
///
/// Panics if `server_name` is not a usable DNS name or IP address.
#[must_use]
pub fn untrusted_client_tls(server_name: &str) -> ClientTls {
    let other = self_signed(&[server_name]);
    client_tls(&other, server_name)
}
