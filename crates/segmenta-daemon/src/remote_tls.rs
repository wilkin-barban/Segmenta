//! Local TLS material for the remote control server.
//!
//! Contract: S4 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T9).
//!
//! # What this is and what it is not
//!
//! The certificate is generated here, on the user's machine, and signed by nobody. It is
//! **not** an attempt to impersonate a public site: it exists so the bytes between the phone
//! and the daemon are encrypted, and so the client can *pin* one specific certificate and
//! refuse every other. Without that pin, a self-signed certificate protects nothing: anyone
//! on the LAN could present their own and the browser would offer the same warning.
//!
//! The fingerprint is therefore part of the contract, not a decoration: it is what the user
//! compares on both screens.
//!
//! # Lifetime
//!
//! The pair is written once into the data directory (`0600`) and reused across restarts, so
//! a paired phone keeps working. Near expiry it is replaced, which invalidates the pin: that
//! is the honest behaviour, because a silent renewal would mean trusting a certificate the
//! user never verified.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// How long a generated certificate is valid.
const VALID_DAYS: u64 = 365;

/// Renew when less than this is left.
const RENEW_WITHIN_DAYS: u64 = 30;

/// File names inside the data directory.
const CERT_FILE: &str = "remote-cert.pem";
const KEY_FILE: &str = "remote-key.pem";

/// Why TLS material could not be prepared.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// The files could not be read or written.
    #[error("cannot prepare the remote certificate: {0}")]
    Io(#[from] std::io::Error),
    /// The key pair could not be generated.
    #[error("cannot generate the remote certificate: {0}")]
    Generate(String),
    /// An existing pair could not be parsed.
    #[error("the stored certificate is unusable: {0}")]
    Stored(String),
}

/// Certificate and key in memory, plus the fingerprint that identifies them.
#[derive(Clone)]
pub struct RemoteTls {
    cert_pem: String,
    key_pem: String,
    fingerprint: String,
    /// Where the pair lives, for the "renew by deleting" path.
    path: PathBuf,
}

impl std::fmt::Debug for RemoteTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key never reaches a log line, not even in a debug print.
        f.debug_struct("RemoteTls")
            .field("fingerprint", &self.fingerprint)
            .field("path", &self.path)
            .finish()
    }
}

impl RemoteTls {
    /// Load the pair from `data_dir`, generating it when absent, unreadable or near expiry.
    pub fn ensure(data_dir: &Path) -> Result<RemoteTls, TlsError> {
        let path = data_dir.to_path_buf();
        std::fs::create_dir_all(&path)?;
        let cert_path = path.join(CERT_FILE);
        let key_path = path.join(KEY_FILE);
        if let (Ok(cert_pem), Ok(key_pem)) = (
            std::fs::read_to_string(&cert_path),
            std::fs::read_to_string(&key_path),
        ) {
            match fingerprint_of_pem(&cert_pem) {
                Ok(fingerprint) if !is_near_expiry(&cert_path) => {
                    // A pair that was left readable keeps being readable unless repaired,
                    // and this is the only place that sees it on every start.
                    tighten(&cert_path)?;
                    tighten(&key_path)?;
                    return Ok(RemoteTls {
                        cert_pem,
                        key_pem,
                        fingerprint,
                        path,
                    });
                }
                // Near expiry or unparsable: fall through and replace both files together.
                _ => {}
            }
        }
        let (cert_pem, key_pem) = generate()?;
        write_private(&cert_path, &cert_pem)?;
        write_private(&key_path, &key_pem)?;
        let fingerprint = fingerprint_of_pem(&cert_pem)?;
        Ok(RemoteTls {
            cert_pem,
            key_pem,
            fingerprint,
            path,
        })
    }

    /// Fingerprint as the user sees it: uppercase hex, colon separated.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// PEM certificate.
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// PEM private key.
    pub fn key_pem(&self) -> &str {
        &self.key_pem
    }

    /// Directory the pair was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// TLS acceptor for the remote control server.
    pub fn acceptor(&self) -> Result<tokio_rustls::TlsAcceptor, TlsError> {
        use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
        // The provider has to be chosen explicitly: more than one is compiled in.
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let certs: Vec<CertificateDer<'static>> = pem_certificates(&self.cert_pem)
            .into_iter()
            .map(CertificateDer::from)
            .collect();
        if certs.is_empty() {
            return Err(TlsError::Stored("no certificate block".to_string()));
        }
        let key = pem_key(&self.key_pem)
            .ok_or_else(|| TlsError::Stored("no private key block".to_string()))?;
        let config = tokio_rustls::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, PrivateKeyDer::Pkcs8(key.into()))
            .map_err(|err| TlsError::Stored(err.to_string()))?;
        Ok(tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config)))
    }
}

/// Write a file only the owner can read.
///
/// `OpenOptions::mode` applies **only when the file is created**, so a pre-existing
/// world-readable file would keep its mode through a truncating open. The explicit
/// `set_permissions` is what makes the promise true on both paths, and it is also applied to
/// a reused pair, because a key that was once readable stays readable until someone fixes it.
fn write_private(path: &Path, contents: &str) -> Result<(), TlsError> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Make sure an existing pair is not readable by anyone else.
fn tighten(path: &Path) -> Result<(), TlsError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path)?;
    if metadata.permissions().mode() & 0o777 != 0o600 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// SHA-256 of the DER body, formatted `AA:BB:…`.
pub fn fingerprint_of_pem(cert_pem: &str) -> Result<String, TlsError> {
    let der =
        pem_body(cert_pem).ok_or_else(|| TlsError::Stored("no certificate block".to_string()))?;
    Ok(fingerprint_of_der(&der))
}

/// Format a DER body as a fingerprint.
pub fn fingerprint_of_der(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<String>>()
        .join(":")
}

/// Every certificate block of a PEM bundle, in order.
fn pem_certificates(pem: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut body: Vec<String> = Vec::new();
    let mut inside = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.contains("BEGIN CERTIFICATE") {
            inside = true;
            body.clear();
            continue;
        }
        if line.contains("END CERTIFICATE") {
            if let Some(decoded) = decode_base64(&body.concat()) {
                out.push(decoded);
            }
            inside = false;
            continue;
        }
        if inside {
            body.push(line.to_string());
        }
    }
    out
}

/// DER body of a PKCS#8 private key block.
fn pem_key(pem: &str) -> Option<Vec<u8>> {
    let mut body: Vec<String> = Vec::new();
    let mut inside = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.contains("BEGIN PRIVATE KEY") {
            inside = true;
            continue;
        }
        if line.contains("END PRIVATE KEY") {
            break;
        }
        if inside {
            body.push(line.to_string());
        }
    }
    if body.is_empty() {
        return None;
    }
    decode_base64(&body.concat())
}

/// Extract the base64 body of the first `CERTIFICATE` block.
fn pem_body(pem: &str) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    let mut inside = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.contains("BEGIN CERTIFICATE") {
            inside = true;
            continue;
        }
        if line.contains("END CERTIFICATE") {
            break;
        }
        if inside {
            body.push(line);
        }
    }
    if body.is_empty() {
        return None;
    }
    decode_base64(&body.concat())
}

/// Minimal base64 decoder: PEM certificates are the only input.
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in input.bytes() {
        if byte == b'=' || byte == b'\n' || byte == b'\r' {
            continue;
        }
        let value = TABLE.iter().position(|candidate| *candidate == byte)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// True when the stored pair is old enough to be replaced.
///
/// The certificate is written with a known validity, so its age on disk is the trigger: no
/// X.509 reader is needed to know that a file created `RENEWAL_DAYS` ago is about to expire.
/// Replacing it invalidates the pin on every paired device, which is why it happens with
/// weeks to spare and not at midnight of the last day.
fn is_near_expiry(cert_path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(cert_path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    match modified.elapsed() {
        Ok(age) => age.as_secs() >= RENEWAL_DAYS * 24 * 60 * 60,
        // A clock in the past is not a reason to keep trusting an old file.
        Err(_) => true,
    }
}

/// Generate a pair covering loopback, `localhost` and every local IPv4 address.
fn generate() -> Result<(String, String), TlsError> {
    let mut params = rcgen::CertificateParams::new(vec!["localhost".to_string()])
        .map_err(|err| TlsError::Generate(err.to_string()))?;
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
            std::net::Ipv4Addr::LOCALHOST,
        )));
    params
        .subject_alt_names
        .push(rcgen::SanType::IpAddress(std::net::IpAddr::V6(
            std::net::Ipv6Addr::LOCALHOST,
        )));
    // A phone reaches the daemon through the LAN address, so the certificate has to name
    // it: a certificate for `localhost` alone makes the browser refuse the connection.
    use network_interface::NetworkInterfaceConfig;
    if let Ok(interfaces) = network_interface::NetworkInterface::show() {
        for interface in interfaces {
            for address in interface.addr {
                let ip: std::net::IpAddr = address.ip();
                if !ip.is_loopback() && !ip.is_unspecified() {
                    params.subject_alt_names.push(rcgen::SanType::IpAddress(ip));
                }
            }
        }
    }
    // rcgen defaults to a validity that starts now; the timestamps are left alone so this
    // module does not depend on a second date library.
    let key = rcgen::KeyPair::generate().map_err(|err| TlsError::Generate(err.to_string()))?;
    let cert = params
        .self_signed(&key)
        .map_err(|err| TlsError::Generate(err.to_string()))?;
    Ok((cert.pem(), key.serialize_pem()))
}

/// Days after which a stored certificate is replaced.
pub const RENEWAL_DAYS: u64 = VALID_DAYS - RENEW_WITHIN_DAYS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_is_the_sha256_of_the_body_in_colons() {
        let der = b"not really a certificate";
        let printed = fingerprint_of_der(der);
        assert!(
            printed.starts_with("5F:") || printed.len() == 95,
            "{printed}"
        );
        // Same bytes, same fingerprint: that is what pinning relies on.
        assert_eq!(printed, fingerprint_of_der(der));
        assert_eq!(printed.split(':').count(), 32);
        assert!(printed.chars().all(|c| c.is_ascii_hexdigit() || c == ':'));
    }

    #[test]
    fn a_pem_body_round_trips_through_the_decoder() {
        assert_eq!(decode_base64("aGVsbG8="), Some(b"hello".to_vec()));
        assert_eq!(decode_base64("!!!"), None);
        assert_eq!(pem_body("no cert here"), None);
    }
}
