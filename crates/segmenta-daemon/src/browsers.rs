//! Supported-browser detection and native-messaging manifest status (0.1.1, S8).

use std::path::{Path, PathBuf};

/// One browser the installer knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    /// Stable identifier used by the UI.
    pub id: &'static str,
    /// Human name.
    pub name: &'static str,
    /// `chromium`, `firefox` or `other`.
    pub family: &'static str,
    /// Executable found on `PATH`.
    pub command: Option<String>,
    /// First line of `--version`, when it ran.
    pub version: Option<String>,
    /// Reason the browser cannot use the host, when that is known.
    pub unsupported: Option<&'static str>,
    /// System manifest path for this browser.
    pub manifest_path: PathBuf,
    /// Whether the system manifest exists.
    pub manifest_present: bool,
}

struct Spec {
    id: &'static str,
    name: &'static str,
    family: &'static str,
    commands: &'static [&'static str],
    manifest: &'static str,
}

const SPECS: &[Spec] = &[
    Spec {
        id: "chrome",
        name: "Google Chrome",
        family: "chromium",
        commands: &["google-chrome", "google-chrome-stable"],
        manifest: "/etc/opt/chrome/native-messaging-hosts/io.github.wilkin.segmenta.json",
    },
    Spec {
        id: "chromium",
        name: "Chromium",
        family: "chromium",
        commands: &["chromium", "chromium-browser"],
        manifest: "/etc/chromium/native-messaging-hosts/io.github.wilkin.segmenta.json",
    },
    Spec {
        id: "edge",
        name: "Microsoft Edge",
        family: "chromium",
        commands: &["microsoft-edge", "microsoft-edge-stable"],
        manifest: "/etc/opt/edge/native-messaging-hosts/io.github.wilkin.segmenta.json",
    },
    Spec {
        id: "brave",
        name: "Brave",
        family: "chromium",
        commands: &["brave-browser", "brave"],
        manifest: "/etc/brave/native-messaging-hosts/io.github.wilkin.segmenta.json",
    },
    Spec {
        id: "vivaldi",
        name: "Vivaldi",
        family: "chromium",
        commands: &["vivaldi", "vivaldi-stable"],
        manifest: "/etc/vivaldi/native-messaging-hosts/io.github.wilkin.segmenta.json",
    },
    Spec {
        id: "opera",
        name: "Opera",
        family: "chromium",
        commands: &["opera"],
        manifest: "/etc/opera/native-messaging-hosts/io.github.wilkin.segmenta.json",
    },
    Spec {
        id: "firefox",
        name: "Firefox",
        family: "firefox",
        commands: &["firefox"],
        manifest: "/usr/lib/mozilla/native-messaging-hosts/io.github.wilkin.segmenta.json",
    },
];

/// Find an executable on an explicit `PATH`.
pub fn which(command: &str, path_env: &str) -> Option<PathBuf> {
    path_env
        .split(':')
        .filter(|entry| !entry.is_empty())
        .map(|entry| Path::new(entry).join(command))
        .find(|candidate| candidate.is_file())
}

fn version_of(executable: &Path) -> Option<String> {
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text.lines().next().unwrap_or_default().to_string())
    }
}

/// Detect every supported browser against an explicit `PATH` and manifest root.
pub fn detect_with(path_env: &str, manifest_root: &Path, probe_version: bool) -> Vec<Browser> {
    SPECS
        .iter()
        .map(|spec| {
            let executable = spec
                .commands
                .iter()
                .find_map(|command| which(command, path_env));
            let manifest_path = if manifest_root == Path::new("") {
                PathBuf::from(spec.manifest)
            } else {
                manifest_root.join(Path::new(spec.manifest).file_name().unwrap_or_default())
            };
            // Snap and Flatpak wrappers cannot launch the native host from outside
            // their sandbox; report that instead of offering a broken install.
            let unsupported = executable.as_ref().and_then(|path| {
                let path = path.to_string_lossy();
                if path.contains("/snap/") || path.contains("/snap/bin/") {
                    Some("snap")
                } else if path.contains("/flatpak/") || path.contains("/var/lib/flatpak/") {
                    Some("flatpak")
                } else {
                    None
                }
            });
            Browser {
                id: spec.id,
                name: spec.name,
                family: spec.family,
                command: executable
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                version: if probe_version {
                    executable.as_deref().and_then(version_of)
                } else {
                    None
                },
                unsupported,
                manifest_present: manifest_path.is_file(),
                manifest_path,
            }
        })
        .collect()
}

/// Detect against the running system.
pub fn detect() -> Vec<Browser> {
    let path_env = std::env::var("PATH").unwrap_or_default();
    detect_with(&path_env, Path::new(""), true)
}

/// Browsers whose command exists and that can use the native host.
pub fn supported(browsers: &[Browser]) -> Vec<&Browser> {
    browsers
        .iter()
        .filter(|browser| browser.command.is_some() && browser.unsupported.is_none())
        .collect()
}

/// Chromium extension id shipped by the `.deb` (also in `extension/CHROMIUM-ID.txt`).
pub const CHROMIUM_EXTENSION_ID: &str = "kdeldnifdbpemnhfpmhonlnibbhnoaan";
/// Firefox add-on id.
pub const FIREFOX_EXTENSION_ID: &str = "segmenta@wilkin.github.io";
/// Native host executable installed by the package.
pub const HOST_PATH: &str = "/usr/bin/segmenta-host";

fn user_manifest_relative(id: &str) -> Option<&'static str> {
    Some(match id {
        "chrome" => ".config/google-chrome/NativeMessagingHosts",
        "chromium" => ".config/chromium/NativeMessagingHosts",
        "edge" => ".config/microsoft-edge/NativeMessagingHosts",
        "brave" => ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
        "vivaldi" => ".config/vivaldi/NativeMessagingHosts",
        "opera" => ".config/opera/NativeMessagingHosts",
        "firefox" => ".mozilla/native-messaging-hosts",
        _ => return None,
    })
}

/// Manifest body for one browser family.
pub fn manifest_json(family: &str) -> String {
    if family == "firefox" {
        format!(
            "{{\n  \"name\": \"io.github.wilkin.segmenta\",\n  \"description\": \"Segmenta native host\",\n  \"path\": \"{HOST_PATH}\",\n  \"type\": \"stdio\",\n  \"allowed_extensions\": [\"{FIREFOX_EXTENSION_ID}\"]\n}}\n"
        )
    } else {
        format!(
            "{{\n  \"name\": \"io.github.wilkin.segmenta\",\n  \"description\": \"Segmenta native host\",\n  \"path\": \"{HOST_PATH}\",\n  \"type\": \"stdio\",\n  \"allowed_origins\": [\"chrome-extension://{CHROMIUM_EXTENSION_ID}/\"]\n}}\n"
        )
    }
}

/// Write the per-user manifest for a detected browser; never overwrites a file.
pub fn install_user_manifest(browser: &Browser, home: &Path) -> Result<PathBuf, String> {
    if browser.command.is_none() {
        return Err(format!("{} is not installed", browser.name));
    }
    if let Some(reason) = browser.unsupported {
        return Err(format!(
            "{} is a {reason} package and cannot run the host",
            browser.name
        ));
    }
    let relative = user_manifest_relative(browser.id)
        .ok_or_else(|| format!("{} has no manifest location", browser.name))?;
    let directory = home.join(relative);
    std::fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    let path = directory.join("io.github.wilkin.segmenta.json");
    if path.exists() {
        return Ok(path);
    }
    std::fs::write(&path, manifest_json(browser.family)).map_err(|err| err.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn detection_reports_found_missing_and_snap_wrappers() {
        let dir = fake_dir();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["google-chrome", "firefox"] {
            let path = bin.join(name);
            std::fs::write(&path, "#!/bin/sh\necho 'Browser 123.0'\n").unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
        }
        let snap = dir.path().join("snap/bin");
        std::fs::create_dir_all(&snap).unwrap();
        let chromium = snap.join("chromium");
        std::fs::write(&chromium, "").unwrap();
        std::os::unix::fs::PermissionsExt::set_mode(
            &mut std::fs::metadata(&chromium).unwrap().permissions(),
            0o755,
        );
        std::fs::set_permissions(
            &chromium,
            std::fs::metadata(&chromium).unwrap().permissions(),
        )
        .unwrap();

        let path_env = format!("{}:{}", bin.display(), snap.display());
        let browsers = detect_with(&path_env, &dir.path().join("manifests"), false);
        let chrome = browsers.iter().find(|b| b.id == "chrome").unwrap();
        assert!(chrome.command.is_some());
        assert!(chrome.unsupported.is_none());
        assert!(!chrome.manifest_present);
        let chromium = browsers.iter().find(|b| b.id == "chromium").unwrap();
        assert_eq!(chromium.unsupported, Some("snap"));
        let edge = browsers.iter().find(|b| b.id == "edge").unwrap();
        assert!(edge.command.is_none());
        assert_eq!(supported(&browsers).len(), 2);

        // Per-user manifests are written once and never overwrite an existing file.
        let home = dir.path().join("home");
        let chrome = browsers.iter().find(|b| b.id == "chrome").unwrap();
        let written = install_user_manifest(chrome, &home).unwrap();
        assert!(written.is_file());
        let body = std::fs::read_to_string(&written).unwrap();
        assert!(body.contains(CHROMIUM_EXTENSION_ID));
        assert!(body.contains(HOST_PATH));
        std::fs::write(&written, "custom").unwrap();
        install_user_manifest(chrome, &home).unwrap();
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "custom");
        let chromium = browsers.iter().find(|b| b.id == "chromium").unwrap();
        assert!(install_user_manifest(chromium, &home).is_err());
        let firefox = browsers.iter().find(|b| b.id == "firefox").unwrap();
        let firefox_manifest = install_user_manifest(firefox, &home).unwrap();
        assert!(std::fs::read_to_string(firefox_manifest)
            .unwrap()
            .contains(FIREFOX_EXTENSION_ID));
    }

    /// The constant and the shipped manifest must never drift again (0.1.2 P0).
    #[test]
    fn firefox_extension_id_matches_the_shipped_manifest() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extension/firefox/manifest.json");
        let body = std::fs::read_to_string(&manifest).expect("firefox manifest");
        assert!(
            body.contains(FIREFOX_EXTENSION_ID),
            "manifest does not declare {FIREFOX_EXTENSION_ID}"
        );
    }
}
