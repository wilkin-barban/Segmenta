//! Public HLS/DASH ingestion and merge planning (spec S9).
//!
//! The module is deliberately pure: manifest parsing and segment expansion only read
//! strings and URLs, and the merge layer only produces text plans and argument
//! vectors. Downloading the fragments stays in [`crate::engine`], and `ffmpeg` is
//! always invoked with separated arguments, never through a shell.
//!
//! Scope for 0.1.0: public, unencrypted, static streams. DRM (`EXT-X-KEY` other than
//! `NONE`, `ContentProtection`), live playlists and live MPDs are rejected with a
//! stable [`StreamError`].

pub mod dash;
pub mod hls;
pub mod merge;

use url::Url;

/// Everything that can stop a public HLS/DASH download before the first byte.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// The manifest carries `EXT-X-KEY` (not `NONE`) or `ContentProtection`.
    #[error("stream is DRM protected")]
    DrmProtected,
    /// The manifest is live (`type="dynamic"` or no `EXT-X-ENDLIST`).
    #[error("live streams are not supported")]
    LiveNotSupported,
    /// The manifest is not valid XML/M3U8 or misses required fields.
    #[error("malformed manifest: {0}")]
    Malformed(String),
    /// A manifest feature outside the 0.1.0 scope.
    #[error("unsupported stream: {0}")]
    Unsupported(String),
}

impl StreamError {
    /// Numeric code stored in `downloads.error_code` and sent as `data.code`.
    pub fn code(&self) -> i64 {
        match self {
            StreamError::DrmProtected => 1040,
            StreamError::LiveNotSupported => 1041,
            StreamError::Malformed(_) => 1042,
            StreamError::Unsupported(_) => 1043,
        }
    }

    /// Stable identifier for user-facing messages and logs.
    pub fn err_name(&self) -> &'static str {
        match self {
            StreamError::DrmProtected => "ERR_DRM_PROTECTED",
            StreamError::LiveNotSupported => "ERR_LIVE_NOT_SUPPORTED",
            StreamError::Malformed(_) => "ERR_MANIFEST_MALFORMED",
            StreamError::Unsupported(_) => "ERR_STREAM_UNSUPPORTED",
        }
    }
}

/// Shorthand for a `Malformed` error.
fn malformed(message: impl Into<String>) -> StreamError {
    StreamError::Malformed(message.into())
}

/// Join a manifest URI against its base URL.
fn resolve(base: &Url, raw: &str) -> Result<Url, StreamError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(malformed("empty URI"));
    }
    base.join(raw)
        .map_err(|err| malformed(format!("invalid URI `{raw}`: {err}")))
}

/// Look up one attribute of an already split attribute list.
fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Split a `KEY=VALUE,KEY="quoted,value"` list (HLS) or an XML attribute list.
///
/// Both grammars share the `KEY=VALUE` shape: HLS separates pairs with commas,
/// XML with whitespace, and only the quaternary ambiguity of a comma inside a
/// quoted value matters for HLS.
fn parse_attributes(input: &str) -> Vec<(String, String)> {
    fn separator(byte: u8) -> bool {
        matches!(byte, b',' | b' ' | b'\t' | b'\n' | b'\r')
    }

    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        while cursor < bytes.len() && separator(bytes[cursor]) {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        let key_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != b'=' && !separator(bytes[cursor]) {
            cursor += 1;
        }
        let key = input[key_start..cursor].to_string();
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        let mut value = String::new();
        if cursor < bytes.len() && bytes[cursor] == b'=' {
            cursor += 1;
            while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
                cursor += 1;
            }
            if cursor < bytes.len() && (bytes[cursor] == b'"' || bytes[cursor] == b'\'') {
                let quote = bytes[cursor];
                cursor += 1;
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                value = unescape_entities(&input[start..cursor]);
                if cursor < bytes.len() {
                    cursor += 1;
                }
            } else {
                // Unquoted: HLS values stop at the comma, XML always quotes.
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor] != b',' {
                    cursor += 1;
                }
                value = input[start..cursor].trim().to_string();
            }
        }
        if !key.is_empty() {
            out.push((key, value));
        }
    }
    out
}

/// Decode the XML entities that can appear in manifest attributes and `BaseURL`.
fn unescape_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(position) = rest.find('&') {
        out.push_str(&rest[..position]);
        let after = &rest[position + 1..];
        let Some(end) = after.find(';') else {
            out.push_str(&rest[position..]);
            return out;
        };
        let decoded = match &after[..end] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            entity => entity
                .strip_prefix('#')
                .and_then(|digits| match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => digits.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_split_on_commas_outside_quotes() {
        let attrs = parse_attributes("BANDWIDTH=800000,CODECS=\"avc1.42e01e,mp4a.40.2\",NAME=es");
        assert_eq!(attr(&attrs, "BANDWIDTH"), Some("800000"));
        assert_eq!(attr(&attrs, "CODECS"), Some("avc1.42e01e,mp4a.40.2"));
        assert_eq!(attr(&attrs, "NAME"), Some("es"));
        assert_eq!(attr(&attrs, "MISSING"), None);
    }

    #[test]
    fn attributes_accept_xml_spacing_and_entities() {
        let attrs = parse_attributes(" id = 'a&amp;b' width=640");
        assert_eq!(attr(&attrs, "id"), Some("a&b"));
        assert_eq!(attr(&attrs, "width"), Some("640"));
    }

    #[test]
    fn entities_decode_the_five_predefined_names_and_numeric_forms() {
        assert_eq!(unescape_entities("a&amp;b"), "a&b");
        assert_eq!(unescape_entities("&#38;&#x26;"), "&&");
        assert_eq!(unescape_entities("plain"), "plain");
        assert_eq!(unescape_entities("a & b"), "a & b");
        assert_eq!(unescape_entities("&unknown;"), "&unknown;");
    }
}
