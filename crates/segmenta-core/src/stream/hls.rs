//! HLS (M3U8) master and media playlist parsing.
//!
//! Only tags that matter for a public, unencrypted, static download are interpreted;
//! byte ranges, subtitles and closed captions are deliberately ignored.

use url::Url;

use super::{attr, malformed, parse_attributes, resolve, StreamError};

/// One `#EXT-X-STREAM-INF` variant of a master playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlsVariant {
    /// Playlist URL of the variant, already resolved against the master URL.
    pub uri: Url,
    /// `BANDWIDTH` in bits per second.
    pub bandwidth: Option<u64>,
    /// `AVERAGE-BANDWIDTH` in bits per second.
    pub average_bandwidth: Option<u64>,
    /// `RESOLUTION` of the variant.
    pub resolution: Option<Resolution>,
    /// Raw `CODECS` attribute.
    pub codecs: Option<String>,
}

impl HlsVariant {
    /// Variant height, when the master declares a resolution.
    pub fn height(&self) -> Option<u32> {
        self.resolution.map(|resolution| resolution.height)
    }
}

/// Width/height pair from `RESOLUTION=WxH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// Horizontal pixels.
    pub width: u32,
    /// Vertical pixels.
    pub height: u32,
}

/// One `#EXT-X-MEDIA` rendition (audio in the 0.1.0 scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlsRendition {
    /// `TYPE`: `AUDIO`, `VIDEO`, `SUBTITLES` or `CLOSED-CAPTIONS`.
    pub kind: Option<String>,
    /// `GROUP-ID` that ties the rendition to a variant.
    pub group_id: Option<String>,
    /// `NAME` shown to the user.
    pub name: Option<String>,
    /// `LANGUAGE` tag.
    pub language: Option<String>,
    /// Media playlist URL, absent for `CLOSED-CAPTIONS`.
    pub uri: Option<Url>,
    /// `DEFAULT=YES`.
    pub is_default: bool,
}

/// A parsed master playlist.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HlsMaster {
    /// Variants in manifest order.
    pub variants: Vec<HlsVariant>,
    /// `EXT-X-MEDIA` renditions in manifest order.
    pub audio: Vec<HlsRendition>,
}

/// One media segment of a media playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct HlsSegment {
    /// Segment URL, already resolved against the playlist URL.
    pub uri: Url,
    /// `#EXTINF` duration in seconds.
    pub duration: Option<f64>,
    /// An `#EXT-X-DISCONTINUITY` precedes this segment.
    pub discontinuity: bool,
}

/// A parsed media playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct HlsMedia {
    /// Initialization segment from `#EXT-X-MAP`.
    pub init: Option<Url>,
    /// Segments in playback order.
    pub segments: Vec<HlsSegment>,
    /// `#EXT-X-TARGETDURATION` in seconds.
    pub target_duration: Option<f64>,
}

/// Parse a master playlist, resolving every URI against `base`.
pub fn parse_master(body: &str, base: &Url) -> Result<HlsMaster, StreamError> {
    require_header(body)?;
    let mut master = HlsMaster::default();
    let mut pending: Option<Vec<(String, String)>> = None;
    for line in lines(body) {
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            if pending.is_some() {
                return Err(malformed("#EXT-X-STREAM-INF without a URI"));
            }
            pending = Some(parse_attributes(rest));
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA:") {
            let attrs = parse_attributes(rest);
            let kind = attr(&attrs, "TYPE");
            if !kind
                .map(|kind| kind.eq_ignore_ascii_case("AUDIO"))
                .unwrap_or(false)
            {
                // Subtitles, closed captions and alternate video are out of scope.
                continue;
            }
            let uri = match attr(&attrs, "URI") {
                Some(raw) => Some(resolve(base, raw)?),
                None => None,
            };
            master.audio.push(HlsRendition {
                kind: kind.map(str::to_string),
                group_id: attr(&attrs, "GROUP-ID").map(str::to_string),
                name: attr(&attrs, "NAME").map(str::to_string),
                language: attr(&attrs, "LANGUAGE").map(str::to_string),
                uri,
                is_default: attr(&attrs, "DEFAULT")
                    .map(|value| value.eq_ignore_ascii_case("YES"))
                    .unwrap_or(false),
            });
        } else if line.starts_with('#') {
            continue;
        } else if let Some(attrs) = pending.take() {
            master.variants.push(HlsVariant {
                uri: resolve(base, line)?,
                bandwidth: number(&attrs, "BANDWIDTH"),
                average_bandwidth: number(&attrs, "AVERAGE-BANDWIDTH"),
                resolution: attr(&attrs, "RESOLUTION").and_then(parse_resolution),
                codecs: attr(&attrs, "CODECS").map(str::to_string),
            });
        }
    }
    if pending.is_some() {
        return Err(malformed("#EXT-X-STREAM-INF without a URI"));
    }
    Ok(master)
}

/// Parse a media playlist, resolving every URI against `base`.
///
/// A playlist without `#EXT-X-ENDLIST` is live; a key with a method other than
/// `NONE` is DRM.
pub fn parse_media_playlist(body: &str, base: &Url) -> Result<HlsMedia, StreamError> {
    require_header(body)?;
    let mut media = HlsMedia {
        init: None,
        segments: Vec::new(),
        target_duration: None,
    };
    let mut pending_duration: Option<f64> = None;
    let mut discontinuity = false;
    let mut end_list = false;
    let mut encrypted = false;
    for line in lines(body) {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let raw = rest.split(',').next().unwrap_or_default().trim();
            let duration: f64 = raw
                .parse()
                .map_err(|_| malformed(format!("#EXTINF duration `{raw}`")))?;
            if !duration.is_finite() || duration < 0.0 {
                return Err(malformed(format!("#EXTINF duration `{raw}`")));
            }
            pending_duration = Some(duration);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let attrs = parse_attributes(rest);
            if let Some(raw) = attr(&attrs, "URI") {
                media.init = Some(resolve(base, raw)?);
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let attrs = parse_attributes(rest);
            let method = attr(&attrs, "METHOD").unwrap_or("NONE");
            if !method.eq_ignore_ascii_case("NONE") {
                encrypted = true;
            }
        } else if line == "#EXT-X-DISCONTINUITY" {
            discontinuity = true;
        } else if line == "#EXT-X-ENDLIST" {
            end_list = true;
        } else if let Some(rest) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            media.target_duration = rest.trim().parse().ok();
        } else if line.starts_with('#') {
            continue;
        } else {
            media.segments.push(HlsSegment {
                uri: resolve(base, line)?,
                duration: pending_duration.take(),
                discontinuity: std::mem::take(&mut discontinuity),
            });
        }
    }
    // DRM outranks liveness: an encrypted playlist must never be retried as live.
    if encrypted {
        return Err(StreamError::DrmProtected);
    }
    if !end_list {
        return Err(StreamError::LiveNotSupported);
    }
    Ok(media)
}

/// Pick the variant closest to `height`: exact, else the smallest one above, else the
/// highest available. A master without any `RESOLUTION` falls back to the highest
/// declared bandwidth, and an empty master yields `None`.
pub fn select_variant(master: &HlsMaster, height: Option<u32>) -> Option<&HlsVariant> {
    let candidates: Vec<&HlsVariant> = master.variants.iter().collect();
    pick_variant(&candidates, height, HlsVariant::height).or_else(|| {
        master
            .variants
            .iter()
            .max_by_key(|variant| variant.bandwidth.unwrap_or(0))
    })
}

/// Selection rule shared with DASH: exact height, else the smallest one above, else
/// the highest one.
///
/// Candidates without a resolution are only ignored when at least one sibling
/// declares one; `None` means "no candidate carries a resolution".
pub(crate) fn pick_variant<'a, T>(
    candidates: &[&'a T],
    height: Option<u32>,
    height_of: impl Fn(&T) -> Option<u32>,
) -> Option<&'a T> {
    let sized: Vec<&T> = candidates
        .iter()
        .copied()
        .filter(|candidate| height_of(candidate).is_some())
        .collect();
    if sized.is_empty() {
        return None;
    }
    let tallest = || {
        sized
            .iter()
            .copied()
            .max_by_key(|candidate| height_of(candidate))
    };
    match height {
        None => tallest(),
        Some(wanted) => sized
            .iter()
            .copied()
            .find(|candidate| height_of(candidate) == Some(wanted))
            .or_else(|| {
                sized
                    .iter()
                    .copied()
                    .filter(|candidate| height_of(candidate).unwrap_or(0) > wanted)
                    .min_by_key(|candidate| height_of(candidate))
            })
            .or_else(tallest),
    }
}

/// Non-empty, trimmed lines of the document, BOM stripped.
fn lines(body: &str) -> impl Iterator<Item = &str> {
    body.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

/// Reject anything that is not an M3U8 document.
fn require_header(body: &str) -> Result<(), StreamError> {
    match lines(body).next() {
        Some(line) if line.starts_with("#EXTM3U") => Ok(()),
        _ => Err(malformed("missing `#EXTM3U` header")),
    }
}

fn number(attrs: &[(String, String)], name: &str) -> Option<u64> {
    attr(attrs, name).and_then(|value| value.trim().parse().ok())
}

fn parse_resolution(raw: &str) -> Option<Resolution> {
    let (width, height) = raw.trim().split_once(['x', 'X'])?;
    Some(Resolution {
        width: width.trim().parse().ok()?,
        height: height.trim().parse().ok()?,
    })
}
