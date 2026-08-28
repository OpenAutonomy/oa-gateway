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

/// A certificate authority for issuing certificates in mutual-TLS tests —
/// `self_signed`'s single cert/key pair can't produce this, since mTLS
/// tests need a CA distinct from the leaf certificates it issues.
pub struct TestCa {
    pub ca_cert_pem: String,
    cert: rcgen::Certificate,
    key_pair: rcgen::KeyPair,
}

/// Generates a self-signed CA for issuing certificates via [`issue`].
///
/// # Panics
///
/// Panics if certificate generation fails.
#[must_use]
pub fn test_ca() -> TestCa {
    let key_pair = rcgen::KeyPair::generate().expect("CA key generation");
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("CA cert params");
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let cert = params
        .self_signed(&key_pair)
        .expect("self-signed CA certificate");
    TestCa {
        ca_cert_pem: cert.pem(),
        cert,
        key_pair,
    }
}

/// Issues a certificate for `names`, signed by `ca` — for a peer that must
/// present a certificate `ca` (or a [`server_tls_with_client_ca`] built
/// from it) will accept.
///
/// # Panics
///
/// Panics if certificate generation fails.
#[must_use]
pub fn issue(ca: &TestCa, names: &[&str]) -> TestCerts {
    let names: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
    let key_pair = rcgen::KeyPair::generate().expect("certificate key generation");
    let params = rcgen::CertificateParams::new(names).expect("certificate params");
    let cert = params
        .signed_by(&key_pair, &ca.cert, &ca.key_pair)
        .expect("CA-signed certificate");
    TestCerts {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
    }
}

/// As [`server_tls`], but also requires and verifies a client certificate
/// issued by `client_ca`.
///
/// # Panics
///
/// Panics if `certs`/`client_ca` does not parse.
#[must_use]
pub fn server_tls_with_client_ca(certs: &TestCerts, client_ca: &TestCa) -> ServerTls {
    server_tls_from_pem(
        "test",
        certs.cert_pem.as_bytes(),
        certs.key_pem.as_bytes(),
        Some(client_ca.ca_cert_pem.as_bytes()),
    )
    .expect("test certificate and client CA should parse")
}

/// A [`ClientTls`] that trusts `certs` and presents a certificate issued by
/// `client_ca` — for a peer requiring mutual TLS.
///
/// # Panics
///
/// Panics if `certs`/`client_ca`/`client_certs` does not parse, or
/// `server_name` is not a usable DNS name or IP address.
#[must_use]
pub fn client_tls_with_client_cert(
    certs: &TestCerts,
    server_name: &str,
    client_certs: &TestCerts,
) -> ClientTls {
    client_tls_from_pem(
        "test",
        Some(certs.cert_pem.as_bytes()),
        server_name,
        Some(client_certs.cert_pem.as_bytes()),
        Some(client_certs.key_pem.as_bytes()),
    )
    .expect("test certificate, server name, and client certificate should be usable")
}
