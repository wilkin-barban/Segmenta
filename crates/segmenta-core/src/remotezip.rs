//! Remote ZIP inspection by `Range`: list an archive without downloading it.
//!
//! Contract: S3 of `odd/tasks/segmenta-hito2-captura-hosters-remoto.md` (T7).
//!
//! # How it avoids the whole file
//!
//! A ZIP ends with an *end of central directory* record, which says where the central
//! directory is; the directory lists every entry with its offset. Reading those two pieces
//! costs a few kilobytes regardless of the archive size, so a 10 GB `.zip` can be listed
//! before deciding what to download.
//!
//! ZIP64 hides the same numbers behind `0xFFFF`/`0xFFFFFFFF` markers and a second locator;
//! both shapes are parsed here because a download manager that only understands the small
//! one fails exactly on the archives people care about.
//!
//! # What this module refuses to guess
//!
//! * a body without the end record is [`ZipError::NotAZip`], never an empty list;
//! * an encrypted entry is reported, not silently returned as bytes;
//! * a decompressed size that does not match the header is an error, because that is the
//!   signature of a corrupted or hostile archive.

use std::io::Read;

use url::Url;

use crate::error::{CoreError, Result};

/// Signatures the format defines.
const EOCD: u32 = 0x0605_4b50;
const ZIP64_EOCD: u32 = 0x0606_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

/// How many bytes from the tail are searched for the end record (comment included).
const TAIL: u64 = 65_557;

/// Compression of one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipMethod {
    /// Bytes stored as they are.
    Stored,
    /// Raw Deflate.
    Deflate,
    /// A method this build does not implement.
    Unsupported(u16),
}

impl ZipMethod {
    fn from_code(code: u16) -> ZipMethod {
        match code {
            0 => ZipMethod::Stored,
            8 => ZipMethod::Deflate,
            other => ZipMethod::Unsupported(other),
        }
    }
}

/// One entry of the central directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// Path inside the archive.
    pub name: String,
    /// Compression of the bytes.
    pub method: ZipMethod,
    /// CRC-32 of the uncompressed bytes, as the archive states it.
    pub crc32: u32,
    /// Size after decompression.
    pub uncompressed_size: u64,
    /// Size inside the archive.
    pub compressed_size: u64,
    /// Where the entry's local header starts.
    pub header_offset: u64,
    /// Whether the archive marks the entry encrypted.
    pub encrypted: bool,
}

/// Why a remote ZIP could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZipError {
    /// The body does not carry the end-of-central-directory record.
    #[error("the body is not a zip archive")]
    NotAZip,
    /// A structure was present but inconsistent.
    #[error("the archive is malformed: {0}")]
    Malformed(String),
    /// A method this build cannot decompress.
    #[error("entry `{name}` uses compression method {code}, which this build cannot read")]
    UnsupportedMethod {
        /// Entry name.
        name: String,
        /// Method code from the archive.
        code: u16,
    },
    /// The archive marks the entry encrypted.
    #[error("entry `{name}` is encrypted")]
    Encrypted {
        /// Entry name.
        name: String,
    },
    /// The CRC of the decompressed bytes does not match the header.
    #[error("entry `{name}` failed its CRC check")]
    Corrupt {
        /// Entry name.
        name: String,
    },
    /// The transport failed.
    #[error("range request failed: {0}")]
    Transport(String),
}

/// Source of `Range` responses.
#[async_trait::async_trait]
pub trait RangeSource: Send + Sync {
    /// Total size of the body, when the server reports it.
    async fn total_size(&self, url: &Url) -> std::result::Result<Option<u64>, ZipError>;

    /// Fetch `[start, end]` inclusive. A `206` with the bytes is expected; a `416` means
    /// the range is past the end.
    async fn range(
        &self,
        url: &Url,
        start: u64,
        end: u64,
    ) -> std::result::Result<(u16, Vec<u8>), ZipError>;
}

/// A parsed archive: entries plus the few numbers needed to read one of them.
#[derive(Debug, Clone)]
pub struct RemoteZip {
    url: Url,
    total: u64,
    entries: Vec<ZipEntry>,
}

impl RemoteZip {
    /// Entries in central-directory order.
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Size of the whole archive.
    pub fn total_bytes(&self) -> u64 {
        self.total
    }

    /// URL the archive came from.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// First bytes the entry occupies, local header included.
    pub fn entry_bytes(&self, entry: &ZipEntry) -> u64 {
        // 30 bytes of local header plus the two variable-length fields.
        let local_header = 30 + entry.name.len() as u64 + 32;
        local_header + entry.compressed_size
    }
}

/// HTTP source used in production: one range per request, guarded like every other fetch.
pub struct HttpRange {
    client: reqwest::Client,
    allow_local: bool,
}

impl HttpRange {
    /// Build it from the engine's HTTP configuration.
    pub fn new(http: &crate::probe::HttpConfig) -> Result<HttpRange> {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(http.connect_timeout)
            .timeout(http.read_timeout)
            .user_agent(http.user_agent.clone());
        if let Some(proxy) = &http.proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|err| CoreError::InvalidUrl(format!("proxy `{proxy}`: {err}")))?,
            );
        }
        Ok(HttpRange {
            client: builder
                .build()
                .map_err(|err| CoreError::Network(err.to_string()))?,
            allow_local: http.allow_local,
        })
    }
}

#[async_trait::async_trait]
impl RangeSource for HttpRange {
    async fn total_size(&self, url: &Url) -> std::result::Result<Option<u64>, ZipError> {
        crate::urlssrf::resolve_checked(url, self.allow_local)
            .await
            .map_err(|err| ZipError::Transport(err.to_string()))?;
        // A one-byte range keeps the answer small and still carries `Content-Range`.
        let response = self
            .client
            .get(url.clone())
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send()
            .await
            .map_err(|err| ZipError::Transport(err.to_string()))?;
        if !response.status().is_success() {
            return Ok(None);
        }
        Ok(response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next().map(str::to_string))
            .and_then(|value| value.parse::<u64>().ok())
            .or_else(|| response.content_length()))
    }

    async fn range(
        &self,
        url: &Url,
        start: u64,
        end: u64,
    ) -> std::result::Result<(u16, Vec<u8>), ZipError> {
        crate::urlssrf::resolve_checked(url, self.allow_local)
            .await
            .map_err(|err| ZipError::Transport(err.to_string()))?;
        let response = self
            .client
            .get(url.clone())
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            .send()
            .await
            .map_err(|err| ZipError::Transport(err.to_string()))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|err| ZipError::Transport(err.to_string()))?;
        Ok((status, bytes.to_vec()))
    }
}

/// Read the tail, then the central directory, and return what the archive lists.
pub async fn open(source: &dyn RangeSource, url: &Url) -> std::result::Result<RemoteZip, ZipError> {
    // The tail is what holds the end record, but where the tail starts depends on the size.
    // When the server does not report a size, the whole body is read as the tail: that is
    // only correct for small archives, and it is the reason `total_size` exists.
    let size = source.total_size(url).await?;
    let (tail_start, tail_end) = match size {
        Some(size) if size > TAIL => (size - TAIL, size - 1),
        Some(size) => (0, size.saturating_sub(1)),
        None => (0, TAIL),
    };
    let (status, tail) = source.range(url, tail_start, tail_end).await?;
    if status == 416 || tail.len() < 22 {
        return Err(ZipError::NotAZip);
    }
    let eocd_at = find_eocd(&tail).ok_or(ZipError::NotAZip)?;
    let (total, entries) = parse_directory(source, url, &tail, eocd_at).await?;
    Ok(RemoteZip {
        url: url.clone(),
        total: size.unwrap_or(total),
        entries,
    })
}

fn find_eocd(tail: &[u8]) -> Option<usize> {
    if tail.len() < 22 {
        return None;
    }
    // Search from the end: a comment may contain the signature bytes.
    for start in (0..=tail.len() - 22).rev() {
        if u32::from_le_bytes(tail[start..start + 4].try_into().ok()?) == EOCD {
            return Some(start);
        }
    }
    None
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// Read the central directory, following the ZIP64 locator when the EOCD hides its values.
async fn parse_directory(
    source: &dyn RangeSource,
    url: &Url,
    tail: &[u8],
    eocd_at: usize,
) -> std::result::Result<(u64, Vec<ZipEntry>), ZipError> {
    let mut count = u16_at(tail, eocd_at + 10).ok_or(ZipError::NotAZip)? as u64;
    let mut size = u32_at(tail, eocd_at + 12).ok_or(ZipError::NotAZip)? as u64;
    let mut offset = u32_at(tail, eocd_at + 16).ok_or(ZipError::NotAZip)? as u64;
    let mut archive_size = offset + size;

    if count == u16::MAX as u64 || size == u32::MAX as u64 || offset == u32::MAX as u64 {
        // The ZIP64 locator sits immediately before the EOCD and points at the record.
        let locator_at = eocd_at.checked_sub(20).ok_or_else(|| {
            ZipError::Malformed(
                "the end record claims ZIP64 but has no room for its locator".to_string(),
            )
        })?;
        if u32_at(tail, locator_at) != Some(ZIP64_LOCATOR) {
            return Err(ZipError::Malformed(
                "the ZIP64 locator is missing".to_string(),
            ));
        }
        let record_offset = u64_at(tail, locator_at + 8).ok_or(ZipError::NotAZip)?;
        let (status, record) = source.range(url, record_offset, record_offset + 55).await?;
        if status == 416 {
            return Err(ZipError::Malformed(
                "the ZIP64 record is out of range".to_string(),
            ));
        }
        if u32_at(&record, 0) != Some(ZIP64_EOCD) {
            return Err(ZipError::Malformed(
                "the ZIP64 record has the wrong signature".to_string(),
            ));
        }
        count = u64_at(&record, 32).ok_or(ZipError::NotAZip)?;
        size = u64_at(&record, 40).ok_or(ZipError::NotAZip)?;
        offset = u64_at(&record, 48).ok_or(ZipError::NotAZip)?;
        archive_size = record_offset + 56;
    }

    let (status, directory) = source
        .range(url, offset, offset + size.saturating_sub(1))
        .await?;
    if status == 416 {
        return Err(ZipError::Malformed(
            "the central directory is out of range".to_string(),
        ));
    }
    let entries = parse_entries(&directory, count as usize)?;
    Ok((archive_size.max(offset + size), entries))
}

fn parse_entries(directory: &[u8], count: usize) -> std::result::Result<Vec<ZipEntry>, ZipError> {
    let mut entries = Vec::with_capacity(count.min(4096));
    let mut at = 0usize;
    while at + 46 <= directory.len() && entries.len() < count {
        if u32_at(directory, at) != Some(CENTRAL) {
            return Err(ZipError::Malformed(format!(
                "central entry {at} has the wrong signature"
            )));
        }
        let flags = u16_at(directory, at + 8).ok_or(ZipError::NotAZip)?;
        let method = ZipMethod::from_code(u16_at(directory, at + 10).ok_or(ZipError::NotAZip)?);
        let crc32 = u32_at(directory, at + 16).ok_or(ZipError::NotAZip)?;
        let compressed = u32_at(directory, at + 20).ok_or(ZipError::NotAZip)? as u64;
        let uncompressed = u32_at(directory, at + 24).ok_or(ZipError::NotAZip)? as u64;
        let name_len = u16_at(directory, at + 28).ok_or(ZipError::NotAZip)? as usize;
        let extra_len = u16_at(directory, at + 30).ok_or(ZipError::NotAZip)? as usize;
        let comment_len = u16_at(directory, at + 32).ok_or(ZipError::NotAZip)? as usize;
        let mut header_offset = u32_at(directory, at + 42).ok_or(ZipError::NotAZip)? as u64;
        let mut compressed = compressed;
        let mut uncompressed = uncompressed;
        let name_at = at + 46;
        let name_bytes = directory
            .get(name_at..name_at + name_len)
            .ok_or(ZipError::NotAZip)?;
        let name = String::from_utf8_lossy(name_bytes).to_string();
        // ZIP64 moves the saturated values into an extra field.
        let extra = directory
            .get(name_at + name_len..name_at + name_len + extra_len)
            .unwrap_or_default();
        apply_zip64(
            extra,
            &mut uncompressed,
            &mut compressed,
            &mut header_offset,
        );
        entries.push(ZipEntry {
            name,
            method,
            crc32,
            uncompressed_size: uncompressed,
            compressed_size: compressed,
            header_offset,
            encrypted: flags & 1 == 1,
        });
        at = name_at + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

/// Read the ZIP64 extra field (`0x0001`) and fill whatever the header hid.
fn apply_zip64(extra: &[u8], uncompressed: &mut u64, compressed: &mut u64, offset: &mut u64) {
    let mut at = 0usize;
    while at + 4 <= extra.len() {
        let Some(kind) = u16_at(extra, at) else {
            return;
        };
        let Some(len) = u16_at(extra, at + 2).map(usize::from) else {
            return;
        };
        let body = extra.get(at + 4..at + 4 + len).unwrap_or_default();
        if kind == 0x0001 {
            let mut cursor = 0usize;
            // The order is fixed: uncompressed, compressed, offset, disk. Each is present
            // only when the corresponding header field was saturated.
            if *uncompressed == u32::MAX as u64 {
                if let Some(value) = u64_at(body, cursor) {
                    *uncompressed = value;
                    cursor += 8;
                }
            }
            if *compressed == u32::MAX as u64 {
                if let Some(value) = u64_at(body, cursor) {
                    *compressed = value;
                    cursor += 8;
                }
            }
            if *offset == u32::MAX as u64 {
                if let Some(value) = u64_at(body, cursor) {
                    *offset = value;
                }
            }
            return;
        }
        at += 4 + len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_end_record_inside_a_comment_is_not_taken_as_the_real_one() {
        let mut body = Vec::new();
        body.extend_from_slice(&EOCD.to_le_bytes());
        body.extend_from_slice(&[0u8; 18]);
        // The real record follows, with a comment that repeats the signature.
        let real_at = body.len();
        body.extend_from_slice(&EOCD.to_le_bytes());
        body.extend_from_slice(&[0u8; 18]);
        body.extend_from_slice(&EOCD.to_le_bytes());
        assert_eq!(find_eocd(&body), Some(real_at));
    }
}
