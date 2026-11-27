//! Encrypted DNS: DoH queries whose responses still pass the SSRF guards.
//!
//! Contract: T4 of `odd/tasks/segmenta-hito1-red-inmunidad.md`.
//!
//! # Scope
//!
//! `dns_mode` accepts `system`, `doh` and `dot`. **DoT is not available in this build**:
//! it needs a TLS client of its own plus a test certificate, and returning a typed error is
//! better than pretending it resolves. `doh` speaks RFC 8484 (`application/dns-message`)
//! against Cloudflare, Quad9, Google or a custom URL.
//!
//! # Guards
//!
//! The resolver is only a different transport. Its answers go through the same
//! `urlssrf::is_blocked_ip` check as the system resolver, so a hostile or compromised
//! endpoint cannot point a download at loopback or the LAN.
//!
//! # Interface
//!
//! With a VPN configured the query is sent from a socket bound to that interface, so the
//! lookup itself does not leak which sites the user visits.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Resolver the daemon uses for names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DnsMode {
    /// The operating system's resolver.
    #[default]
    System,
    /// DNS over HTTPS (RFC 8484).
    Doh,
    /// DNS over TLS; recognized but not available in this build.
    Dot,
}

impl DnsMode {
    /// Parse the `dns_mode` setting.
    pub fn parse(value: &str) -> Option<DnsMode> {
        match value.trim().to_ascii_lowercase().as_str() {
            "system" | "" => Some(DnsMode::System),
            "doh" => Some(DnsMode::Doh),
            "dot" => Some(DnsMode::Dot),
            _ => None,
        }
    }

    /// Stable name used by the settings and the IPC.
    pub fn name(self) -> &'static str {
        match self {
            DnsMode::System => "system",
            DnsMode::Doh => "doh",
            DnsMode::Dot => "dot",
        }
    }
}

/// Why a lookup did not produce an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsError {
    /// The mode needs something this build does not include.
    Unsupported(&'static str),
    /// The endpoint answered with something that is not a valid DNS message.
    Malformed,
    /// The endpoint could not be reached.
    Unreachable,
    /// The answer was blocked by the SSRF rules.
    Blocked,
}

impl DnsError {
    /// Stable code for the task record.
    pub fn code(&self) -> &'static str {
        match self {
            DnsError::Unsupported(_) => "ERR_DNS_UNSUPPORTED",
            DnsError::Malformed => "ERR_DNS_MALFORMED",
            DnsError::Unreachable => "ERR_DNS_UNREACHABLE",
            DnsError::Blocked => "ERR_DNS_BLOCKED",
        }
    }

    /// Plain sentence, never echoing the query or the endpoint credentials.
    pub fn message(&self) -> String {
        match self {
            DnsError::Unsupported(what) => format!("{what} is not available in this build"),
            DnsError::Malformed => "the resolver answered an invalid message".to_string(),
            DnsError::Unreachable => "the resolver could not be reached".to_string(),
            DnsError::Blocked => "the resolver answered a blocked address".to_string(),
        }
    }
}

/// Known DoH endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DohProvider {
    /// Cloudflare.
    #[default]
    Cloudflare,
    /// Quad9.
    Quad9,
    /// Google.
    Google,
}

impl DohProvider {
    /// Parse the `dns_provider` setting.
    pub fn parse(value: &str) -> Option<DohProvider> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cloudflare" | "" => Some(DohProvider::Cloudflare),
            "quad9" => Some(DohProvider::Quad9),
            "google" => Some(DohProvider::Google),
            _ => None,
        }
    }

    /// Endpoint URL.
    pub fn url(self) -> &'static str {
        match self {
            DohProvider::Cloudflare => "https://cloudflare-dns.com/dns-query",
            DohProvider::Quad9 => "https://dns.quad9.net/dns-query",
            DohProvider::Google => "https://dns.google/dns-query",
        }
    }
}

/// Configuration the resolver reads at query time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsConfig {
    /// Selected mode.
    pub mode: DnsMode,
    /// Endpoint when the mode is `doh`.
    pub endpoint: String,
    /// Interface the query must be bound to, if any.
    pub interface: Option<String>,
    /// Whether a private answer is acceptable (local downloads enabled).
    pub allow_local: bool,
}

impl Default for DnsConfig {
    fn default() -> Self {
        DnsConfig {
            mode: DnsMode::System,
            endpoint: DohProvider::default().url().to_string(),
            interface: None,
            allow_local: false,
        }
    }
}

/// Configuration in force, published by the daemon once per settings change.
static ACTIVE: std::sync::Mutex<Option<DnsConfig>> = std::sync::Mutex::new(None);

/// Publish the configuration the resolver uses.
pub fn set_config(config: Option<DnsConfig>) {
    *ACTIVE.lock().unwrap_or_else(|err| err.into_inner()) = config;
}

/// Resolve `host` with the configuration in force, falling back to the system resolver.
///
/// Returns an empty vector when the mode is `system`, which tells
/// [`crate::urlssrf::resolve_checked`] to keep using the ordinary lookup.
pub async fn resolve_host(host: &str) -> Result<Vec<IpAddr>, DnsError> {
    let config = ACTIVE
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone()
        .unwrap_or_default();
    match config.mode {
        DnsMode::System => Ok(Vec::new()),
        DnsMode::Dot => Err(DnsError::Unsupported("DNS over TLS")),
        DnsMode::Doh => {
            let ips = query_doh(&config, host).await?;
            if ips.is_empty() {
                return Err(DnsError::Malformed);
            }
            if !config.allow_local && ips.iter().any(|ip| crate::urlssrf::is_blocked_ip(*ip)) {
                return Err(DnsError::Blocked);
            }
            Ok(ips)
        }
    }
}

/// Send one wire-format query and parse the addresses out of the answer.
pub async fn query_doh(config: &DnsConfig, host: &str) -> Result<Vec<IpAddr>, DnsError> {
    let query = build_query(host).ok_or(DnsError::Malformed)?;
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10));
    if let Some(interface) = config.interface.as_deref() {
        crate::netguard::ensure_up(interface).map_err(|_| DnsError::Unreachable)?;
        builder = builder.interface(interface);
    }
    let client = builder.build().map_err(|_| DnsError::Unreachable)?;
    let response = client
        .post(&config.endpoint)
        .header("content-type", "application/dns-message")
        .header("accept", "application/dns-message")
        .body(query)
        .send()
        .await
        .map_err(|_| DnsError::Unreachable)?;
    if !response.status().is_success() {
        return Err(DnsError::Unreachable);
    }
    let body = response.bytes().await.map_err(|_| DnsError::Unreachable)?;
    parse_response(&body)
}

/// Build a minimal `A` query for `host`.
pub fn build_query(host: &str) -> Option<Vec<u8>> {
    let host = host.trim_end_matches('.');
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    let mut packet = Vec::with_capacity(12 + host.len() + 6);
    packet.extend_from_slice(&0x1234u16.to_be_bytes()); // id
    packet.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
    packet.extend_from_slice(&1u16.to_be_bytes()); // questions
    packet.extend_from_slice(&0u16.to_be_bytes()); // answers
    packet.extend_from_slice(&0u16.to_be_bytes()); // authority
    packet.extend_from_slice(&0u16.to_be_bytes()); // additional
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&1u16.to_be_bytes()); // A
    packet.extend_from_slice(&1u16.to_be_bytes()); // IN
    Some(packet)
}

/// Read the `A`/`AAAA` records of a response, skipping over names and compression.
pub fn parse_response(packet: &[u8]) -> Result<Vec<IpAddr>, DnsError> {
    if packet.len() < 12 {
        return Err(DnsError::Malformed);
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_name(packet, offset)?;
        offset += 4; // type + class
        if offset > packet.len() {
            return Err(DnsError::Malformed);
        }
    }
    let mut addresses = Vec::new();
    for _ in 0..answers {
        offset = skip_name(packet, offset)?;
        if offset + 10 > packet.len() {
            return Err(DnsError::Malformed);
        }
        let kind = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        let data = packet
            .get(offset..offset + length)
            .ok_or(DnsError::Malformed)?;
        match (kind, length) {
            (1, 4) => addresses.push(IpAddr::V4(Ipv4Addr::new(
                data[0], data[1], data[2], data[3],
            ))),
            (28, 16) => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(data);
                addresses.push(IpAddr::V6(Ipv6Addr::from(octets)));
            }
            _ => {}
        }
        offset += length;
    }
    Ok(addresses)
}

/// Advance past a (possibly compressed) name.
fn skip_name(packet: &[u8], mut offset: usize) -> Result<usize, DnsError> {
    loop {
        let length = *packet.get(offset).ok_or(DnsError::Malformed)?;
        if length == 0 {
            return Ok(offset + 1);
        }
        if length & 0xC0 == 0xC0 {
            // Compression pointer: two bytes and the name continues elsewhere.
            return Ok(offset + 2);
        }
        offset += 1 + length as usize;
        if offset > packet.len() {
            return Err(DnsError::Malformed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The active configuration is process-wide, so the tests that publish one must not
    /// overlap with each other.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|err| err.into_inner())
    }

    #[test]
    fn modes_and_providers_parse_and_fail_closed() {
        assert_eq!(DnsMode::parse("system"), Some(DnsMode::System));
        assert_eq!(DnsMode::parse("DoH"), Some(DnsMode::Doh));
        assert_eq!(DnsMode::parse("dot"), Some(DnsMode::Dot));
        assert_eq!(DnsMode::parse("dns-over-ftp"), None);
        assert_eq!(DohProvider::parse("quad9"), Some(DohProvider::Quad9));
        assert_eq!(DohProvider::parse("google"), Some(DohProvider::Google));
        assert_eq!(DohProvider::parse("custom"), None);
        assert!(DohProvider::Cloudflare.url().starts_with("https://"));
    }

    #[test]
    fn the_query_carries_the_name_and_the_a_record_type() {
        let query = build_query("example.test").unwrap();
        assert_eq!(&query[12..14], &[7, b'e']);
        assert!(query.ends_with(&[0, 0, 1, 0, 1]));
        assert_eq!(build_query(""), None);
        assert_eq!(build_query(&"a".repeat(300)), None);
    }

    #[test]
    fn a_response_yields_only_address_records() {
        // Header: one question, one answer; the answer is 93.184.216.34 for example.test.
        let mut packet = vec![
            0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        ];
        packet.extend_from_slice(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        packet.extend_from_slice(&[4, b't', b'e', b's', b't', 0]);
        packet.extend_from_slice(&[0, 1, 0, 1]);
        packet.extend_from_slice(&[0xC0, 0x0C]); // pointer to the question name
        packet.extend_from_slice(&[0, 1, 0, 1]);
        packet.extend_from_slice(&0u32.to_be_bytes()); // ttl
        packet.extend_from_slice(&4u16.to_be_bytes()); // rdlength
        packet.extend_from_slice(&[93, 184, 216, 34]);
        assert_eq!(
            parse_response(&packet).unwrap(),
            vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]
        );
        assert_eq!(parse_response(&[0, 1, 2]), Err(DnsError::Malformed));
    }

    #[test]
    fn dot_is_declared_unavailable_instead_of_faking_a_lookup() {
        let _guard = serial();
        set_config(Some(DnsConfig {
            mode: DnsMode::Dot,
            ..DnsConfig::default()
        }));
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(resolve_host("example.test"))
            .unwrap_err();
        set_config(None);
        assert_eq!(error, DnsError::Unsupported("DNS over TLS"));
        assert_eq!(error.code(), "ERR_DNS_UNSUPPORTED");
    }

    #[test]
    fn the_system_mode_leaves_resolution_to_the_os() {
        let _guard = serial();
        set_config(Some(DnsConfig::default()));
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(resolve_host("example.test"))
            .unwrap();
        set_config(None);
        assert!(result.is_empty(), "system mode must not resolve here");
    }
}
