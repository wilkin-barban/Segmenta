//! Per-site overrides: a `SiteRule` wins over the global HTTP settings for its host.
//!
//! Contract: T5 of `odd/tasks/segmenta-hito1-red-inmunidad.md`.
//!
//! The rule carries only what can be sent safely: a user agent, a connection ceiling, a
//! "look like a browser" flag and extra headers. Sensitive headers never live here in a
//! plain struct owned by SQLite: the daemon keeps them in the secret store and hands them
//! over already resolved, so a database copy cannot leak a cookie.

/// One override for the hosts it matches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SiteRule {
    /// Pattern: `example.com`, `*.example.com` or `*` for every host.
    pub domain_pattern: String,
    /// User agent that replaces the global one.
    pub user_agent: Option<String>,
    /// Per-host connection ceiling.
    pub max_connections: Option<usize>,
    /// Send the headers a current browser sends.
    pub browser_like: bool,
    /// Extra headers, already resolved from the secret store.
    pub headers: Vec<(String, String)>,
}

impl SiteRule {
    /// Does this rule apply to `host`?
    ///
    /// Matching is deliberately narrow: the pattern is compared against the host and its
    /// parent domains, so `example.com` also covers `cdn.example.com` but `ample.com`
    /// never matches. A leading `*.` is accepted and means the same thing.
    pub fn matches(&self, host: &str) -> bool {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        let pattern = self
            .domain_pattern
            .trim()
            .trim_start_matches("*.")
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if pattern.is_empty() || host.is_empty() {
            return false;
        }
        if pattern == "*" {
            return true;
        }
        host == pattern || host.ends_with(&format!(".{pattern}"))
    }

    /// Effective user agent, falling back to the caller's default.
    pub fn user_agent_or<'a>(&'a self, default: &'a str) -> &'a str {
        self.user_agent
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(default)
    }

    /// Connection ceiling, clamped to what the engine accepts.
    pub fn connection_limit(&self, fallback: usize, ceiling: usize) -> usize {
        self.max_connections
            .unwrap_or(fallback)
            .clamp(1, ceiling.max(1))
    }

    /// Headers a browser sends, minus the ones that would break a request.
    pub fn browser_headers(&self) -> Vec<(String, String)> {
        if !self.browser_like {
            return Vec::new();
        }
        vec![
            (
                "accept".to_string(),
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8"
                    .to_string(),
            ),
            ("accept-language".to_string(), "en-US,en;q=0.9".to_string()),
            (
                "sec-fetch-dest".to_string(),
                "document".to_string(),
            ),
            ("sec-fetch-mode".to_string(), "navigate".to_string()),
            ("sec-fetch-site".to_string(), "none".to_string()),
            ("upgrade-insecure-requests".to_string(), "1".to_string()),
        ]
    }

    /// Every header the rule contributes, browser set first and explicit ones last.
    pub fn all_headers(&self) -> Vec<(String, String)> {
        let mut headers = self.browser_headers();
        headers.extend(self.headers.iter().cloned());
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(pattern: &str) -> SiteRule {
        SiteRule {
            domain_pattern: pattern.to_string(),
            ..SiteRule::default()
        }
    }

    #[test]
    fn a_pattern_covers_its_host_and_its_subdomains_only() {
        let example = rule("example.com");
        assert!(example.matches("example.com"));
        assert!(example.matches("cdn.example.com"));
        assert!(example.matches("EXAMPLE.com."));
        assert!(!example.matches("ample.com"));
        assert!(!example.matches("example.com.evil.test"));
        assert!(rule("*.example.com").matches("cdn.example.com"));
        assert!(rule("*").matches("anything.test"));
        assert!(!rule("  ").matches("anything.test"));
    }

    #[test]
    fn the_rule_overrides_only_what_it_declares() {
        let empty = SiteRule::default();
        assert_eq!(empty.user_agent_or("segmenta/0.7"), "segmenta/0.7");
        assert_eq!(empty.connection_limit(8, 128), 8);

        let custom = SiteRule {
            user_agent: Some("custom/1".to_string()),
            max_connections: Some(2),
            browser_like: true,
            headers: vec![("referer".to_string(), "https://example.com".to_string())],
            ..SiteRule::default()
        };
        assert_eq!(custom.user_agent_or("segmenta/0.7"), "custom/1");
        assert_eq!(custom.connection_limit(8, 128), 2);
        // The ceiling wins over an absurd value instead of trusting the database.
        let absurd = SiteRule {
            max_connections: Some(10_000),
            ..SiteRule::default()
        };
        assert_eq!(absurd.connection_limit(8, 128), 128);
        let headers = custom.all_headers();
        assert!(headers.iter().any(|(name, _)| name == "accept"));
        assert!(headers
            .iter()
            .any(|(name, value)| name == "referer" && value == "https://example.com"));
    }

    #[test]
    fn a_rule_without_the_browser_flag_sends_no_extra_headers() {
        let plain = SiteRule {
            headers: vec![("x-test".to_string(), "1".to_string())],
            ..SiteRule::default()
        };
        let headers = plain.all_headers();
        assert_eq!(headers, vec![("x-test".to_string(), "1".to_string())]);
    }
}
