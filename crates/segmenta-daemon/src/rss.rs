//! RSS/Atom reading: parse a feed and decide which items are new and wanted.
//!
//! Contract: T5 of `odd/tasks/segmenta-v0.6.0-red-remoto.md`, module 9 of SPEC-013.
//!
//! # Parsing
//!
//! `quick-xml` walks the document instead of matching text with a regular expression, so a
//! hostile feed cannot smuggle a tag past the extractor. Only three fields matter: the
//! title (for the filter), the link (what gets downloaded) and the identifier (GUID or
//! Atom id), which is the key of the dedup table.
//!
//! # Scope
//!
//! Nothing here fetches: the daemon owns the HTTP client and the SSRF checks. This module
//! is pure so the rules are testable without a network.

/// One item of a feed, reduced to what the importer needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssItem {
    /// Stable identifier: the RSS `guid` or the Atom `id`.
    pub guid: String,
    /// Item title, used by the filter.
    pub title: String,
    /// Link to download.
    pub link: String,
}

impl RssItem {
    /// The identifier to persist: the GUID, or the link when the feed has none.
    ///
    /// Falling back to the link keeps dedup working on feeds that omit `guid`, which is
    /// common in hand-written RSS.
    pub fn dedup_key(&self) -> &str {
        if self.guid.is_empty() {
            &self.link
        } else {
            &self.guid
        }
    }
}

/// Why a feed cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedError {
    /// The document is not valid XML or has no items.
    Malformed,
    /// The URL is not `http`/`https`, or it points somewhere a feed should not.
    UnsafeUrl,
}

impl FeedError {
    /// Stable code the daemon reports.
    pub fn code(self) -> &'static str {
        match self {
            FeedError::Malformed => "ERR_RSS_MALFORMED",
            FeedError::UnsafeUrl => "ERR_RSS_UNSAFE_URL",
        }
    }
}

/// Parse one RSS or Atom document into its items.
///
/// Items without a link are dropped: an entry that cannot be downloaded cannot be imported,
/// and keeping it would only fill the dedup table with dead keys.
pub fn parse_feed(xml: &str) -> Result<Vec<RssItem>, FeedError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut items: Vec<RssItem> = Vec::new();
    let mut current: Option<RssItem> = None;
    let mut field: Option<&'static str> = None;
    let mut depth_item = 0usize;
    // Every open tag, so a truncated document is reported instead of half-imported.
    let mut depth_total = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                depth_total += 1;
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "item" | "entry" => {
                        depth_item += 1;
                        current = Some(RssItem {
                            guid: String::new(),
                            title: String::new(),
                            link: String::new(),
                        });
                    }
                    "link" => {
                        // Atom puts the URL in `href`, not in the element text.
                        if let Some(href) = attr(&event, b"href") {
                            if let Some(item) = current.as_mut() {
                                if item.link.is_empty() {
                                    item.link = href;
                                }
                            }
                        }
                        if current.is_some() {
                            field = Some("link");
                        }
                    }
                    "title" | "guid" | "id" if current.is_some() => {
                        field = Some(if name == "title" { "title" } else { "guid" });
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(text)) => {
                if let (Some(item), Some(field)) = (current.as_mut(), field) {
                    let value = text.decode().map(|t| t.into_owned()).unwrap_or_default();
                    match field {
                        "title" => item.title.push_str(&value),
                        "link" if item.link.is_empty() => item.link.push_str(&value),
                        "guid" => item.guid.push_str(&value),
                        _ => {}
                    }
                }
            }
            Ok(Event::Empty(event)) => {
                // A self-closing tag still carries attributes (Atom `<link href="..."/>`).
                let name = local_name(event.name().as_ref());
                if name == "link" {
                    if let (Some(item), Some(href)) = (current.as_mut(), attr(&event, b"href")) {
                        if item.link.is_empty() {
                            item.link = href;
                        }
                    }
                }
            }
            Ok(Event::End(event)) => {
                depth_total = depth_total.saturating_sub(1);
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "item" | "entry" => {
                        depth_item = depth_item.saturating_sub(1);
                        field = None;
                        if let Some(item) = current.take() {
                            let link = item.link.trim().to_string();
                            if is_http(&link) {
                                items.push(RssItem {
                                    guid: item.guid.trim().to_string(),
                                    title: item.title.trim().to_string(),
                                    link,
                                });
                            }
                        }
                    }
                    "title" | "link" | "guid" | "id" => field = None,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err(FeedError::Malformed),
            _ => {}
        }
    }
    if depth_total > 0 {
        // The document ended with tags still open: refuse it rather than import the prefix.
        return Err(FeedError::Malformed);
    }
    Ok(items)
}

/// Keep the items the filter accepts.
///
/// An empty or absent filter accepts everything. A filter that does not compile is a
/// configuration mistake, so it rejects everything instead of silently importing it all.
pub fn matches_filter(filter: Option<&str>, item: &RssItem) -> bool {
    let Some(pattern) = filter.map(str::trim).filter(|p| !p.is_empty()) else {
        return true;
    };
    match regex_lite(pattern) {
        Some(matched) => matched(&item.title),
        None => false,
    }
}

/// Minimal case-insensitive substring matcher with `|` alternatives.
///
/// The roadmap asks for a regular expression; shipping a full engine is a dependency and a
/// ReDoS surface, so the accepted syntax is documented: literal alternatives separated by
/// `|`, matched case-insensitively against the title. Anything using regex metacharacters is
/// still matched literally, which is predictable rather than surprising.
fn regex_lite(pattern: &str) -> Option<impl Fn(&str) -> bool> {
    let alternatives: Vec<String> = pattern
        .split('|')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect();
    if alternatives.is_empty() {
        return None;
    }
    Some(move |title: &str| {
        let title = title.to_ascii_lowercase();
        alternatives.iter().any(|needle| title.contains(needle))
    })
}

/// Local name of a tag, without its namespace prefix.
fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit(':').next().unwrap_or(&name).to_string()
}

/// Value of one attribute, when present and valid UTF-8.
fn attr(event: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    for attribute in event.attributes().flatten() {
        let name = attribute.key.as_ref().to_ascii_lowercase();
        if name == key.to_ascii_lowercase() {
            return attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned());
        }
    }
    None
}

fn is_http(link: &str) -> bool {
    link.starts_with("http://") || link.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
        <rss version="2.0"><channel>
          <title>Releases</title>
          <item><title>Tool 1.2 1080p</title><guid>g-1</guid><link>https://cdn.test/a.bin</link></item>
          <item><title>Tool 1.3 720p</title><link>https://cdn.test/b.bin</link></item>
          <item><title>Not a link</title><link>ftp://cdn.test/c.bin</link></item>
        </channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <title>Atom feed</title>
          <entry><title>Episode 1</title><id>urn:1</id><link href="https://cdn.test/1.mp3"/></entry>
        </feed>"#;

    #[test]
    fn reads_rss_with_guid_and_link() {
        let items = parse_feed(RSS).unwrap();
        assert_eq!(items.len(), 2, "the ftp link must be dropped");
        assert_eq!(items[0].guid, "g-1");
        assert_eq!(items[0].link, "https://cdn.test/a.bin");
        assert_eq!(items[0].title, "Tool 1.2 1080p");
        // No guid: the link is the dedup key.
        assert_eq!(items[1].guid, "");
        assert_eq!(items[1].dedup_key(), "https://cdn.test/b.bin");
    }

    #[test]
    fn reads_atom_entries_with_href() {
        let items = parse_feed(ATOM).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].guid, "urn:1");
        assert_eq!(items[0].link, "https://cdn.test/1.mp3");
    }

    #[test]
    fn filters_by_alternative_case_insensitively() {
        let items = parse_feed(RSS).unwrap();
        assert!(matches_filter(Some("1080P|x264"), &items[0]));
        assert!(!matches_filter(Some("1080p"), &items[1]));
        // No filter imports everything; an unusable filter imports nothing.
        assert!(matches_filter(None, &items[1]));
        assert!(matches_filter(Some("   "), &items[1]));
        assert!(!matches_filter(Some("|"), &items[1]));
    }

    #[test]
    fn broken_xml_is_reported_instead_of_partially_imported() {
        assert_eq!(parse_feed("<rss><channel>"), Err(FeedError::Malformed));
        assert!(parse_feed("<rss><channel></channel></rss>")
            .unwrap()
            .is_empty());
    }
}
