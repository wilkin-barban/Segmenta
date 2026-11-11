//! URL scheme and private-address validation (OpenSpec 06 §1.2).
//!
//! The engine resolves DNS itself and validates every resolved address before a
//! connection is pinned, so a hosts-file entry or a private DNS answer cannot reach
//! the LAN. Redirects repeat the check on every hop.

use std::net::{IpAddr, SocketAddr};

use tokio::net::lookup_host;
use url::Url;

use crate::error::{CoreError, Result};

/// Whether an address is loopback, private, link-local or otherwise local.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
        }
        IpAddr::V6(v6) => {
            let head = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique local
                || (head & 0xfe00) == 0xfc00
                // fe80::/10 link local
                || (head & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .map(|mapped| is_blocked_ip(IpAddr::V4(mapped)))
                    .unwrap_or(false)
        }
    }
}

/// Only `http` and `https` are downloadable schemes.
pub fn validate_scheme(url: &Url) -> Result<()> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(CoreError::InvalidUrl(format!(
            "unsupported scheme `{other}`: only http and https are accepted"
        ))),
    }
}

/// Resolve `url`'s host and validate every address before a connection is made.
pub async fn resolve_checked(url: &Url, allow_local: bool) -> Result<Vec<SocketAddr>> {
    validate_scheme(url)?;
    let host = url
        .host_str()
        .ok_or_else(|| CoreError::InvalidUrl("missing host".to_string()))?;
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        lookup_host((host, port))
            .await
            .map_err(|err| CoreError::InvalidUrl(format!("{host}: {err}")))?
            .collect()
    };
    if addrs.is_empty() {
        return Err(CoreError::InvalidUrl(format!(
            "{host} did not resolve to any address"
        )));
    }
    if !allow_local {
        if let Some(blocked) = addrs.iter().find(|addr| is_blocked_ip(addr.ip())) {
            return Err(CoreError::BlockedPrivateIp(format!(
                "{host} resolves to {}; enable local downloads to allow it",
                blocked.ip()
            )));
        }
    }
    Ok(addrs)
}
