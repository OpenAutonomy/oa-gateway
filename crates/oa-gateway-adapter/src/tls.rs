//! TLS shared by every adapter that rides a plain TCP stream: OWP terminates
//! it as a server, STOMP originates it as a client. Neither DDS nor loopback
//! use this — DDS's RTPS transport is UDP, not a stream, so ordinary TLS does
//! not apply to it.
//!
//! This covers encryption and proving the *server's* identity to whoever
//! connects to it. There is no client certificate support: a peer that
//! completes a handshake here is not authenticated, only talking over an
//! encrypted channel. [`server_tls`]'s [`rustls::ServerConfig::builder`] call
//! is the exact spot a future mutual-TLS change would touch.
//!
//! rustls implements only TLS 1.2 and 1.3, so there is no separate minimum
//! version to configure — this satisfies OMSC-STD-001's own "TLS 1.2 at a
//! minimum" convention for its secure transport variants by construction.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// A stream that may or may not be wrapped in TLS.
///
/// One concrete type flows through an adapter's session regardless of
/// whether TLS is configured, so nothing above the transport (WebSocket
/// framing, STOMP framing) needs to know or care.
pub enum MaybeTlsStream<S> {
    /// TLS is not configured for this connection.
    Plain(S),
    /// This side accepted a TLS handshake (OWP).
    Server(Box<tokio_rustls::server::TlsStream<S>>),
    /// This side originated a TLS handshake (STOMP).
    Client(Box<tokio_rustls::client::TlsStream<S>>),
}

impl<S> MaybeTlsStream<S> {
    /// `"tls"` or `"tcp"`, for one log field at accept/connect time.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Plain(_) => "tcp",
            Self::Server(_) | Self::Client(_) => "tls",
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for MaybeTlsStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Server(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
            Self::Client(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for MaybeTlsStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Server(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            Self::Client(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Server(s) => Pin::new(s.as_mut()).poll_flush(cx),
            Self::Client(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Server(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            Self::Client(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// A configured TLS listener side. Terminates TLS on an accepted connection.
#[derive(Clone)]
pub struct ServerTls {
    acceptor: TlsAcceptor,
}

impl fmt::Debug for ServerTls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerTls").finish_non_exhaustive()
    }
}

impl ServerTls {
    /// Runs the TLS handshake on an accepted connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the handshake fails.
    pub async fn accept<S>(&self, stream: S) -> io::Result<MaybeTlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let stream = self.acceptor.accept(stream).await?;
        Ok(MaybeTlsStream::Server(Box::new(stream)))
    }
}

/// A configured TLS dialer side, plus the name checked in the peer's
/// certificate.
#[derive(Clone)]
pub struct ClientTls {
    connector: TlsConnector,
    server_name: ServerName<'static>,
}

impl fmt::Debug for ClientTls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientTls")
            .field("server_name", &self.server_name)
            .finish_non_exhaustive()
    }
}

impl ClientTls {
    /// Runs the TLS handshake on a connected socket.
    ///
    /// # Errors
    ///
    /// Returns an error if the handshake fails, including when the peer's
    /// certificate does not verify against the configured trust anchors or
    /// does not match the configured server name.
    pub async fn connect<S>(&self, stream: S) -> io::Result<MaybeTlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let stream = self
            .connector
            .connect(self.server_name.clone(), stream)
            .await?;
        Ok(MaybeTlsStream::Client(Box::new(stream)))
    }
}

/// Installs the `ring` crypto provider as the process default, if one is not
/// already installed. Idempotent: a second install attempt is expected once
/// both an OWP and a STOMP adapter configure TLS in the same process, and is
/// silently discarded.
fn install_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Loads a server certificate and key from `cert`/`key` paths, if both are
/// set. Both unset leaves TLS off; exactly one set is a configuration error.
///
/// `key_prefix` names the pair in error messages, e.g. `"owp.tls"` for
/// `owp.tls_cert` / `owp.tls_key`.
///
/// # Errors
///
/// Returns a message naming the offending key if only one of `cert`/`key`
/// is set, if either file cannot be read, or if the certificate and key do
/// not parse or do not match each other.
pub fn server_tls(
    key_prefix: &str,
    cert: Option<&Path>,
    key: Option<&Path>,
) -> Result<Option<ServerTls>, String> {
    match (cert, key) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(format!(
            "{key_prefix}_cert is set but {key_prefix}_key is not. TLS needs both a \
             certificate chain and its private key."
        )),
        (None, Some(_)) => Err(format!(
            "{key_prefix}_key is set but {key_prefix}_cert is not. TLS needs both a \
             certificate chain and its private key."
        )),
        (Some(cert_path), Some(key_path)) => {
            let cert_pem = fs::read(cert_path).map_err(|err| {
                format!(
                    "cannot read {key_prefix}_cert {}: {err}",
                    cert_path.display()
                )
            })?;
            let key_pem = fs::read(key_path).map_err(|err| {
                format!("cannot read {key_prefix}_key {}: {err}", key_path.display())
            })?;
            server_tls_from_pem(key_prefix, &cert_pem, &key_pem).map(Some)
        }
    }
}

/// As [`server_tls`], but parses PEM bytes already in memory rather than
/// reading files — the entry point tests use to avoid touching a filesystem.
///
/// # Errors
///
/// Returns a message if `cert_pem`/`key_pem` do not parse, or the
/// certificate and key do not match each other.
pub fn server_tls_from_pem(
    key_prefix: &str,
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<ServerTls, String> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|err| format!("{key_prefix}_cert is not a valid PEM certificate chain: {err}"))?;
    if certs.is_empty() {
        return Err(format!(
            "{key_prefix}_cert contains no certificates. Expected a PEM chain, leaf \
             certificate first."
        ));
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|err| {
        format!(
            "{key_prefix}_key contains no private key ({err}). Expected a PEM PKCS#8, \
             PKCS#1, or SEC1 key."
        )
    })?;

    install_provider();
    let config = ServerConfig::builder()
        // Mutual TLS is not implemented. To add it, replace this call with
        // `.with_client_cert_verifier(WebPkiClientVerifier::builder(roots).build()?)`.
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|err| format!("{key_prefix}_cert and {key_prefix}_key do not match: {err}"))?;
    Ok(ServerTls {
        acceptor: TlsAcceptor::from(Arc::new(config)),
    })
}

/// Loads a client TLS configuration that verifies a peer as `server_name`.
///
/// `ca` set to a PEM bundle trusts exactly those certificate authorities;
/// `None` trusts the operating system's trust store instead, which is where
/// an organizational CA normally lives.
///
/// # Errors
///
/// Returns a message if `ca` cannot be read or parsed, if the OS trust
/// store cannot be read and no `ca` was given, or if `server_name` is not a
/// usable DNS name or IP address.
pub fn client_tls(
    key_prefix: &str,
    ca: Option<&Path>,
    server_name: &str,
) -> Result<ClientTls, String> {
    let ca_pem = ca
        .map(|path| {
            fs::read(path)
                .map_err(|err| format!("cannot read {key_prefix}_ca {}: {err}", path.display()))
        })
        .transpose()?;
    client_tls_from_pem(key_prefix, ca_pem.as_deref(), server_name)
}

/// As [`client_tls`], but parses PEM bytes already in memory rather than
/// reading a file.
///
/// # Errors
///
/// Returns a message if `ca_pem` does not parse, if the OS trust store
/// cannot be read and `ca_pem` is `None`, or if `server_name` is not a
/// usable DNS name or IP address.
pub fn client_tls_from_pem(
    key_prefix: &str,
    ca_pem: Option<&[u8]>,
    server_name: &str,
) -> Result<ClientTls, String> {
    let mut roots = RootCertStore::empty();
    match ca_pem {
        Some(pem) => {
            let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem)
                .collect::<Result<_, _>>()
                .map_err(|err| {
                    format!("{key_prefix}_ca is not a valid PEM certificate bundle: {err}")
                })?;
            if certs.is_empty() {
                return Err(format!(
                    "{key_prefix}_ca contains no certificates. Expected a PEM bundle of the \
                     certificate authorities the broker's certificate chains to."
                ));
            }
            for cert in certs {
                roots
                    .add(cert)
                    .map_err(|err| format!("{key_prefix}_ca: {err}"))?;
            }
        }
        None => {
            let found = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(found.certs);
            if roots.is_empty() {
                let detail = found
                    .errors
                    .first()
                    .map_or_else(String::new, |err| format!(": {err}"));
                return Err(format!(
                    "cannot read the operating system trust store for {key_prefix}{detail}. \
                     Set {key_prefix}_ca to a PEM bundle instead."
                ));
            }
        }
    }

    let name = ServerName::try_from(server_name.to_owned()).map_err(|err| {
        format!(
            "{key_prefix}_server_name = \"{server_name}\" is not a usable DNS name or IP \
             address ({err})."
        )
    })?;

    install_provider();
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(ClientTls {
        connector: TlsConnector::from(Arc::new(config)),
        server_name: name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A self-signed cert/key for "localhost", generated fresh per test run
    /// rather than checked in — nothing expires, and no key material lives
    /// in the repository, even a throwaway one.
    fn localhost_pair() -> (String, String) {
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        (cert.pem(), key_pair.serialize_pem())
    }

    #[test]
    fn neither_cert_nor_key_leaves_tls_off() {
        assert!(server_tls("owp.tls", None, None).unwrap().is_none());
    }

    #[test]
    fn a_cert_without_a_key_is_refused() {
        let err = server_tls("owp.tls", Some(Path::new("cert.pem")), None).unwrap_err();
        assert!(err.contains("owp.tls_key"), "{err}");
    }

    #[test]
    fn a_key_without_a_cert_is_refused() {
        let err = server_tls("owp.tls", None, Some(Path::new("key.pem"))).unwrap_err();
        assert!(err.contains("owp.tls_cert"), "{err}");
    }

    #[test]
    fn a_matching_cert_and_key_load() {
        let (cert_pem, key_pem) = localhost_pair();
        server_tls_from_pem("owp.tls", cert_pem.as_bytes(), key_pem.as_bytes()).unwrap();
    }

    #[test]
    fn an_empty_cert_is_refused() {
        let (_, key_pem) = localhost_pair();
        let err = server_tls_from_pem("owp.tls", b"", key_pem.as_bytes()).unwrap_err();
        assert!(err.contains("owp.tls_cert"), "{err}");
    }

    #[test]
    fn an_unparsable_key_is_refused() {
        let (cert_pem, _) = localhost_pair();
        let err = server_tls_from_pem("owp.tls", cert_pem.as_bytes(), b"not a key").unwrap_err();
        assert!(err.contains("owp.tls_key"), "{err}");
    }

    #[test]
    fn a_ca_bundle_loads_for_the_client_side() {
        let (cert_pem, _) = localhost_pair();
        client_tls_from_pem("stomp.tls", Some(cert_pem.as_bytes()), "localhost").unwrap();
    }

    #[test]
    fn an_empty_ca_bundle_is_refused() {
        let err = client_tls_from_pem("stomp.tls", Some(b""), "localhost").unwrap_err();
        assert!(err.contains("stomp.tls_ca"), "{err}");
    }

    #[test]
    fn a_bad_server_name_is_refused() {
        let (cert_pem, _) = localhost_pair();
        let err = client_tls_from_pem("stomp.tls", Some(cert_pem.as_bytes()), "not a hostname!")
            .unwrap_err();
        assert!(err.contains("stomp.tls_server_name"), "{err}");
    }
}
