//! Human-readable rendering for the CLI: sizes, speeds, ETAs, tables and segment bars.

use segmenta_daemon::rpc::DownloadView;

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

/// Render the `list` table.
pub fn render_table(rows: &[DownloadView]) -> String {
    if rows.is_empty() {
        return "No downloads.\n".to_string();
    }
    let header = format!(
        "{:<8}  {:<11}  {:>6}  {:>10}  {:>10}  {:>8}  {}",
        "ID", "STATUS", "PROG", "SIZE", "SPEED", "ETA", "NAME"
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
            view.status,
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
pub fn render_status(view: &DownloadView) -> String {
    let mut out = String::new();
    out.push_str(&format!("task:      {}\n", view.task_id));
    out.push_str(&format!("url:       {}\n", view.url));
    out.push_str(&format!("file:      {}\n", view.final_path));
    out.push_str(&format!("status:    {}\n", view.status));
    out.push_str(&format!("progress:  {} of ", format_bytes(view.downloaded_bytes)));
    out.push_str(
        &view
            .total_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "unknown".to_string()),
    );
    out.push('\n');
    out.push_str(&format!(
        "speed:     {}   eta: {}\n",
        format_speed(view.speed_bps),
        format_eta(view.eta_seconds)
    ));
    out.push_str(&format!(
        "ranges:    {}   queue: {}   retries: {}\n",
        if view.supports_ranges { "yes" } else { "no" },
        view.queue_name,
        view.retry_count
    ));
    if let Some(message) = &view.error_message {
        out.push_str(&format!(
            "error:     {} ({})\n",
            message,
            view.error_code.unwrap_or(0)
        ));
    }
    if !view.segments.is_empty() {
        out.push_str("segments:  ");
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
pub fn render_queues(queues: &serde_json::Value) -> String {
    let Some(rows) = queues.as_array() else {
        return format!("{queues}\n");
    };
    if rows.is_empty() {
        return "No queues.\n".to_string();
    }
    let mut out = format!("{:<16} {:>6} {:>7} {:>7} {:>7}\n", "NAME", "SLOTS", "ACTIVE", "PENDING", "PAUSED");
    for row in rows {
        out.push_str(&format!(
            "{:<16} {:>6} {:>7} {:>7} {:>7}\n",
            row["name"].as_str().unwrap_or("-"),
            row["max_active"].as_u64().unwrap_or(0),
            row["active"].as_u64().unwrap_or(0),
            row["pending"].as_u64().unwrap_or(0),
            row["paused"].as_u64().unwrap_or(0),
        ));
    }
    out
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
        assert_eq!(render_table(&[]), "No downloads.\n");
        let table = render_table(&[view()]);
        assert!(table.starts_with("ID "));
        assert!(table.contains("c1f7a28e"));
        assert!(table.contains("downloading"));
        assert!(table.contains("ubuntu.iso"));
        assert!(table.contains("10.0 MiB"));
    }

    #[test]
    fn status_renders_segments_and_errors() {
        let mut download = view();
        let text = render_status(&download);
        assert!(text.contains("task:      c1f7a28e-8b1b-4f9e-9d22-29f123456789"));
        assert!(text.contains("segments:  [####....]"));
        download.error_message = Some("disk full".to_string());
        download.error_code = Some(1006);
        assert!(render_status(&download).contains("disk full (1006)"));
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
