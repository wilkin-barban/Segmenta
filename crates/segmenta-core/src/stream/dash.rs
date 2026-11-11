//! MPEG-DASH (MPD) parsing and segment expansion.
//!
//! The parser understands the subset required for a public, static, unencrypted
//! download: `SegmentTemplate` with `@duration` or a `SegmentTimeline`, plus the
//! `BaseURL` chain. `SegmentList`/`SegmentBase`, `ContentComponent`-based roles and
//! multi-period audio merging are out of the 0.1.0 scope.
//!
//! XML is read with a small purpose-built scanner instead of a dependency: MPDs are
//! flat, attributes are always quoted, and the only text content that matters is
//! `BaseURL`.

use url::Url;

use super::hls::pick_variant;
use super::{attr, malformed, parse_attributes, resolve, StreamError};

/// Upper bound for one representation; a hostile manifest must not allocate without limit.
const MAX_SEGMENTS: usize = 10_000;

/// Media kind of an `AdaptationSet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// `contentType="video"` or a `video/*` mime type.
    Video,
    /// `contentType="audio"` or an `audio/*` mime type.
    Audio,
    /// Text, images or anything else; never selected for download.
    Other,
}

/// A parsed MPD document.
#[derive(Debug, Clone, PartialEq)]
pub struct DashManifest {
    /// `MPD@mediaPresentationDuration` in seconds.
    pub duration_seconds: Option<f64>,
    /// `MPD@type`; anything other than `static` is rejected by [`parse_mpd`].
    pub is_static: bool,
    /// Effective `MPD/BaseURL`, already resolved against the document URL.
    pub base_url: Url,
    /// Periods in document order.
    pub periods: Vec<Period>,
}

/// One `Period`.
#[derive(Debug, Clone, PartialEq)]
pub struct Period {
    /// `@id` when present.
    pub id: Option<String>,
    /// `@duration` in seconds when present.
    pub duration_seconds: Option<f64>,
    /// Adaptation sets in document order.
    pub adaptations: Vec<AdaptationSet>,
}

/// One `AdaptationSet`.
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptationSet {
    /// `@id` when present.
    pub id: Option<String>,
    /// Video/audio/other classification.
    pub kind: MediaKind,
    /// `@mimeType`.
    pub mime_type: Option<String>,
    /// `@codecs`.
    pub codecs: Option<String>,
    /// Set-level `SegmentTemplate`, inherited by the representations.
    pub segment_template: Option<SegmentTemplate>,
    /// Representations in document order.
    pub representations: Vec<Representation>,
}

/// One `Representation`.
#[derive(Debug, Clone, PartialEq)]
pub struct Representation {
    /// `@id`, used to address the segment list.
    pub id: String,
    /// `@bandwidth` in bits per second.
    pub bandwidth: Option<u64>,
    /// `@width` in pixels.
    pub width: Option<u32>,
    /// `@height` in pixels.
    pub height: Option<u32>,
    /// `@codecs`.
    pub codecs: Option<String>,
    /// `@mimeType`, falling back to the adaptation set value.
    pub mime_type: Option<String>,
    /// Effective base URL for the representation segments.
    pub base_url: Url,
    /// Representation-level `SegmentTemplate`, overriding the set-level one.
    pub segment_template: Option<SegmentTemplate>,
}

/// `SegmentTemplate` element, from either the set or the representation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SegmentTemplate {
    /// `@media` template with `$Number$`/`$Time$` placeholders.
    pub media: Option<String>,
    /// `@initialization` template.
    pub initialization: Option<String>,
    /// `@startNumber`; defaults to 1 when absent.
    pub start_number: Option<u64>,
    /// `@timescale`; defaults to 1 when absent.
    pub timescale: Option<u64>,
    /// `@duration` in `@timescale` units.
    pub duration: Option<u64>,
    /// `SegmentTimeline` entries, when present.
    pub timeline: Vec<TimelineEntry>,
}

impl SegmentTemplate {
    /// First segment number, 1 by default.
    pub fn start_number(&self) -> u64 {
        self.start_number.unwrap_or(1)
    }

    /// Ticks per second, 1 by default. A zero `@timescale` is treated as 1.
    pub fn timescale(&self) -> u64 {
        self.timescale.unwrap_or(1).max(1)
    }

    /// Segment duration in seconds when `@duration` is present.
    pub fn duration_seconds(&self) -> Option<f64> {
        self.duration
            .map(|duration| duration as f64 / self.timescale() as f64)
    }

    /// Overlay `other` on `self`; representation-level values win.
    fn merged_with(&self, other: &SegmentTemplate) -> SegmentTemplate {
        SegmentTemplate {
            media: other.media.clone().or_else(|| self.media.clone()),
            initialization: other
                .initialization
                .clone()
                .or_else(|| self.initialization.clone()),
            start_number: other.start_number.or(self.start_number),
            timescale: other.timescale.or(self.timescale),
            duration: other.duration.or(self.duration),
            timeline: if other.timeline.is_empty() {
                self.timeline.clone()
            } else {
                other.timeline.clone()
            },
        }
    }
}

/// One `SegmentTimeline/S` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineEntry {
    /// `@t` in `@timescale` units; absent means "continue from the previous entry".
    pub t: Option<i64>,
    /// `@d`, the segment duration in `@timescale` units.
    pub d: u64,
    /// `@r`, additional repeats; `-1` means "until the period ends".
    pub r: i64,
}

/// Result of expanding a representation into concrete URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashSegments {
    /// Initialization segment, when the template declares one.
    pub init: Option<Url>,
    /// Media segments in playback order.
    pub media: Vec<Url>,
}

/// Parse an MPD, resolving the `BaseURL` chain against the document URL.
pub fn parse_mpd(body: &str, base: &Url) -> Result<DashManifest, StreamError> {
    let root = parse_xml(body)?;
    if root.name != "MPD" {
        return Err(malformed(format!(
            "root element is `{}`, expected `MPD`",
            root.name
        )));
    }
    // DRM outranks liveness, exactly like the HLS media playlist.
    if root.find("ContentProtection").is_some() {
        return Err(StreamError::DrmProtected);
    }
    let is_static = root
        .attr("type")
        .map(|kind| kind.eq_ignore_ascii_case("static"))
        .unwrap_or(true);
    if !is_static {
        return Err(StreamError::LiveNotSupported);
    }

    let base_url = match element_base(&root) {
        Some(raw) => resolve(base, raw)?,
        None => base.clone(),
    };
    let duration_seconds = root
        .attr("mediaPresentationDuration")
        .and_then(parse_iso_duration);

    let mut periods = Vec::new();
    for period_node in root.children_named("Period") {
        let period_base = match element_base(period_node) {
            Some(raw) => resolve(&base_url, raw)?,
            None => base_url.clone(),
        };
        let mut period = Period {
            id: period_node.attr("id").map(str::to_string),
            duration_seconds: period_node.attr("duration").and_then(parse_iso_duration),
            adaptations: Vec::new(),
        };
        for set_node in period_node.children_named("AdaptationSet") {
            let set_base = match element_base(set_node) {
                Some(raw) => resolve(&period_base, raw)?,
                None => period_base.clone(),
            };
            let kind = media_kind(set_node);
            let set_codecs = set_node.attr("codecs").map(str::to_string);
            let set_mime = set_node.attr("mimeType").map(str::to_string);
            let mut adaptation = AdaptationSet {
                id: set_node.attr("id").map(str::to_string),
                kind,
                mime_type: set_mime.clone(),
                codecs: set_codecs.clone(),
                segment_template: set_node
                    .child("SegmentTemplate")
                    .map(parse_template)
                    .transpose()?,
                representations: Vec::new(),
            };
            for rep_node in set_node.children_named("Representation") {
                let id = rep_node
                    .attr("id")
                    .ok_or_else(|| malformed("Representation without `@id`"))?
                    .to_string();
                let rep_base = match element_base(rep_node) {
                    Some(raw) => resolve(&set_base, raw)?,
                    None => set_base.clone(),
                };
                adaptation.representations.push(Representation {
                    bandwidth: node_number(rep_node, "bandwidth"),
                    width: node_number(rep_node, "width")
                        .and_then(|value| u32::try_from(value).ok()),
                    height: node_number(rep_node, "height")
                        .and_then(|value| u32::try_from(value).ok()),
                    codecs: rep_node
                        .attr("codecs")
                        .map(str::to_string)
                        .or(set_codecs.clone()),
                    mime_type: rep_node
                        .attr("mimeType")
                        .map(str::to_string)
                        .or(set_mime.clone()),
                    base_url: rep_base,
                    segment_template: rep_node
                        .child("SegmentTemplate")
                        .map(parse_template)
                        .transpose()?,
                    id,
                });
            }
            period.adaptations.push(adaptation);
        }
        periods.push(period);
    }

    Ok(DashManifest {
        duration_seconds,
        is_static,
        base_url,
        periods,
    })
}

/// Expand the segment list of `rep_id`.
///
/// Multi-period manifests expand the first representation carrying that id; merging
/// the periods of one representation is left to a later release.
pub fn segments_for(manifest: &DashManifest, rep_id: &str) -> Result<DashSegments, StreamError> {
    let mut located: Option<(&Representation, SegmentTemplate, Option<f64>)> = None;
    for period in &manifest.periods {
        let period_duration = period.duration_seconds.or(manifest.duration_seconds);
        for adaptation in &period.adaptations {
            for representation in &adaptation.representations {
                if representation.id != rep_id {
                    continue;
                }
                let template = match (
                    &adaptation.segment_template,
                    &representation.segment_template,
                ) {
                    (Some(set_level), Some(rep_level)) => set_level.merged_with(rep_level),
                    (Some(set_level), None) => set_level.clone(),
                    (None, Some(rep_level)) => rep_level.clone(),
                    (None, None) => {
                        return Err(StreamError::Unsupported(format!(
                            "representation `{rep_id}` uses SegmentList/SegmentBase, which 0.1.0 does not support"
                        )));
                    }
                };
                located = Some((representation, template, period_duration));
            }
        }
    }
    let (representation, template, period_duration) = located.ok_or_else(|| {
        StreamError::Unsupported(format!("representation `{rep_id}` is not in the manifest"))
    })?;

    let init = match template.initialization.as_deref() {
        Some(pattern) => Some(resolve(
            &representation.base_url,
            &substitute(pattern, representation, None, None)?,
        )?),
        None => None,
    };

    let media_pattern = template.media.as_deref().ok_or_else(|| {
        StreamError::Unsupported(format!(
            "representation `{rep_id}` has a SegmentTemplate without `@media`"
        ))
    })?;

    let mut media = Vec::new();
    if template.timeline.is_empty() {
        let duration = template.duration_seconds().ok_or_else(|| {
            StreamError::Unsupported(format!(
                "representation `{rep_id}` has neither `@duration` nor a SegmentTimeline"
            ))
        })?;
        let total = period_duration.ok_or_else(|| {
            StreamError::Unsupported(
                "`@mediaPresentationDuration` is required with `SegmentTemplate@duration`"
                    .to_string(),
            )
        })?;
        if duration <= 0.0 || !duration.is_finite() {
            return Err(malformed("`SegmentTemplate@duration` is zero"));
        }
        let count = (total / duration).ceil().max(1.0) as u64;
        if count as usize > MAX_SEGMENTS {
            return Err(StreamError::Unsupported(format!(
                "representation `{rep_id}` needs {count} segments, above the {MAX_SEGMENTS} limit"
            )));
        }
        let start_number = template.start_number();
        for index in 0..count {
            let number = start_number + index;
            media.push(resolve(
                &representation.base_url,
                &substitute(media_pattern, representation, Some(number), None)?,
            )?);
        }
    } else {
        for time in expand_timeline(&template, period_duration)? {
            media.push(resolve(
                &representation.base_url,
                &substitute(media_pattern, representation, None, Some(time))?,
            )?);
        }
    }

    Ok(DashSegments { init, media })
}

/// Pick the video representation closest to `height`: exact, else the smallest one
/// above, else the highest available.
pub fn select_representation(
    manifest: &DashManifest,
    height: Option<u32>,
) -> Option<&Representation> {
    let videos: Vec<&Representation> = manifest
        .periods
        .iter()
        .flat_map(|period| &period.adaptations)
        .filter(|adaptation| adaptation.kind == MediaKind::Video)
        .flat_map(|adaptation| &adaptation.representations)
        .collect();
    pick_variant(&videos, height, |representation| representation.height).or_else(|| {
        videos
            .iter()
            .copied()
            .max_by_key(|rep| rep.bandwidth.unwrap_or(0))
    })
}

/// Segment start times (in `@timescale` ticks) of a `SegmentTimeline`.
fn expand_timeline(
    template: &SegmentTemplate,
    period_duration: Option<f64>,
) -> Result<Vec<i64>, StreamError> {
    let timescale = template.timescale() as f64;
    let mut starts = Vec::new();
    let mut cursor: i64 = 0;
    for (index, entry) in template.timeline.iter().enumerate() {
        if entry.d == 0 {
            return Err(malformed("`SegmentTimeline/S@d` must not be zero"));
        }
        let step = i64::try_from(entry.d).map_err(|_| malformed("`S@d` is too large"))?;
        let start = entry.t.unwrap_or(cursor);
        let count = if entry.r < 0 {
            let end = match template.timeline.get(index + 1).and_then(|next| next.t) {
                Some(next_start) => next_start,
                None => match period_duration {
                    Some(seconds) if seconds > 0.0 => (seconds * timescale).round() as i64,
                    // Unknown period end: emit the declared segment only.
                    _ => start + step,
                },
            };
            let span = end - start;
            if span <= 0 {
                1
            } else {
                (span + step - 1) / step
            }
        } else {
            entry
                .r
                .checked_add(1)
                .ok_or_else(|| malformed("`S@r` is too large"))?
        };
        for repeat in 0..count.max(0) {
            if starts.len() >= MAX_SEGMENTS {
                return Err(StreamError::Unsupported(format!(
                    "the SegmentTimeline expands to more than {MAX_SEGMENTS} segments"
                )));
            }
            starts.push(start + repeat * step);
        }
        cursor = start + count.max(0) * step;
    }
    Ok(starts)
}

/// Replace the `$...$` placeholders of a segment template.
fn substitute(
    pattern: &str,
    representation: &Representation,
    number: Option<u64>,
    time: Option<i64>,
) -> Result<String, StreamError> {
    if !pattern.contains('$') {
        return Ok(pattern.to_string());
    }
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut rest = pattern;
    while let Some(open) = rest.find('$') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let end = after
            .find('$')
            .ok_or_else(|| malformed(format!("unterminated `$` in `{pattern}`")))?;
        let (name, format) = match after[..end].split_once('%') {
            Some((name, format)) => (name, Some(format)),
            None => (&after[..end], None),
        };
        let value = match name {
            "RepresentationID" => representation.id.clone(),
            "Bandwidth" => representation.bandwidth.unwrap_or(0).to_string(),
            "Number" => {
                let number = number.ok_or_else(|| {
                    malformed(format!(
                        "`$Number$` used without a segment number in `{pattern}`"
                    ))
                })?;
                format_value(number, format)?
            }
            "Time" => {
                let time = time.ok_or_else(|| {
                    malformed(format!("`$Time$` used without a timeline in `{pattern}`"))
                })?;
                format_value(time.unsigned_abs(), format)?
            }
            other => {
                return Err(StreamError::Unsupported(format!(
                    "template variable `${other}$` is not supported"
                )));
            }
        };
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Apply a printf-style `%05d` width from the template.
fn format_value(value: u64, format: Option<&str>) -> Result<String, StreamError> {
    let Some(spec) = format else {
        return Ok(value.to_string());
    };
    let digits = spec
        .strip_suffix('d')
        .or_else(|| spec.strip_suffix('u'))
        .ok_or_else(|| {
            StreamError::Unsupported(format!("template format `%{spec}` is not supported"))
        })?;
    let width: usize = digits.parse().map_err(|_| {
        StreamError::Unsupported(format!("template format `%{spec}` is not supported"))
    })?;
    Ok(format!("{value:0width$}", width = width.min(64)))
}

/// `BaseURL` text of an element, when present and non-empty.
fn element_base(node: &XmlNode) -> Option<&str> {
    node.child("BaseURL")
        .map(|base| base.text.trim())
        .filter(|text| !text.is_empty())
}

fn media_kind(node: &XmlNode) -> MediaKind {
    fn classify(value: &str) -> Option<MediaKind> {
        let value = value.to_ascii_lowercase();
        if value.starts_with("video/") || value == "video" {
            Some(MediaKind::Video)
        } else if value.starts_with("audio/") || value == "audio" {
            Some(MediaKind::Audio)
        } else {
            None
        }
    }
    if let Some(kind) = node.attr("contentType").and_then(classify) {
        return kind;
    }
    node.attr("mimeType")
        .and_then(classify)
        .unwrap_or(MediaKind::Other)
}

fn parse_template(node: &XmlNode) -> Result<SegmentTemplate, StreamError> {
    let mut timeline = Vec::new();
    if let Some(timeline_node) = node.child("SegmentTimeline") {
        for entry in timeline_node.children_named("S") {
            let d = node_number(entry, "d")
                .ok_or_else(|| malformed("`SegmentTimeline/S` without `@d`"))?;
            timeline.push(TimelineEntry {
                t: entry
                    .attr("t")
                    .and_then(|value| value.trim().parse::<i64>().ok()),
                d,
                r: entry
                    .attr("r")
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .unwrap_or(0),
            });
        }
    }
    Ok(SegmentTemplate {
        media: node.attr("media").map(str::to_string),
        initialization: node.attr("initialization").map(str::to_string),
        start_number: node_number(node, "startNumber"),
        timescale: node_number(node, "timescale"),
        duration: node_number(node, "duration"),
        timeline,
    })
}

fn node_number(node: &XmlNode, name: &str) -> Option<u64> {
    node.attr(name).and_then(|value| value.trim().parse().ok())
}

/// Parse an ISO 8601 duration (`PT30S`, `PT1H2M3.5S`, `P1DT30S`) into seconds.
fn parse_iso_duration(value: &str) -> Option<f64> {
    let value = value.trim();
    let rest = value
        .strip_prefix('P')
        .or_else(|| value.strip_prefix('p'))?;
    let (date_part, time_part) = match rest.find(['T', 't']) {
        Some(index) => (&rest[..index], &rest[index + 1..]),
        None => (rest, ""),
    };
    let mut total = 0.0;
    for (amount, unit) in duration_segments(date_part) {
        match unit.to_ascii_uppercase() {
            // Years and months carry no fixed length; static manifests do not use them.
            'D' => total += amount * 86_400.0,
            'W' => total += amount * 604_800.0,
            'Y' | 'M' => {}
            _ => return None,
        }
    }
    for (amount, unit) in duration_segments(time_part) {
        match unit.to_ascii_uppercase() {
            'H' => total += amount * 3_600.0,
            'M' => total += amount * 60.0,
            'S' => total += amount,
            _ => return None,
        }
    }
    Some(total)
}

/// Split `1H2M3.5S` into `(amount, unit)` pairs.
fn duration_segments(part: &str) -> Vec<(f64, char)> {
    let mut out = Vec::new();
    let mut number = String::new();
    for character in part.chars() {
        if character.is_ascii_digit() || character == '.' || character == ',' {
            number.push(if character == ',' { '.' } else { character });
        } else {
            if let Ok(amount) = number.parse::<f64>() {
                out.push((amount, character));
            }
            number.clear();
        }
    }
    out
}

/// A deliberately small XML element tree: name, attributes, text and children.
#[derive(Debug, Clone, Default)]
struct XmlNode {
    name: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<XmlNode>,
}

impl XmlNode {
    fn attr(&self, name: &str) -> Option<&str> {
        attr(&self.attrs, name)
    }

    fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XmlNode> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// Depth-first search, including this node.
    fn find(&self, name: &str) -> Option<&XmlNode> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(name))
    }
}

/// Read the document into a single root element.
fn parse_xml(body: &str) -> Result<XmlNode, StreamError> {
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut root: Option<XmlNode> = None;
    let mut cursor = 0usize;
    let body = body.trim_start_matches('\u{feff}');

    while cursor < body.len() {
        let Some(open) = body[cursor..].find('<').map(|offset| cursor + offset) else {
            let tail = &body[cursor..];
            if let Some(parent) = stack.last_mut() {
                parent.text.push_str(tail);
            }
            break;
        };
        let chunk = &body[cursor..open];
        if !chunk.trim().is_empty() {
            if let Some(parent) = stack.last_mut() {
                parent.text.push_str(chunk);
            }
        }
        let rest = &body[open..];
        if rest.starts_with("<!--") {
            cursor = open + skip(rest, "-->")?;
            continue;
        }
        if rest.starts_with("<?") {
            cursor = open + skip(rest, "?>")?;
            continue;
        }
        if rest.starts_with("<!") {
            cursor = open + skip(rest, ">")?;
            continue;
        }
        let end = rest
            .find('>')
            .ok_or_else(|| malformed("unterminated tag"))?;
        let tag = &rest[1..end];
        cursor = open + end + 1;

        if let Some(name) = tag.strip_prefix('/') {
            let name = clean_name(name);
            let node = stack
                .pop()
                .ok_or_else(|| malformed(format!("unexpected closing tag `</{name}>`")))?;
            if node.name != name {
                return Err(malformed(format!(
                    "closing tag `</{name}>` does not match `<{}>`",
                    node.name
                )));
            }
            attach(node, &mut stack, &mut root)?;
            continue;
        }

        let self_closing = tag.ends_with('/');
        let inner = if self_closing {
            &tag[..tag.len() - 1]
        } else {
            tag
        };
        let node = split_tag(inner);
        if self_closing {
            attach(node, &mut stack, &mut root)?;
        } else {
            stack.push(node);
        }
    }

    if let Some(node) = stack.last() {
        return Err(malformed(format!("unclosed tag `<{}>`", node.name)));
    }
    root.ok_or_else(|| malformed("no root element"))
}

fn attach(
    node: XmlNode,
    stack: &mut [XmlNode],
    root: &mut Option<XmlNode>,
) -> Result<(), StreamError> {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => {
            if root.is_some() {
                return Err(malformed("more than one root element"));
            }
            *root = Some(node);
        }
    }
    Ok(())
}

/// Length of the document consumed by `closing`, comment markers included.
fn skip(rest: &str, closing: &str) -> Result<usize, StreamError> {
    let end = rest
        .find(closing)
        .ok_or_else(|| malformed(format!("unterminated `{closing}`")))?;
    Ok(end + closing.len())
}

fn split_tag(tag: &str) -> XmlNode {
    let tag = tag.trim();
    let name_end = tag
        .find(|character: char| character.is_ascii_whitespace())
        .unwrap_or(tag.len());
    XmlNode {
        name: clean_name(&tag[..name_end]),
        attrs: parse_attributes(&tag[name_end..]),
        text: String::new(),
        children: Vec::new(),
    }
}

/// Drop the namespace prefix of a tag name.
fn clean_name(name: &str) -> String {
    let name = name.trim();
    match name.split_once(':') {
        Some((_, local)) => local.to_string(),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_durations_cover_the_media_forms() {
        assert_eq!(parse_iso_duration("PT30S"), Some(30.0));
        assert_eq!(parse_iso_duration("PT1H2M3.5S"), Some(3723.5));
        assert_eq!(parse_iso_duration("PT0H0M12.000S"), Some(12.0));
        assert_eq!(parse_iso_duration("P1DT30S"), Some(86_430.0));
        assert_eq!(parse_iso_duration("P1W"), Some(604_800.0));
        assert_eq!(parse_iso_duration("30S"), None);
        assert_eq!(parse_iso_duration("PT30X"), None);
    }

    #[test]
    fn xml_scanner_ignores_prolog_comments_and_namespaces() {
        let body = "<?xml version=\"1.0\"?><!-- note --><mpd:MPD type=\"static\">\
                    <mpd:BaseURL>https://a.example/b/</mpd:BaseURL><Period/></mpd:MPD>";
        let root = parse_xml(body).expect("well formed");
        assert_eq!(root.name, "MPD");
        assert_eq!(root.attr("type"), Some("static"));
        assert_eq!(
            root.child("BaseURL").map(|node| node.text.as_str()),
            Some("https://a.example/b/")
        );
        assert!(root.child("Period").is_some());
        assert!(root
            .child("BaseURL")
            .is_some_and(|node| node.children.is_empty()));
    }

    #[test]
    fn xml_scanner_rejects_broken_documents() {
        assert!(parse_xml("<MPD>").is_err());
        assert!(parse_xml("<MPD></Period>").is_err());
        assert!(parse_xml("<MPD/><MPD/>").is_err());
        assert!(parse_xml("<!-- unterminated").is_err());
        assert!(parse_xml("").is_err());
    }

    #[test]
    fn templates_substitute_number_width_and_representation_id() {
        let representation = Representation {
            id: "v1".to_string(),
            bandwidth: Some(900_000),
            width: None,
            height: None,
            codecs: None,
            mime_type: None,
            base_url: Url::parse("https://a.example/b/").expect("base"),
            segment_template: None,
        };
        assert_eq!(
            substitute(
                "seg-$RepresentationID$-$Number%05d$.m4s",
                &representation,
                Some(7),
                None
            )
            .expect("substitution"),
            "seg-v1-00007.m4s"
        );
        assert_eq!(
            substitute("$Bandwidth$/$Time$.m4s", &representation, None, Some(4000))
                .expect("substitution"),
            "900000/4000.m4s"
        );
        assert!(matches!(
            substitute("$Unknown$", &representation, None, None),
            Err(StreamError::Unsupported(_))
        ));
        assert!(matches!(
            substitute("$Number$", &representation, None, None),
            Err(StreamError::Malformed(_))
        ));
    }

    #[test]
    fn timeline_live_repeat_stops_at_the_period_end() {
        let template = SegmentTemplate {
            media: Some("s-$Time$.m4s".to_string()),
            initialization: None,
            start_number: None,
            timescale: Some(1000),
            duration: None,
            timeline: vec![TimelineEntry {
                t: Some(0),
                d: 4000,
                r: -1,
            }],
        };
        assert_eq!(
            expand_timeline(&template, Some(12.0)).expect("expand"),
            vec![0, 4000, 8000]
        );
        // Without a known period end the declared segment stands alone.
        assert_eq!(expand_timeline(&template, None).expect("expand"), vec![0]);
    }
}
