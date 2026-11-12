//! Supported-browser detection and native-messaging manifest status (0.1.1, S8).

use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
    /// Prepared extension path if present.
    pub prepared_path: Option<PathBuf>,
    /// Shipped archive path if present.
    pub archive_path: Option<PathBuf>,
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
                prepared_path: get_prepared_path(spec.family),
                archive_path: shipped_archive_path(spec.family),
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

/// Directory where a browser keeps unpacked/installed extensions.
fn profile_roots(browser_id: &str, home: &Path, config_home: &Path) -> Vec<PathBuf> {
    match browser_id {
        "chrome" => vec![config_home.join("google-chrome")],
        "chromium" => vec![config_home.join("chromium")],
        "edge" => vec![config_home.join("microsoft-edge")],
        "brave" => vec![config_home.join("BraveSoftware/Brave-Browser")],
        "vivaldi" => vec![config_home.join("vivaldi")],
        "opera" => vec![config_home.join("opera")],
        "firefox" => vec![home.join(".mozilla/firefox")],
        _ => Vec::new(),
    }
}

/// Whether the browser profile shows the extension, read-only.
///
/// Chromium keeps an unpacked extension under `Default/Extensions/<id>`; Firefox
/// records it under `<profile>/extensions/`. Nothing here writes to a profile.
pub fn extension_installed(browser_id: &str, home: &Path, config_home: &Path) -> bool {
    if browser_id == "firefox" {
        for root in profile_roots(browser_id, home, config_home) {
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in entries.flatten() {
                let extensions = entry.path().join("extensions");
                let Ok(files) = std::fs::read_dir(&extensions) else {
                    continue;
                };
                if files.flatten().any(|file| {
                    file.file_name()
                        .to_string_lossy()
                        .starts_with(FIREFOX_EXTENSION_ID)
                }) {
                    return true;
                }
            }
        }
        return false;
    }
    for root in profile_roots(browser_id, home, config_home) {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry
                .path()
                .join("Extensions")
                .join(CHROMIUM_EXTENSION_ID)
                .is_dir()
            {
                return true;
            }
        }
    }
    false
}

/// Per-user manifest path for a browser, following XDG.
pub fn user_manifest_path(browser_id: &str, home: &Path, config_home: &Path) -> Option<PathBuf> {
    let relative = user_manifest_relative(browser_id)?;
    let relative = relative.strip_prefix(".config/").unwrap_or(relative);
    let base = if relative.starts_with(".mozilla") {
        home.to_path_buf()
    } else {
        config_home.to_path_buf()
    };
    Some(base.join(relative).join("io.github.wilkin.segmenta.json"))
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
    if HOST_PATH.is_empty() {
        return Err("the native host path is not configured".to_string());
    }
    // XDG_CONFIG_HOME wins over ~/.config for every browser that follows the spec.
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let relative = relative.strip_prefix(".config/").unwrap_or(relative);
    let directory = if relative.starts_with(".mozilla") {
        home.join(relative)
    } else {
        config_home.join(relative)
    };
    std::fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    let path = directory.join("io.github.wilkin.segmenta.json");
    if path.exists() {
        return Ok(path);
    }
    std::fs::write(&path, manifest_json(browser.family)).map_err(|err| err.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .map_err(|err| err.to_string())?;
    Ok(path)
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let input = input.trim();
    let mut output = Vec::new();
    let mut val: u32 = 0;
    let mut valb: i32 = -8;
    for c in input.chars() {
        let dec = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            '=' => break,
            _ => continue,
        };
        val = (val << 6) | dec;
        valb += 6;
        if valb >= 0 {
            output.push((val >> valb) as u8);
            val &= (1 << valb) - 1;
            valb -= 8;
        }
    }
    Ok(output)
}

fn compute_chromium_extension_id(key_base64: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let der_bytes = base64_decode(key_base64)?;
    let hash = Sha256::digest(&der_bytes);
    let mut id = String::new();
    for &b in hash.iter().take(16) {
        let high = (b >> 4) & 0x0f;
        let low = b & 0x0f;
        id.push((b'a' + high) as char);
        id.push((b'a' + low) as char);
    }
    Ok(id)
}

/// Find the shipped extension archive path for a family.
pub fn shipped_archive_path(family: &str) -> Option<PathBuf> {
    let filename = if family == "firefox" {
        "segmenta-firefox.zip"
    } else {
        "segmenta-chromium.zip"
    };

    let env_key = if family == "firefox" {
        "SEGMENTA_FIREFOX_ZIP"
    } else {
        "SEGMENTA_CHROMIUM_ZIP"
    };
    if let Ok(path) = std::env::var(env_key) {
        if !path.is_empty() {
            let p = PathBuf::from(path);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    let system_path = PathBuf::from("/usr/share/segmenta/extensions").join(filename);
    if system_path.is_file() {
        return Some(system_path);
    }

    if let Ok(cargo_manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(&cargo_manifest).join(format!("../../extension/{filename}"));
        if p.is_file() {
            return Some(p);
        }
        let p2 = PathBuf::from(&cargo_manifest).join(format!("../../dist/{filename}"));
        if p2.is_file() {
            return Some(p2);
        }
        let p3 = PathBuf::from(&cargo_manifest).join(format!("../extension/{filename}"));
        if p3.is_file() {
            return Some(p3);
        }
    }

    let local = PathBuf::from("extension").join(filename);
    if local.is_file() {
        return Some(local);
    }
    let dist = PathBuf::from("dist").join(filename);
    if dist.is_file() {
        return Some(dist);
    }

    Some(system_path)
}

/// Get the prepared extension path if it exists and is valid.
pub fn get_prepared_path(family: &str) -> Option<PathBuf> {
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".local/share")
        });
    let version = env!("CARGO_PKG_VERSION");
    let path = data_home
        .join("segmenta/extensions")
        .join(format!("{family}-{version}"));
    if path.join("manifest.json").is_file() && path.join("_locales").is_dir() {
        Some(path)
    } else {
        None
    }
}

/// Unzip and prepare browser extension(s) into $XDG_DATA_HOME/segmenta/extensions/<family>-<version>.
pub fn prepare_extension(family: &str) -> Result<Vec<(String, PathBuf)>, String> {
    let families = match family {
        "chromium" => vec!["chromium"],
        "firefox" => vec!["firefox"],
        "all" | "" => vec!["chromium", "firefox"],
        _ => return Err(format!("unknown family `{family}`")),
    };
    let mut results = Vec::new();
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".local/share")
        });
    let extensions_base = data_home.join("segmenta/extensions");
    std::fs::create_dir_all(&extensions_base).map_err(|e| e.to_string())?;
    let version = env!("CARGO_PKG_VERSION");

    for fam in families {
        let archive_path = shipped_archive_path(fam)
            .ok_or_else(|| format!("extension archive for `{fam}` not found"))?;
        let target_dir = extensions_base.join(format!("{fam}-{version}"));
        let rand_suffix: u64 = rand::random();
        let temp_dir = extensions_base.join(format!("{fam}-{version}.tmp.{rand_suffix}"));
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }

        let file = std::fs::File::open(&archive_path)
            .map_err(|e| format!("cannot open archive {}: {e}", archive_path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("corrupt zip archive {}: {e}", archive_path.display()))?;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| format!("zip index error: {e}"))?;
            let outpath = match file.enclosed_name() {
                Some(path) => temp_dir.join(path),
                None => continue,
            };
            if file.name().ends_with('/') {
                std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
                    }
                }
                let mut outfile = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
                std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
            }
        }

        let manifest_path = temp_dir.join("manifest.json");
        let locales_path = temp_dir.join("_locales");
        if !manifest_path.is_file() || !locales_path.is_dir() {
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Err(format!(
                "invalid extension archive for `{fam}`: missing manifest.json or _locales"
            ));
        }

        let manifest_content = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("cannot read manifest.json: {e}"))?;
        let manifest_json: serde_json::Value = serde_json::from_str(&manifest_content)
            .map_err(|e| format!("invalid manifest.json JSON: {e}"))?;

        if fam == "firefox" {
            let gecko_id = manifest_json
                .get("browser_specific_settings")
                .and_then(|v| v.get("gecko"))
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if gecko_id != FIREFOX_EXTENSION_ID {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(format!("firefox extension id mismatch: expected `{FIREFOX_EXTENSION_ID}`, got `{gecko_id}`"));
            }
        } else {
            let key_str = manifest_json
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "chromium manifest.json missing `key`".to_string())?;
            let computed_id = compute_chromium_extension_id(key_str)?;
            if computed_id != CHROMIUM_EXTENSION_ID {
                let _ = std::fs::remove_dir_all(&temp_dir);
                return Err(format!("chromium extension id mismatch: expected `{CHROMIUM_EXTENSION_ID}`, got `{computed_id}`"));
            }
        }

        if target_dir.exists() {
            let _ = std::fs::remove_dir_all(&target_dir);
        }
        std::fs::rename(&temp_dir, &target_dir)
            .map_err(|e| format!("cannot rename extracted extension dir: {e}"))?;

        results.push((fam.to_string(), target_dir));
    }

    Ok(results)
}

/// Open browser extensions page with fixed argv array.
pub fn open_browser(browser_id: &str) -> Result<(), String> {
    let browsers = detect();
    let browser = browsers
        .iter()
        .find(|b| b.id == browser_id)
        .ok_or_else(|| format!("unknown browser `{browser_id}`"))?;
    if let Some(reason) = browser.unsupported {
        return Err(format!(
            "browser `{}` is a {reason} package and cannot run",
            browser.name
        ));
    }
    let binary = browser
        .command
        .as_ref()
        .ok_or_else(|| format!("browser `{}` is not installed", browser.name))?;
    let url = match browser.id {
        "firefox" => "about:debugging#/runtime/this-firefox",
        "chrome" | "chromium" | "edge" | "brave" | "vivaldi" | "opera" => "chrome://extensions",
        _ => "chrome://extensions",
    };
    std::process::Command::new(binary)
        .arg(url)
        .spawn()
        .map_err(|e| format!("cannot launch `{binary}`: {e}"))?;
    Ok(())
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

    #[test]
    fn installed_extensions_are_detected_from_profiles_without_writing() {
        let dir = fake_dir();
        let home = dir.path().join("home");
        let config_home = home.join(".config");
        assert!(!extension_installed("chrome", &home, &config_home));
        let profile = config_home
            .join("google-chrome/Default/Extensions")
            .join(CHROMIUM_EXTENSION_ID);
        std::fs::create_dir_all(&profile).unwrap();
        assert!(extension_installed("chrome", &home, &config_home));
        assert!(!extension_installed("chromium", &home, &config_home));
        let firefox_profile = home.join(".mozilla/firefox/abc.default/extensions");
        std::fs::create_dir_all(&firefox_profile).unwrap();
        std::fs::write(
            firefox_profile.join(format!("{FIREFOX_EXTENSION_ID}.xpi")),
            b"xpi",
        )
        .unwrap();
        assert!(extension_installed("firefox", &home, &config_home));
        // XDG wins for the per-user manifest path.
        let path = user_manifest_path("chrome", &home, &config_home).unwrap();
        assert!(path.starts_with(&config_home));
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

    #[test]
    fn test_compute_chromium_extension_id() {
        let key = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAld43LYBouFzGPA0BOUOLBF3Wv/DMAh431KHvYebpNr4OMKKS2IHfhq7o8qNwfBQsVNi2KD8zXvtT+Skf9b2Cs1VGa3Hm9fgRV26u0yISA0nGEh7pEZr8axGS7A2loRtr8uGNGXTERGYrxlCgXAjkU1JFMO9Un7X0n8dtBWVxGfxCNMIPzAUaJpOw+SsphKUv3l01vdK1HPCoPnkkGWi/kkgVA2ceBBg8c6Srq6I/5AKzTOOunuHsFjqsQhZSBCDGwhEKTceRC5DCWEDFyj0gXp/ArjNgpxct9/RX7pa4aMfwKDeYfSp1YKLWH5WeeDQlpLl82gQkPxej07LuvpIidwIDAQAB";
        let id = compute_chromium_extension_id(key).unwrap();
        assert_eq!(id, CHROMIUM_EXTENSION_ID);
    }

    fn create_test_zip(path: &Path, manifest_json: &str, include_locales: bool) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::FileOptions::default();
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(manifest_json.as_bytes()).unwrap();
        if include_locales {
            zip.add_directory("_locales/en", options).unwrap();
            zip.start_file("_locales/en/messages.json", options)
                .unwrap();
            zip.write_all(b"{}").unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn extension_prepare_tests() {
        let dir = fake_dir();
        let zip_path = dir.path().join("segmenta-chromium.zip");
        let valid_manifest =
            "{\"key\": \"MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAld43LYBouFzGPA0BOUOLBF3Wv/DMAh431KHvYebpNr4OMKKS2IHfhq7o8qNwfBQsVNi2KD8zXvtT+Skf9b2Cs1VGa3Hm9fgRV26u0yISA0nGEh7pEZr8axGS7A2loRtr8uGNGXTERGYrxlCgXAjkU1JFMO9Un7X0n8dtBWVxGfxCNMIPzAUaJpOw+SsphKUv3l01vdK1HPCoPnkkGWi/kkgVA2ceBBg8c6Srq6I/5AKzTOOunuHsFjqsQhZSBCDGwhEKTceRC5DCWEDFyj0gXp/ArjNgpxct9/RX7pa4aMfwKDeYfSp1YKLWH5WeeDQlpLl82gQkPxej07LuvpIidwIDAQAB\"}"
                .to_string();
        create_test_zip(&zip_path, &valid_manifest, true);

        std::env::set_var("SEGMENTA_CHROMIUM_ZIP", &zip_path);
        let home = dir.path().join("home");
        std::env::set_var("HOME", &home);

        let res = prepare_extension("chromium");
        assert!(res.is_ok());

        let invalid_zip = dir.path().join("invalid.zip");
        create_test_zip(&invalid_zip, &valid_manifest, false);
        std::env::set_var("SEGMENTA_CHROMIUM_ZIP", &invalid_zip);
        let res2 = prepare_extension("chromium");
        assert!(res2.is_err());

        let wrong_zip = dir.path().join("wrong.zip");
        create_test_zip(&wrong_zip, "{\"key\": \"INVALIDKEY\"}", true);
        std::env::set_var("SEGMENTA_CHROMIUM_ZIP", &wrong_zip);
        let res3 = prepare_extension("chromium");
        assert!(res3.is_err());
    }
}
