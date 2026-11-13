//! Human-readable rendering for the CLI: sizes, speeds, ETAs, tables and segment bars.
//!
//! Every human word comes from the [`segmenta_i18n`] catalog through [`tr`], and each
//! renderer takes an explicit [`Locale`] so tests never mutate the process environment.
//! Values owned by the daemon (task ids, URLs, paths, queue names, protocol status words,
//! stable codes) are printed verbatim and never translated.
//!
//! Catalog gaps left in English on purpose: the status block keeps the `ranges`, `error`
//! labels and the `yes`/`no` values, and the queue table keeps its `SLOTS` column, because
//! the catalog has no key for them yet.

use segmenta_daemon::rpc::DownloadView;
use segmenta_i18n::{tr, ErrorText, Locale, Msg};
use serde_json::Value;

/// Width of the `label:` column of the detail blocks.
const LABEL_WIDTH: usize = 12;

/// A padded `label:` for the key/value blocks.
fn label(locale: Locale, msg: Msg) -> String {
    format!("{:<LABEL_WIDTH$}", format!("{}:", tr(locale, msg)))
}

/// Format a byte count with binary units.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Format a transfer rate.
pub fn format_speed(bytes_per_second: u64) -> String {
    if bytes_per_second == 0 {
        "-".to_string()
    } else {
        format!("{}/s", format_bytes(bytes_per_second))
    }
}

/// Format an ETA in `1h02m`, `3m05s` or `7s`.
pub fn format_eta(seconds: Option<u64>) -> String {
    match seconds {
        None => "-".to_string(),
        Some(0) => "0s".to_string(),
        Some(total) if total >= 3600 => {
            format!("{}h{:02}m", total / 3600, (total % 3600) / 60)
        }
        Some(total) if total >= 60 => format!("{}m{:02}s", total / 60, total % 60),
        Some(total) => format!("{total}s"),
    }
}

/// Percentage of a download, or `-` when the total is unknown.
pub fn format_progress(view: &DownloadView) -> String {
    match view.total_bytes {
        Some(total) if total > 0 => {
            let percent = (view.downloaded_bytes as f64 / total as f64 * 100.0).min(100.0);
            format!("{percent:5.1}%")
        }
        _ => "  -  ".to_string(),
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let head: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Localized label for the protocol status word.
pub fn status_label(locale: Locale, status: &str) -> &'static str {
    use segmenta_i18n::Msg;
    let message = match status {
        "pending" => Msg::StatusPending,
        "probing" => Msg::StatusProbing,
        "downloading" => Msg::StatusDownloading,
        "paused" => Msg::StatusPaused,
        "verifying" => Msg::StatusVerifying,
        "completed" => Msg::StatusCompleted,
        "failed" => Msg::StatusFailed,
        "cancelled" => Msg::StatusCancelled,
        _ => Msg::StatusUnknown,
    };
    segmenta_i18n::tr(locale, message)
}

/// Render the `list` table.
pub fn render_table(locale: Locale, rows: &[DownloadView]) -> String {
    if rows.is_empty() {
        return format!("{}\n", tr(locale, Msg::CliNoDownloads));
    }
    let header = format!(
        "{:<8}  {:<11}  {:>6}  {:>10}  {:>10}  {:>8}  {}",
        tr(locale, Msg::CliTableId),
        tr(locale, Msg::CliTableStatus),
        tr(locale, Msg::CliTableProgress),
        tr(locale, Msg::CliTableSize),
        tr(locale, Msg::CliTableSpeed),
        tr(locale, Msg::CliTableEta),
        tr(locale, Msg::CliTableName),
    );
    let mut out = String::new();
    out.push_str(&header);
    out.push('\n');
    for view in rows {
        let size = view
            .total_bytes
            .map(format_bytes)
            .unwrap_or_else(|| format!("≥ {}", format_bytes(view.downloaded_bytes)));
        out.push_str(&format!(
            "{:<8}  {:<11}  {:>6}  {:>10}  {:>10}  {:>8}  {}\n",
            short_id(&view.task_id),
            status_label(locale, &view.status),
            format_progress(view),
            size,
            format_speed(view.speed_bps),
            format_eta(view.eta_seconds),
            truncate(&view.filename, 48),
        ));
    }
    out
}

/// Render one download in detail, including a per-segment bar.
pub fn render_status(locale: Locale, view: &DownloadView) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}{}\n",
        label(locale, Msg::CliTableId),
        view.task_id
    ));
    out.push_str(&format!("{}{}\n", label(locale, Msg::UrlLabel), view.url));
    out.push_str(&format!(
        "{}{}\n",
        label(locale, Msg::CliFile),
        view.final_path
    ));
    out.push_str(&format!(
        "{}{}\n",
        label(locale, Msg::CliStatus),
        status_label(locale, &view.status)
    ));
    let total = view
        .total_bytes
        .map(format_bytes)
        .unwrap_or_else(|| tr(locale, Msg::UnknownSize).to_string());
    out.push_str(&format!(
        "{}{} / {}\n",
        label(locale, Msg::CliTableProgress),
        format_bytes(view.downloaded_bytes),
        total
    ));
    out.push_str(&format!(
        "{}{}   {}{}\n",
        label(locale, Msg::Speed),
        format_speed(view.speed_bps),
        label(locale, Msg::Eta),
        format_eta(view.eta_seconds)
    ));
    // `ranges` has no catalog key yet; the label and its yes/no stay English on purpose.
    out.push_str(&format!(
        "ranges: {}  {}: {}  {}: {}\n",
        if view.supports_ranges { "yes" } else { "no" },
        tr(locale, Msg::Queues),
        view.queue_name,
        tr(locale, Msg::MaxRetries),
        view.retry_count,
    ));
    if let Some(message) = &view.error_message {
        // The daemon keeps the code as the source of truth; the client resolves the text.
        let text = match view
            .error_code
            .and_then(ErrorText::from_code)
            .map(|error| (segmenta_i18n::tr_error(locale, error), view.error_code))
        {
            Some((localized, Some(code))) => format!("{localized} (code {code})"),
            Some((localized, None)) => localized.to_string(),
            None => match view.error_code {
                Some(code) => format!("{message} (code {code})"),
                None => message.clone(),
            },
        };
        out.push_str(&format!("error:     {text}\n"));
    }
    if let Some(path) = &view.quarantine_path {
        // The file is not in the downloads folder any more: say where it is.
        out.push_str(&format!("quarantine: {path}\n"));
    }
    if !view.segments.is_empty() {
        out.push_str(&label(locale, Msg::Segments));
        for segment in &view.segments {
            let width = segment.end_offset - segment.start_offset + 1;
            let written = segment
                .next_offset
                .saturating_sub(segment.start_offset)
                .min(width);
            let ratio = if width == 0 {
                0.0
            } else {
                written as f64 / width as f64
            };
            let cells = ((ratio * 8.0).round() as usize).clamp(0, 8);
            out.push('[');
            for cell in 0..8 {
                out.push(if cell < cells { '#' } else { '.' });
            }
            out.push(']');
        }
        out.push('\n');
    }
    out
}

/// Render the settings object as `key = value` lines.
///
/// Keys and values come from the daemon: there is no human word of the CLI here, so the
/// output is identical in every locale.
pub fn render_settings(settings: &serde_json::Value) -> String {
    let Some(map) = settings.as_object() else {
        return format!("{settings}\n");
    };
    let mut out = String::new();
    for (key, value) in map {
        let rendered = match value {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Null => "null".to_string(),
            other => other.to_string(),
        };
        out.push_str(&format!("{key} = {rendered}\n"));
    }
    out
}

/// Render the queue list.
pub fn render_queues(locale: Locale, queues: &serde_json::Value) -> String {
    let Some(rows) = queues.as_array() else {
        return format!("{queues}\n");
    };
    if rows.is_empty() {
        return format!("{}\n", tr(locale, Msg::CliNoQueues));
    }
    // The `SLOTS` column has no catalog key yet and stays English on purpose.
    let mut out = format!(
        "{:<16} {:>6} {:>10} {:>10} {:>10}\n",
        tr(locale, Msg::CliTableName),
        "SLOTS",
        tr(locale, Msg::FilterActive),
        tr(locale, Msg::StatusPending),
        tr(locale, Msg::StatusPaused),
    );
    for row in rows {
        out.push_str(&format!(
            "{:<16} {:>6} {:>10} {:>10} {:>10}\n",
            row["name"].as_str().unwrap_or("-"),
            row["max_active"].as_u64().unwrap_or(0),
            row["active"].as_u64().unwrap_or(0),
            row["pending"].as_u64().unwrap_or(0),
            row["paused"].as_u64().unwrap_or(0),
        ));
    }
    out
}

pub fn render_extension_status(_locale: Locale, value: &Value) -> String {
    let Some(browsers) = value.get("browsers").and_then(Value::as_array) else {
        return format!("{value}\n");
    };
    let mut out = String::new();
    out.push_str(&format!(
        "{:<12}  {:<16}  {:<20}  {}\n",
        "ID", "NAME", "STATE", "PATH"
    ));
    for b in browsers {
        let id = b.get("id").and_then(Value::as_str).unwrap_or("-");
        let name = b.get("name").and_then(Value::as_str).unwrap_or("-");
        let state = b.get("state").and_then(Value::as_str).unwrap_or("-");
        let path = b
            .get("prepared_path")
            .or_else(|| b.get("manifest_path"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        out.push_str(&format!(
            "{:<12}  {:<16}  {:<20}  {}\n",
            id, name, state, path
        ));
    }
    out
}

pub fn render_extension_prepare(_locale: Locale, value: &Value) -> String {
    let Some(prepared) = value.get("prepared").and_then(Value::as_array) else {
        return format!("{value}\n");
    };
    let mut out = String::new();
    for p in prepared {
        let family = p.get("family").and_then(Value::as_str).unwrap_or("-");
        let path = p.get("path").and_then(Value::as_str).unwrap_or("-");
        out.push_str(&format!("prepared {family}: {path}\n"));
    }
    out
}

pub fn render_extension_install_native(_locale: Locale, value: &Value) -> String {
    let Some(installed) = value.get("installed").and_then(Value::as_array) else {
        return format!("{value}\n");
    };
    let mut out = String::new();
    for i in installed {
        let id = i.get("id").and_then(Value::as_str).unwrap_or("-");
        let path = i.get("path").and_then(Value::as_str).unwrap_or("-");
        out.push_str(&format!("installed manifest for {id}: {path}\n"));
    }
    out
}

pub fn render_extension_open(_locale: Locale, value: &Value) -> String {
    let browser = value
        .get("browser_id")
        .and_then(Value::as_str)
        .unwrap_or("browser");
    format!("opened extensions page for {browser}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use segmenta_daemon::rpc::SegmentView;

    fn view() -> DownloadView {
        DownloadView {
            task_id: "c1f7a28e-8b1b-4f9e-9d22-29f123456789".to_string(),
            url: "https://example.test/ubuntu.iso".to_string(),
            final_path: "/home/u/Downloads/ubuntu.iso".to_string(),
            filename: "ubuntu.iso".to_string(),
            status: "downloading".to_string(),
            total_bytes: Some(10 * 1024 * 1024),
            downloaded_bytes: 5 * 1024 * 1024,
            speed_bps: 1024 * 1024,
            eta_seconds: Some(5),
            retry_count: 0,
            supports_ranges: true,
            queue_name: "main".to_string(),
            created_at: None,
            completed_at: None,
            error_code: None,
            error_message: None,
            quarantine_path: None,
            media_kind: None,
            playlist_title: None,
            segments: vec![SegmentView {
                id: 1,
                start_offset: 0,
                end_offset: 1023,
                next_offset: 512,
                status: "active".to_string(),
                speed_bps: 0,
            }],
        }
    }

    #[test]
    fn byte_units_are_binary_and_stable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn eta_is_compact() {
        assert_eq!(format_eta(None), "-");
        assert_eq!(format_eta(Some(0)), "0s");
        assert_eq!(format_eta(Some(59)), "59s");
        assert_eq!(format_eta(Some(65)), "1m05s");
        assert_eq!(format_eta(Some(3700)), "1h01m");
    }

    #[test]
    fn progress_is_a_percentage_or_a_dash() {
        let mut download = view();
        assert_eq!(format_progress(&download).trim(), "50.0%");
        download.total_bytes = None;
        assert_eq!(format_progress(&download).trim(), "-");
    }

    #[test]
    fn the_table_lists_a_row_per_download_and_handles_empty() {
        assert_eq!(render_table(Locale::En, &[]), "No downloads.\n");
        let table = render_table(Locale::En, &[view()]);
        assert!(table.starts_with("ID "));
        assert!(table.contains("c1f7a28e"));
        // The status word is localized now (S21).
        assert!(table.contains("Downloading"));
        assert!(!table.contains("downloading"));
        assert!(table.contains("ubuntu.iso"));
        assert!(table.contains("10.0 MiB"));
    }

    #[test]
    fn status_renders_segments_and_errors() {
        let download = view();
        let text = render_status(Locale::En, &download);
        assert!(text.contains("c1f7a28e-8b1b-4f9e-9d22-29f123456789"));
        assert!(text.contains("[####....]"));
        // A known code is resolved in the client and keeps the code visible.
        let mut download = view();
        download.error_message = Some("disk full".to_string());
        download.error_code = Some(1006);
        assert!(render_status(Locale::En, &download).contains("Not enough disk space (code 1006)"));
        // An unknown code falls back to the daemon message.
        download.error_code = Some(4242);
        assert!(render_status(Locale::En, &download).contains("disk full (code 4242)"));
    }

    #[test]
    fn settings_render_one_key_per_line() {
        let rendered = render_settings(&serde_json::json!({
            "max_connections": 16,
            "download_dir": "/home/u/Downloads",
            "global_speed_limit_bps": null,
        }));
        assert!(rendered.contains("max_connections = 16"));
        assert!(rendered.contains("download_dir = /home/u/Downloads"));
        assert!(rendered.contains("global_speed_limit_bps = null"));
    }
}
