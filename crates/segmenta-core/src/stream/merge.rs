//! Merge planning: the `ffconcat` file and the separated `ffmpeg` arguments.
//!
//! The plan is text and vectors only. `ffmpeg` runs with `-f concat -safe 0` and
//! `-c copy`; the binary receives one argument per value, so a path can never become
//! a shell word.

use std::path::{Path, PathBuf};

/// Header required by the `concat` demuxer.
const CONCAT_HEADER: &str = "ffconcat version 1.0";

/// Render one `file` directive, escaping `'` as `'\''` so the value cannot end early.
fn file_line(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "'\\''");
    format!("file '{escaped}'")
}

fn render<'a>(paths: impl Iterator<Item = &'a Path>) -> String {
    let mut out = String::from(CONCAT_HEADER);
    out.push('\n');
    for path in paths {
        out.push_str(&file_line(path));
        out.push('\n');
    }
    out
}

/// Build the `ffconcat` document for the fragments, in order.
pub fn ffconcat(segments: &[PathBuf]) -> String {
    render(segments.iter().map(PathBuf::as_path))
}

/// Build the `ffconcat` document with the initialization segment first.
pub fn ffconcat_with_init(init: Option<&Path>, fragments: &[PathBuf]) -> String {
    render(
        init.into_iter()
            .chain(fragments.iter().map(PathBuf::as_path)),
    )
}

/// ffmpeg arguments for the concat merge, optionally muxing one separate audio track.
///
/// Every value travels as its own argument: the binary is never started through a
/// shell, so spaces, quotes and `$()` in a path stay literal.
pub fn ffmpeg_args(concat_path: &Path, output: &Path, audio: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
    ]
    .iter()
    .map(|argument| (*argument).to_string())
    .collect();
    args.push("-i".to_string());
    args.push(concat_path.to_string_lossy().into_owned());
    args.push("-c".to_string());
    args.push("copy".to_string());
    if let Some(audio) = audio {
        args.push("-i".to_string());
        args.push(audio.to_string_lossy().into_owned());
        args.push("-map".to_string());
        args.push("0:v:0".to_string());
        args.push("-map".to_string());
        args.push("1:a:0".to_string());
        args.push("-c:a".to_string());
        args.push("copy".to_string());
    }
    args.push(output.to_string_lossy().into_owned());
    args
}
