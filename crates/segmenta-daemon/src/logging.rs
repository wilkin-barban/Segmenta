//! Structured logging with strict redaction (OpenSpec S19/S20).
//!
//! The daemon writes to stderr, which systemd captures in
//! `journalctl --user -u segmenta`. Every lifecycle event carries structured fields
//! (`task_id`, `attempt_id`, `state`, `status`, `error_code`, `retry`, `duration_ms`)
//! and nothing sensitive ever reaches the output: URLs are reduced to
//! `scheme://host/path`, headers are filtered, and a last-line scrubber rewrites any
//! `key=value` whose key looks sensitive before the line is written.

use std::io::Write;
use std::sync::Once;
use std::time::Instant;

use tracing_subscriber::EnvFilter;

/// Fields whose value must never be printed.
const SENSITIVE: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "secret",
    "token",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "body",
];

/// Placeholder written instead of a sensitive value.
pub const REDACTED: &str = "[redacted]";

/// Install the global subscriber once. Safe to call from tests and from `main`.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let filter = filter_from(std::env::var("SEGMENTA_LOG").ok().as_deref())
            .or_else(|| filter_from(std::env::var("RUST_LOG").ok().as_deref()))
            .unwrap_or_else(default_filter);
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(|| ScrubWriter::new(std::io::stderr()))
            .with_target(true)
            .with_level(true)
            .with_ansi(false)
            .finish();
        if let Err(err) = tracing::subscriber::set_global_default(subscriber) {
            // Another subscriber (a test) won the race; that is not fatal.
            eprintln!("segmenta-daemon: logging already installed: {err}");
        }
    });
}

/// Filter built from `SEGMENTA_LOG`/`RUST_LOG`; invalid values fall back to `info`.
pub fn filter_from(value: Option<&str>) -> Option<EnvFilter> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    match EnvFilter::try_new(value) {
        Ok(filter) => Some(filter),
        Err(err) => {
            eprintln!("segmenta-daemon: invalid log filter `{value}` ({err}); using info");
            Some(default_filter())
        }
    }
}

/// Default level: `info`.
pub fn default_filter() -> EnvFilter {
    EnvFilter::new("info")
}

/// Whether `name` must be redacted.
pub fn is_sensitive(name: &str) -> bool {
    // Header names use dashes, setting keys use underscores: normalize both.
    let lowered = name.to_ascii_lowercase().replace('-', "_");
    // Exact names or a sensitive suffix; substring matching would hide audit fields
    // such as `secret_backend` or `checksum_ok`.
    lowered
        .split(['.', '/', ':'])
        .last()
        .map(|last| SENSITIVE.contains(&last) || has_sensitive_suffix(last))
        .unwrap_or(false)
}

fn has_sensitive_suffix(name: &str) -> bool {
    const SUFFIXES: &[&str] = &["_token", "_secret", "_password", "_passwd", "_api_key", "_cookie"];
    SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// Reduce a URL to `scheme://host/path`, dropping query, fragment and userinfo.
pub fn sanitize_url(raw: &str) -> String {
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (scheme, rest),
        None => ("", raw),
    };
    let (authority_and_path, _) = rest.split_once(['?', '#']).unwrap_or((rest, ""));
    let (authority, path) = match authority_and_path.find('/') {
        Some(index) => (&authority_and_path[..index], &authority_and_path[index..]),
        None => (authority_and_path, ""),
    };
    // Drop `user:pass@`.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let path = truncate(path, 120);
    if scheme.is_empty() {
        format!("{authority}{path}")
    } else {
        format!("{scheme}://{authority}{path}")
    }
}

/// Cut `text` to `max` bytes without breaking a UTF-8 character.
fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    // Keep the total length within `max` bytes, ellipsis included (3 bytes).
    let mut end = max.saturating_sub('…'.len_utf8());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Escape control characters and redact the value when the field name is sensitive.
pub fn redact_field(name: &str, value: &str) -> String {
    if is_sensitive(name) {
        return REDACTED.to_string();
    }
    value
        .chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect()
}

/// Redact a whole header list, keeping only safe names and values.
pub fn redact_headers<'a, I>(headers: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    headers
        .into_iter()
        .map(|(name, value)| (name.to_string(), redact_field(name, value)))
        .collect()
}

/// Last-line scrubber applied to the rendered log line.
///
/// It rewrites `key=value` (optionally quoted) for sensitive keys and also removes
/// query strings that a caller forgot to sanitize.
pub fn scrub_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(position) = rest.find(['?', '&']) {
        let (head, tail) = rest.split_at(position);
        out.push_str(head);
        // A query string runs to whitespace; drop it entirely.
        let end = tail
            .find(|ch: char| ch.is_whitespace() || ch == '"')
            .unwrap_or(tail.len());
        out.push_str("?[redacted]");
        rest = &tail[end..];
    }
    out.push_str(rest);
    scrub_assignments(&out)
}

fn scrub_assignments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut index = 0usize;
    while index < line.len() {
        let tail = &line[index..];
        let Some(eq) = tail.find('=') else {
            out.push_str(tail);
            break;
        };
        let (prefix, after_eq) = tail.split_at(eq);
        // The key is the last identifier before `=`.
        let key_start = prefix
            .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-'))
            .map(|position| position + 1)
            .unwrap_or(0);
        let key = &prefix[key_start..];
        out.push_str(prefix);
        out.push('=');
        if is_sensitive(key) {
            let value = &after_eq[1..];
            // A quoted value is consumed with its quotes; otherwise run to a separator.
            let end = if value.starts_with('{') || value.starts_with('[') {
                // A structured body is dropped to the end of the line.
                value.len()
            } else if let Some(quoted) = value.strip_prefix('"') {
                quoted
                    .find('"')
                    .map(|position| position + 2)
                    .unwrap_or(value.len())
            } else {
                value
                    .find(|ch: char| ch.is_whitespace() || ch == ',' || ch == '"')
                    .unwrap_or(value.len())
            };
            out.push_str(REDACTED);
            index += eq + 1 + end;
        } else {
            index += eq + 1;
        }
    }
    out
}

/// Milliseconds since an arbitrary instant, for `duration_ms` fields.
pub fn elapsed_ms(start: Instant) -> u128 {
    start.elapsed().as_millis()
}

/// Writer wrapper that applies [`scrub_line`] to everything the subscriber emits.
pub struct ScrubWriter<W: Write> {
    inner: W,
}

impl<W: Write> ScrubWriter<W> {
    /// Wrap `inner`.
    pub fn new(inner: W) -> ScrubWriter<W> {
        ScrubWriter { inner }
    }
}

impl<W: Write> Write for ScrubWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.inner.write_all(scrub_line(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Write a line to stderr through the scrubber (used by the GUI and by tests).
pub fn write_scrubbed(mut writer: impl Write, line: &str) {
    let _ = writeln!(writer, "{}", scrub_line(line));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_lose_query_fragment_and_userinfo() {
        assert_eq!(
            sanitize_url("https://user:pass@example.test/a/b.iso?token=s3cr3t&x=1#frag"),
            "https://example.test/a/b.iso"
        );
        assert_eq!(sanitize_url("http://example.test"), "http://example.test");
        assert_eq!(sanitize_url("not a url"), "not a url");
        let long = format!("https://example.test/{}", "a".repeat(300));
        let sanitized = sanitize_url(&long);
        let path = sanitized.trim_start_matches("https://example.test");
        assert!(path.len() <= 121, "path was {} bytes", path.len());
    }

    #[test]
    fn sensitive_fields_are_redacted_and_controls_escaped() {
        assert_eq!(redact_field("Authorization", "Bearer abc"), REDACTED);
        assert_eq!(redact_field("set-cookie", "a=b"), REDACTED);
        assert_eq!(redact_field("x-api-key", "abc"), REDACTED);
        // Audit fields are not secrets and must stay readable.
        assert_eq!(redact_field("secret_backend", "keyring"), "keyring");
        assert_eq!(redact_field("checksum_ok", "true"), "true");
        assert_eq!(redact_field("access_token", "abc"), REDACTED);
        assert_eq!(redact_field("Referer", "https://x/"), "https://x/");
        assert_eq!(redact_field("note", "a\nb"), "a?b");
        let headers = redact_headers([("Authorization", "Bearer abc"), ("Accept", "text/plain")]);
        assert_eq!(headers[0].1, REDACTED);
        assert_eq!(headers[1].1, "text/plain");
    }

    #[test]
    fn the_subscriber_redacts_and_scrubs_before_writing() {
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Sink(Arc<Mutex<Vec<u8>>>);
        struct SinkWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for SinkWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
            type Writer = SinkWriter;
            fn make_writer(&'a self) -> Self::Writer {
                SinkWriter(Arc::clone(&self.0))
            }
        }

        let sink = Sink::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                task_id = "abc",
                status = "downloading",
                url = %sanitize_url("https://example.test/x.iso?token=t0k3n"),
                authorization = %redact_field("authorization", "Bearer s3cr3t"),
                "download added"
            );
        });
        let output = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        for secret in ["t0k3n", "s3cr3t"] {
            assert!(!output.contains(secret), "{secret} leaked: {output}");
        }
        assert!(output.contains(REDACTED), "{output}");
        assert!(output.contains("task_id"));
        assert!(output.contains("url=https://example.test/x.iso"));
    }

    #[test]
    fn the_writer_scrubs_a_careless_log_line() {
        let mut buffer: Vec<u8> = Vec::new();
        let mut writer = ScrubWriter::new(&mut buffer);
        writer
            .write_all(b"cookie=session-cookie-value token=abc123 body=secret-body\n")
            .unwrap();
        writer.flush().unwrap();
        let output = String::from_utf8(buffer).unwrap();
        for secret in ["session-cookie-value", "abc123", "secret-body"] {
            assert!(!output.contains(secret), "{secret} leaked: {output}");
        }
        assert!(output.contains(REDACTED), "{output}");
    }

    #[test]
    fn the_scrubber_removes_assignments_and_queries() {
        let line =
            r#"add task_id=abc authorization=Bearer secret cookie="a=b" url=https://x/y?token=tok"#;
        let scrubbed = scrub_line(line);
        assert!(!scrubbed.contains("Bearer secret"), "{scrubbed}");
        assert!(!scrubbed.contains("a=b"), "{scrubbed}");
        assert!(!scrubbed.contains("token=tok"), "{scrubbed}");
        assert!(scrubbed.contains("task_id=abc"));
        assert!(scrubbed.contains(REDACTED));

        let body = "body={\"password\":\"hunter2\"}";
        assert!(!scrub_line(body).contains("hunter2"));
    }
}
