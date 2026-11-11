//! Filename extraction and sanitization (OpenSpec 06 §1.3).
//!
//! Everything that reaches the filesystem goes through [`sanitize_filename`]: header
//! names are hostile input and URL paths are only a hint.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use url::Url;

const MAX_NAME_BYTES: usize = 255;
/// Longer extensions are not preserved when truncating a name.
const MAX_PRESERVED_EXT: usize = 32;
/// Guard against a directory filled with same-named files.
const MAX_COLLISION_ATTEMPTS: u32 = 10_000;

/// Reduce a raw name to a safe single path component.
///
/// Returns `None` when nothing usable is left (empty, only dots), so the caller can
/// fall back to `download_{task_id}.bin`.
pub fn sanitize_filename(raw: &str) -> Option<String> {
    // `rsplit` on both separators drops every directory prefix, including `..\..\`.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let mut cleaned = String::with_capacity(base.len());
    for ch in base.chars() {
        match ch {
            '\0' | '/' | '\\' => cleaned.push('_'),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => cleaned.push(c),
        }
    }
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        return None;
    }
    Some(truncate_keeping_extension(cleaned))
}

/// Cut a name to 255 bytes without splitting a code point, keeping the extension when
/// it is short enough to be worth keeping.
fn truncate_keeping_extension(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_string();
    }
    let extension = name
        .rfind('.')
        .map(|dot| &name[dot + 1..])
        .filter(|ext| !ext.is_empty() && ext.len() <= MAX_PRESERVED_EXT);
    match extension {
        Some(ext) => {
            let budget = MAX_NAME_BYTES - ext.len() - 1;
            let stem = cut_utf8(name, budget);
            format!("{stem}.{ext}")
        }
        None => cut_utf8(name, MAX_NAME_BYTES).to_string(),
    }
}

/// Longest prefix of `s` that fits in `max_bytes` and ends on a character boundary.
fn cut_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Extract a name from `Content-Disposition`, preferring `filename*` (RFC 5987).
pub fn filename_from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    for part in split_parameters(value) {
        let Some((key, raw)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let raw = unquote(raw.trim());
        match key.as_str() {
            "filename*" => extended = Some(decode_ext_value(raw)),
            "filename" => plain = Some(raw.to_string()),
            _ => {}
        }
    }
    let chosen = extended.or(plain)?;
    sanitize_filename(&chosen)
}

/// Split on `;` while respecting quoted strings.
fn split_parameters(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut quoted = false;
    for (idx, ch) in value.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parts.push(&value[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

/// Strip one layer of surrounding double quotes.
fn unquote(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

/// Decode `charset'lang'percent-encoded` from an RFC 5987 `ext-value`.
fn decode_ext_value(raw: &str) -> String {
    let mut pieces = raw.splitn(3, '\'');
    let charset = pieces.next().unwrap_or("utf-8");
    let _lang = pieces.next().unwrap_or_default();
    let encoded = pieces.next().unwrap_or(raw);
    // RFC 5987 allows other charsets; every browser sends UTF-8 and a lossy decode of
    // anything else is still safer than trusting the raw bytes as a file name.
    let _ = charset;
    percent_decode_str(encoded).decode_utf8_lossy().into_owned()
}

/// Last path segment of a URL, percent-decoded and sanitized.
pub fn suggest_filename(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.next_back()?;
    if segment.is_empty() {
        return None;
    }
    let decoded = percent_decode_str(segment).decode_utf8_lossy();
    sanitize_filename(&decoded)
}

/// First free name in `dir`, using the `name (1).ext` policy of OpenSpec 08.
///
/// The caller still has to create the file with `create_new`; this is the suggestion,
/// not the reservation, so two concurrent tasks cannot believe they own the same path.
pub fn resolve_unique_path(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("download"));
    let extension = path.extension().map(|ext| ext.to_owned());
    for attempt in 1..MAX_COLLISION_ATTEMPTS {
        let mut file_name = stem.clone();
        file_name.push(format!(" ({attempt})"));
        if let Some(ext) = &extension {
            file_name.push(".");
            file_name.push(ext);
        }
        let candidate = dir.join(&file_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many files with the same name",
    ))
}
