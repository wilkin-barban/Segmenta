//! VPN interface guard: bind downloads to one interface and refuse to fall back.
//!
//! Contract: T2 of `odd/tasks/segmenta-v0.6.0-red-remoto.md`.
//!
//! # Why
//!
//! Binding a socket to a device is what keeps a download inside a VPN tunnel: if the
//! tunnel is down, the socket fails instead of leaving through the default route, which is
//! the leak a kill switch exists to prevent. That property only holds while the interface
//! is up, so the daemon checks it before it dials and never "tries anyway".
//!
//! # Linux
//!
//! The interface name travels to `reqwest`'s `interface()` option, which sets
//! `SO_BINDTODEVICE` on the socket; this module never opens a socket itself. The checks
//! read `/sys/class/net`, which is the kernel's own view of the device.

use std::path::PathBuf;

/// Longest interface name the kernel accepts (`IFNAMSIZ - 1`).
pub const MAX_INTERFACE_LEN: usize = 15;

/// Where the kernel exposes its network interfaces.
const SYS_CLASS_NET: &str = "/sys/class/net";

/// Why an interface cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceProblem {
    /// The name is empty or has characters the kernel would reject.
    InvalidName,
    /// The interface is not present right now.
    Missing,
    /// The interface exists but is not up: traffic would leave another way.
    Down,
}

impl InterfaceProblem {
    /// Stable code the daemon reports to the client.
    pub fn code(self) -> &'static str {
        match self {
            InterfaceProblem::InvalidName => "ERR_VPN_INTERFACE_INVALID",
            InterfaceProblem::Missing => "ERR_VPN_INTERFACE_MISSING",
            InterfaceProblem::Down => "ERR_VPN_INTERFACE_DOWN",
        }
    }

    /// Sentence for the task record; never includes anything from the environment.
    pub fn message(self, name: &str) -> String {
        match self {
            InterfaceProblem::InvalidName => {
                format!("`{name}` is not a valid network interface name")
            }
            InterfaceProblem::Missing => format!("network interface `{name}` does not exist"),
            InterfaceProblem::Down => {
                format!("network interface `{name}` is not up; the download is blocked")
            }
        }
    }
}

/// Check that `name` is a syntactically valid interface name.
///
/// Mirrors what the kernel accepts: 1..=15 bytes, no slash, no whitespace, and no leading
/// dot, so a name can never walk out of `/sys/class/net`.
pub fn validate_name(name: &str) -> Result<(), InterfaceProblem> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= MAX_INTERFACE_LEN
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(InterfaceProblem::InvalidName)
    }
}

/// Whether the kernel lists `name` as an interface.
pub fn exists(name: &str) -> bool {
    validate_name(name).is_ok() && path_of(name).exists()
}

/// Whether the interface is up, refused when it is down or gone.
pub fn ensure_up(name: &str) -> Result<(), InterfaceProblem> {
    validate_name(name)?;
    let base = path_of(name);
    if !base.exists() {
        return Err(InterfaceProblem::Missing);
    }
    // `operstate` is `up`, `down`, `unknown` or `dormant`. `unknown` is common for
    // tunnels (wireguard reports it), so it counts as usable; `down` does not.
    let state = std::fs::read_to_string(base.join("operstate"))
        .map(|state| state.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    match state.as_str() {
        "down" | "lowerlayerdown" | "notpresent" => Err(InterfaceProblem::Down),
        _ => Ok(()),
    }
}

fn path_of(name: &str) -> PathBuf {
    PathBuf::from(SYS_CLASS_NET).join(name.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_kernel_style_names() {
        assert!(validate_name("wg0").is_ok());
        assert!(validate_name("tun0").is_ok());
        assert!(validate_name("enp0s3.100").is_ok());
        assert!(validate_name("br-1a2b3c").is_ok());

        assert_eq!(validate_name(""), Err(InterfaceProblem::InvalidName));
        assert_eq!(validate_name("   "), Err(InterfaceProblem::InvalidName));
        assert_eq!(
            validate_name("../../etc"),
            Err(InterfaceProblem::InvalidName)
        );
        assert_eq!(validate_name("wg 0"), Err(InterfaceProblem::InvalidName));
        assert_eq!(validate_name(".hidden"), Err(InterfaceProblem::InvalidName));
        assert_eq!(
            validate_name("a-very-long-interface-name"),
            Err(InterfaceProblem::InvalidName)
        );
    }

    #[test]
    fn reads_the_kernel_view_of_the_interfaces() {
        // `lo` exists on every Linux host that runs these tests.
        assert!(exists("lo"));
        assert_eq!(ensure_up("lo"), Ok(()));

        assert!(!exists("vpnx0"));
        assert_eq!(ensure_up("vpnx0"), Err(InterfaceProblem::Missing));
        // A syntactically broken name never reaches the filesystem.
        assert_eq!(ensure_up("../etc"), Err(InterfaceProblem::InvalidName));
        assert!(!exists("../etc"));
    }

    #[test]
    fn every_problem_has_a_stable_code_and_a_plain_message() {
        assert_eq!(
            InterfaceProblem::Missing.code(),
            "ERR_VPN_INTERFACE_MISSING"
        );
        assert_eq!(InterfaceProblem::Down.code(), "ERR_VPN_INTERFACE_DOWN");
        assert!(InterfaceProblem::Down.message("wg0").contains("is not up"));
    }
}
