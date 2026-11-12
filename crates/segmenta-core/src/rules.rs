//! Pure browser-capture rules (0.1.1): extensions, wildcards and denylist.

/// Extensions the extension captures by default (approved 0.1.2 list).
///
/// The daemon's `Settings::default` uses the same tokens and order; a daemon test
/// asserts the parity so the two cannot drift apart silently.
pub const CAPTURE_EXTENSIONS: &[&str] = &[
    "3gp", "7z", "aac", "ace", "aif", "apk", "appimage", "arj", "asf", "avi", "bin", "bz2", "deb",
    "epub", "exe", "gz", "gzip", "img", "iso", "jar", "lzh", "m4a", "m4v", "mkv", "mov", "mp3",
    "mp4", "mpa", "mpe", "mpeg", "mpg", "msi", "msix", "msu", "ogg", "ogv", "pdf", "plj", "pps",
    "ppt", "qt", "r0*", "r1*", "ra", "rar", "rm", "rmvb", "rpm", "sea", "sit", "sitx", "tar",
    "tif", "tiff", "vsix", "wav", "whl", "wma", "wmv", "xpi", "xz", "z", "zip", "pkg",
];
/// Normalize one token: lower case, no leading dot, wildcard only at the end.
pub fn normalize_extension(raw: &str) -> Option<String> {
    let token = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    if token.is_empty() || token.len() > 16 {
        return None;
    }
    let body = token.trim_end_matches('*');
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    if token.contains('*') && !token.ends_with('*') || token.trim_end_matches('*').contains('*') {
        return None;
    }
    Some(token)
}

/// Normalize a list, dropping invalid tokens and preserving the first occurrence.
pub fn normalize_extensions<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for token in tokens {
        if let Some(normalized) = normalize_extension(token) {
            if !seen.contains(&normalized) {
                seen.push(normalized);
            }
        }
    }
    seen
}

/// Whether a file extension matches the configured list (`r0*` covers `r00`…`r09`).
pub fn extension_matches(list: &[String], extension: &str) -> bool {
    let Some(extension) = normalize_extension(extension) else {
        return false;
    };
    list.iter().any(|token| match token.strip_suffix('*') {
        Some(prefix) => extension.starts_with(prefix),
        None => token == &extension,
    })
}

/// Extract the lowercase extension from a file name or URL path.
pub fn name_extension(name: &str) -> Option<String> {
    let without_query = name.split(['?', '#']).next().unwrap_or(name);
    let file = without_query
        .rsplit('/')
        .next()
        .unwrap_or(without_query)
        .to_string();
    let (stem, extension) = file.rsplit_once('.')?;
    if stem.is_empty() || extension.is_empty() {
        return None;
    }
    Some(extension.to_ascii_lowercase())
}

/// Whether a host is covered by the denylist (`example.com` covers `www.example.com`).
pub fn host_denied(denylist: &[String], host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    denylist.iter().any(|entry| {
        let entry = entry.trim_start_matches('.').to_ascii_lowercase();
        !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
    })
}

/// Full capture decision for a browser download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDecision {
    /// The download must be delegated to the daemon.
    pub capture: bool,
    /// Why the rule fired, for logs and tests.
    pub reason: &'static str,
}

/// Decide whether a browser download belongs to Segmenta.
pub fn decide(
    enabled: bool,
    file_name: &str,
    host: &str,
    extensions: &[String],
    denylist: &[String],
) -> CaptureDecision {
    if !enabled {
        return CaptureDecision {
            capture: false,
            reason: "disabled",
        };
    }
    if !denylist.is_empty() && host_denied(denylist, host) {
        return CaptureDecision {
            capture: false,
            reason: "denylisted",
        };
    }
    match name_extension(file_name) {
        Some(extension) if extension_matches(extensions, &extension) => CaptureDecision {
            capture: true,
            reason: "extension",
        },
        Some(_) => CaptureDecision {
            capture: false,
            reason: "extension-not-listed",
        },
        None => CaptureDecision {
            capture: false,
            reason: "no-extension",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tokens() {
        assert_eq!(normalize_extension(" .MP4 "), Some("mp4".to_string()));
        assert_eq!(normalize_extension("r0*"), Some("r0*".to_string()));
        assert_eq!(normalize_extension("*mp4"), None);
        assert_eq!(normalize_extension("m*4"), None);
        assert_eq!(normalize_extension(""), None);
    }

    #[test]
    fn wildcards_match_by_prefix() {
        let list = normalize_extensions(["mp4", "r0*", "r1*", "MP4"]);
        assert_eq!(list.len(), 3);
        assert!(extension_matches(&list, "r00"));
        assert!(extension_matches(&list, "R15"));
        assert!(!extension_matches(&list, "r20"));
        assert!(extension_matches(&list, "mp4"));
    }

    #[test]
    fn denylist_matches_suffixes_only() {
        let list = normalize_extensions(["https://example.com/v.mp4"]);
        assert!(list.is_empty());
        let denied = vec!["example.com".to_string()];
        assert!(host_denied(&denied, "www.example.com"));
        assert!(host_denied(&denied, "example.com"));
        assert!(!host_denied(&denied, "notexample.com"));
    }

    #[test]
    fn decisions_cover_disabled_denylisted_and_listed() {
        let extensions = normalize_extensions(CAPTURE_EXTENSIONS.iter().copied());
        let denylist = vec!["evil.test".to_string()];
        assert_eq!(
            decide(false, "a.mp4", "host", &extensions, &denylist).reason,
            "disabled"
        );
        assert_eq!(
            decide(true, "a.mp4", "evil.test", &extensions, &denylist).reason,
            "denylisted"
        );
        assert!(decide(true, "a.MP4?x=1", "host", &extensions, &denylist).capture);
        assert!(decide(true, "disk.r05", "host", &extensions, &denylist).capture);
        assert_eq!(
            decide(true, "a.txt", "host", &extensions, &denylist).reason,
            "extension-not-listed"
        );
        assert_eq!(
            decide(true, "README", "host", &extensions, &denylist).reason,
            "no-extension"
        );
    }
}
