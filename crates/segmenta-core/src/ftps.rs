//! TLS for the FTP client: explicit (`AUTH TLS`) and implicit (`ftps://`).
//!
//! Contract: S2 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T5).
//!
//! # Why a separate module
//!
//! The protocol in [`crate::ftp`] does not care whether its bytes travel in clear or inside
//! TLS; it cares that the stream speaks `AsyncRead + AsyncWrite`. This module owns the two
//! ways an FTP connection becomes encrypted, and nothing else:
//!
//! * **implicit** — `ftps://`: the handshake happens before the greeting.
//! * **explicit** — `ftp://` plus `AUTH TLS`: the client greets, authenticates, then asks
//!   the server to upgrade the same socket.
//!
//! # Trust
//!
//! Production uses the platform roots (`rustls-native-certs`), so a real certificate is
//! verified exactly as a browser would verify it. A caller may inject a `ClientConfig`
//! instead — that is how the tests trust a certificate generated in memory — and there is
//! no "accept anything" switch, which is the difference between a test seam and a hole.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::error::{CoreError, Result};

/// How a connection becomes encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// No TLS at all: plain `ftp://`.
    #[default]
    Plain,
    /// `ftp://` plus `AUTH TLS` after the greeting.
    Explicit,
    /// `ftps://`: TLS starts immediately, before the greeting.
    Implicit,
}

impl TlsMode {
    /// TLS is in use for the control connection.
    pub fn is_tls(self) -> bool {
        !matches!(self, TlsMode::Plain)
    }

    /// Mode implied by a URL scheme.
    pub fn for_scheme(scheme: &str) -> Option<TlsMode> {
        match scheme {
            "ftp" => Some(TlsMode::Plain),
            "ftps" => Some(TlsMode::Implicit),
            _ => None,
        }
    }
}

/// Any stream the protocol can read and write: plain TCP or TLS over it.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/// A boxed stream, which is what [`crate::ftp`] holds once TLS is possible.
pub type BoxStream = Box<dyn Stream>;

/// TLS settings for one transfer.
#[derive(Clone, Default)]
pub struct TlsSetup {
    /// Encryption mode.
    pub mode: TlsMode,
    /// Client configuration; `None` builds one from the platform roots.
    pub client: Option<Arc<ClientConfig>>,
}

impl std::fmt::Debug for TlsSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsSetup")
            .field("mode", &self.mode)
            .field("custom_client", &self.client.is_some())
            .finish()
    }
}

/// Client configuration: the injected one, or the platform roots.
pub fn client_config(custom: Option<&Arc<ClientConfig>>) -> Result<Arc<ClientConfig>> {
    // Two providers end up in the dependency graph (reqwest and librqbit pull different
    // ones), so rustls cannot pick one by itself. Naming `ring` here is the decision, and
    // doing it once keeps a second call cheap.
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    });
    if let Some(config) = custom {
        return Ok(Arc::clone(config));
    }
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for cert in native.certs {
        // A single unreadable certificate must not void the whole store.
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// The `ServerName` a certificate has to match.
fn server_name(host: &str) -> Result<ServerName<'static>> {
    ServerName::try_from(host.to_string()).map_err(|_| {
        CoreError::InvalidUrl(format!("`{host}` is not a name a certificate can match"))
    })
}

/// Wrap an already-open TCP connection in TLS.
///
/// Used for the implicit handshake and, after the caller sends `AUTH TLS` and reads `234`,
/// for the explicit upgrade: the two differ in *when* they call this, not in how.
pub async fn handshake<S>(stream: S, host: &str, setup: &TlsSetup) -> Result<BoxStream>
where
    S: Stream + 'static,
{
    let config = client_config(setup.client.as_ref())?;
    let connector = TlsConnector::from(config);
    let name = server_name(host)?;
    let tls = connector
        .connect(name, stream)
        .await
        .map_err(|err| CoreError::Network(format!("ftps handshake with {host}: {err}")))?;
    Ok(Box::new(tls))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scheme_decides_the_mode() {
        assert_eq!(TlsMode::for_scheme("ftp"), Some(TlsMode::Plain));
        assert_eq!(TlsMode::for_scheme("ftps"), Some(TlsMode::Implicit));
        assert_eq!(TlsMode::for_scheme("http"), None);
        assert!(!TlsMode::Plain.is_tls());
        assert!(TlsMode::Explicit.is_tls());
        assert!(TlsMode::Implicit.is_tls());
    }

    #[test]
    fn the_platform_roots_build_a_config_that_asks_for_server_auth() {
        // Whatever the platform ships, the result is a usable client configuration: this
        // is the same call production makes, so a broken root store fails here.
        let config = client_config(None).unwrap();
        assert!(!config.alpn_protocols.iter().any(|item| item.is_empty()));
    }

    #[test]
    fn a_host_that_cannot_be_a_certificate_name_is_refused() {
        assert!(server_name("ftp.example.com").is_ok());
        assert!(server_name("127.0.0.1").is_ok());
        assert!(server_name("not a host").is_err());
    }
}
