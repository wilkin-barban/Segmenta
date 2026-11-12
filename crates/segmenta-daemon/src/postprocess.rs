//! Safe post-processing hook: argv only, no shell, bounded time and bounded output.
//!
//! Contract: OpenSpec 12 §4, decisions of Fase 4 of `segmenta-0.2.md`.
//!
//! # Invocation
//!
//! `postprocess_command` is an argv vector, never a command line. The binary is executed
//! with [`std::process::Command`] and an explicit argument vector: no `sh -c` is used and
//! therefore `$(...)`, backticks, `;`, `|` or `*` inside an argument or inside a file name
//! are ordinary bytes. `stdin` is `/dev/null` so a hook cannot consume the daemon's input.
//!
//! # Placeholders
//!
//! Only three placeholders are expanded, and each one is expanded as a whole argument
//! value, never re-split: `%file%` (absolute path of the downloaded file), `%folder%`
//! (its directory) and `%name%` (its file name). Expansion is a single left-to-right pass,
//! so a file whose own name contains `%name%` cannot be expanded twice.
//!
//! A `%...%` token that is not one of the three stays **literal**, which is what the task
//! contract of A1 asks for: the hook receives the bytes the user wrote instead of an empty
//! string. OpenSpec 12 §4 words this differently ("an unknown placeholder leaves the list
//! invalid and the hook does not run"); the divergence is deliberate, documented here and
//! reported to the owner of the document, because silently dropping an argument the user
//! typed is the more surprising of the two behaviours.
//!
//! # Bounds
//!
//! * Time: [`DEFAULT_TIMEOUT_SECS`] seconds. There is no `postprocess_timeout_secs` in the
//!   settings catalog of 0.2 (`store::SETTING_KEYS`, OpenSpec 07), so the hook uses this
//!   documented constant until such a key exists; a hook that exceeds it is killed and the
//!   kill is recorded.
//! * Output: [`DEFAULT_CAPTURE_LIMIT`] bytes per stream. Both pipes are drained by a reader
//!   thread that keeps only the first bytes up to the limit and discards the rest, so a
//!   hook that prints gigabytes is neither blocked nor able to grow the daemon.
//! * Message: [`MESSAGE_LIMIT`] characters, which is what the task record stores.
//!
//! # Exit code
//!
//! Decision of Fase 4 / OpenSpec 12 §4: a non-zero exit **does not** turn a successful
//! download into a failed one unless the user configures that. The hook outcome is always
//! recorded (`postprocess_exit`, `postprocess_message`) and shown as a warning; only
//! [`HookRun::fails_task`] with `fail_on_error = true` turns it into a task failure. No
//! setting of 0.2 enables that yet, so the manager passes `false`: the failure branch is
//! implemented and tested, not wired to a key that the settings catalog does not have.
//!
//! The hook never touches the downloaded file: the outcome is a record, and a failed hook
//! leaves the bytes in place (asserted by the integration test).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Fallback timeout of the hook, in seconds.
///
/// OpenSpec 12 §4 asks for "timeout configurable"; the 0.2 settings catalog has no key for
/// it, so the constant is the whole story today: long enough for a converter or a renamer
/// on a multi-gigabyte file, short enough that a stuck hook cannot block a queue slot for
/// minutes. When `postprocess_timeout_secs` lands, [`HookOptions::timeout`] is the field to
/// feed.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Bytes kept from each captured stream.
pub const DEFAULT_CAPTURE_LIMIT: usize = 8 * 1024;

/// Characters kept from the recorded message.
pub const MESSAGE_LIMIT: usize = 512;

/// Label used in the recorded messages of the post-processing hook.
const HOOK_LABEL: &str = "hook";

/// Longest sleep between two exit polls, so a hook that ends instantly is noticed fast and
/// a hook that runs for the whole timeout does not burn the CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// How long the reader threads get to hand over what they captured once the child is gone.
const CAPTURE_GRACE: Duration = Duration::from_millis(250);

/// The file the hook acts on, plus the two derived placeholders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookTarget {
    /// Absolute path of the downloaded file.
    pub file: PathBuf,
    /// Directory that holds it.
    pub folder: PathBuf,
    /// Its file name.
    pub name: String,
}

impl HookTarget {
    /// Derive the three values from the final path of a task.
    pub fn from_file(file: &Path) -> HookTarget {
        let file = file.to_path_buf();
        let folder = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        HookTarget { file, folder, name }
    }
}

/// How the hook process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookExit {
    /// The process ran to completion with this exit code.
    Code(i32),
    /// The process was killed by a signal.
    Signal(i32),
    /// The deadline expired and the process was killed.
    Timeout,
    /// The process was never started: missing binary, no permission, invalid argv.
    Spawn {
        /// Binary the daemon tried to execute.
        binary: String,
        /// Operating-system error.
        error: String,
    },
}

impl HookExit {
    /// Exit code, when the process produced one.
    pub fn code(&self) -> Option<i32> {
        match self {
            HookExit::Code(code) => Some(*code),
            _ => None,
        }
    }

    /// Whether the hook succeeded.
    pub fn succeeded(&self) -> bool {
        matches!(self, HookExit::Code(0))
    }
}

/// Everything one hook run leaves behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutcome {
    /// How the process ended.
    pub exit: HookExit,
    /// Bounded, human-readable record: the reason plus the tail of the output.
    pub message: String,
    /// Bounded captured stdout.
    pub stdout: String,
    /// Bounded captured stderr.
    pub stderr: String,
}

impl HookOutcome {
    /// Outcome for a run the daemon itself could not execute.
    pub fn not_run(binary: &str, error: &str) -> HookOutcome {
        HookOutcome {
            exit: HookExit::Spawn {
                binary: binary.to_string(),
                error: error.to_string(),
            },
            message: truncate(
                &format!("cannot run hook `{binary}`: {error}"),
                MESSAGE_LIMIT,
            ),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn new(
        label: &str,
        exit: HookExit,
        stdout: String,
        stderr: String,
        timeout: Duration,
    ) -> HookOutcome {
        let message = summarize(label, &exit, &stdout, &stderr, timeout);
        HookOutcome {
            exit,
            message,
            stdout,
            stderr,
        }
    }
}

/// Result of the post-processing phase of one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookRun {
    /// No hook configured: the phase is skipped and the task keeps its normal flow.
    Skipped,
    /// A hook was configured and left a record.
    Ran(HookOutcome),
}

impl HookRun {
    /// Whether this run must fail the task.
    ///
    /// `fail_on_error` is the user's opt-in; with it off, every outcome — non-zero exit,
    /// timeout, missing binary — is a recorded warning, never a failed download.
    pub fn fails_task(&self, fail_on_error: bool) -> bool {
        match self {
            HookRun::Skipped => false,
            HookRun::Ran(outcome) => fail_on_error && !outcome.exit.succeeded(),
        }
    }

    /// The outcome, when a hook actually ran.
    pub fn outcome(&self) -> Option<&HookOutcome> {
        match self {
            HookRun::Skipped => None,
            HookRun::Ran(outcome) => Some(outcome),
        }
    }
}

/// Bounds of one hook run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOptions {
    /// Deadline of the whole process.
    pub timeout: Duration,
    /// Bytes kept from each stream.
    pub capture_limit: usize,
    /// Whether a non-zero exit fails the task (off by default, OpenSpec 12 §4).
    pub fail_on_error: bool,
}

impl Default for HookOptions {
    fn default() -> HookOptions {
        HookOptions {
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            capture_limit: DEFAULT_CAPTURE_LIMIT,
            fail_on_error: false,
        }
    }
}

/// Expand the three documented placeholders in one argument.
///
/// Single pass, so an expanded value is never expanded again. Unknown `%...%` tokens are
/// copied verbatim.
pub fn expand_argument(argument: &str, target: &HookTarget) -> String {
    let mut expanded = String::with_capacity(argument.len());
    let mut rest = argument;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                match &after[..end] {
                    "file" => expanded.push_str(&target.file.to_string_lossy()),
                    "folder" => expanded.push_str(&target.folder.to_string_lossy()),
                    "name" => expanded.push_str(&target.name),
                    // Not a documented placeholder: keep the user's bytes.
                    unknown => {
                        expanded.push('%');
                        expanded.push_str(unknown);
                        expanded.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                // A lone `%`: nothing to close the token, so the rest is literal.
                expanded.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    expanded.push_str(rest);
    expanded
}

/// Expand every argument of the command, keeping the argument boundaries intact.
pub fn build_argv(command: &[String], target: &HookTarget) -> Vec<String> {
    command
        .iter()
        .map(|argument| expand_argument(argument, target))
        .collect()
}

/// Run the configured hook, if any.
pub fn run(command: &[String], target: &HookTarget, options: &HookOptions) -> HookRun {
    if command.is_empty() {
        return HookRun::Skipped;
    }
    HookRun::Ran(execute(command, target, options))
}

fn execute(command: &[String], target: &HookTarget, options: &HookOptions) -> HookOutcome {
    let argv = build_argv(command, target);
    run_process(&argv, options.timeout, options.capture_limit, HOOK_LABEL)
}

/// Run one argv vector with no shell, a deadline and bounded capture.
///
/// Shared by the post-processing hook and the antivirus scan: both execute a binary the
/// user does not control the contents of, so both get the same guarantees — argument
/// boundaries preserved, no `sh -c`, `stdin` closed, kill on timeout and a bounded record
/// of the output.
fn run_process(
    argv: &[String],
    timeout: Duration,
    capture_limit: usize,
    label: &str,
) -> HookOutcome {
    let Some(binary) = argv.first() else {
        return HookOutcome::new(
            label,
            HookExit::Spawn {
                binary: String::new(),
                error: "empty command".to_string(),
            },
            String::new(),
            String::new(),
            timeout,
        );
    };
    if binary.is_empty() {
        return HookOutcome::new(
            label,
            HookExit::Spawn {
                binary: String::new(),
                error: "empty arguments[0]".to_string(),
            },
            String::new(),
            String::new(),
            timeout,
        );
    }
    let mut child = match Command::new(binary)
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return HookOutcome::new(
                label,
                HookExit::Spawn {
                    binary: binary.clone(),
                    error: err.to_string(),
                },
                String::new(),
                String::new(),
                timeout,
            )
        }
    };

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_rx = stdout_pipe.map(|pipe| capture(pipe, capture_limit));
    let stderr_rx = stderr_pipe.map(|pipe| capture(pipe, capture_limit));

    let deadline = Instant::now() + timeout;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => break exit_of(&status),
            Ok(None) => {}
            Err(err) => {
                // The child may be in an unknown state: kill it so nothing is left behind.
                let _ = child.kill();
                let _ = child.wait();
                break HookExit::Spawn {
                    binary: binary.clone(),
                    error: format!("cannot wait for {label}: {err}"),
                };
            }
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break HookExit::Timeout;
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline - now));
    };

    let stdout = stdout_rx.map(drain).unwrap_or_default();
    let stderr = stderr_rx.map(drain).unwrap_or_default();
    HookOutcome::new(label, exit, stdout, stderr, timeout)
}

fn exit_of(status: &std::process::ExitStatus) -> HookExit {
    use std::os::unix::process::ExitStatusExt;
    match status.code() {
        Some(code) => HookExit::Code(code),
        None => HookExit::Signal(status.signal().unwrap_or(0)),
    }
}

/// Spawn the reader of one pipe and hand the bounded bytes back through a channel, so the
/// caller can abandon a reader that never sees EOF instead of blocking the daemon forever.
fn capture<R: Read + Send + 'static>(mut pipe: R, limit: usize) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut chunk = [0u8; 8 * 1024];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    // Keep draining even after the limit: a full pipe would block the hook.
                    let room = limit.saturating_sub(kept.len());
                    if room > 0 {
                        kept.extend_from_slice(&chunk[..read.min(room)]);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = tx.send(kept);
    });
    rx
}

fn drain(rx: Receiver<Vec<u8>>) -> String {
    match rx.recv_timeout(CAPTURE_GRACE) {
        Ok(bytes) => truncate(&String::from_utf8_lossy(&bytes), DEFAULT_CAPTURE_LIMIT),
        // A grandchild may still hold the pipe; the daemon does not wait for it.
        Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => String::new(),
    }
}

fn summarize(
    label: &str,
    exit: &HookExit,
    stdout: &str,
    stderr: &str,
    timeout: Duration,
) -> String {
    let head = match exit {
        HookExit::Code(0) => format!("{label} exited with code 0"),
        HookExit::Code(code) => format!("{label} exited with code {code}"),
        HookExit::Signal(signal) => format!("{label} was killed by signal {signal}"),
        HookExit::Timeout => format!(
            "{label} exceeded its timeout of {:.1}s and was killed",
            timeout.as_secs_f64()
        ),
        HookExit::Spawn { binary, error } => {
            format!("cannot run {label} `{binary}`: {error}")
        }
    };
    match last_line(stderr).or_else(|| last_line(stdout)) {
        Some(detail) => truncate(&format!("{head}: {detail}"), MESSAGE_LIMIT),
        None => truncate(&head, MESSAGE_LIMIT),
    }
}

fn last_line(stream: &str) -> Option<String> {
    stream
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

// ---------------------------------------------------------------------------
// Antivirus (OpenSpec 12 §5)
// ---------------------------------------------------------------------------

/// `ERR_ANTIVIRUS_FOUND`: the scan found an infection and the file is in quarantine.
pub const ERR_ANTIVIRUS_FOUND: i64 = 1046;

/// `ERR_ANTIVIRUS_UNAVAILABLE`: antivirus is enabled and no scan could produce a verdict.
///
/// This covers both documented causes: neither `clamdscan` nor `clamscan` is usable, and a
/// scan that timed out or failed without reporting an infection. Declaring such a file
/// `completed` would tell the user it was checked when it was not (OpenSpec 11 §decisiones).
pub const ERR_ANTIVIRUS_UNAVAILABLE: i64 = 1047;

/// Name of [`ERR_ANTIVIRUS_FOUND`], prefixed to the recorded message.
pub const ERR_ANTIVIRUS_FOUND_NAME: &str = "ERR_ANTIVIRUS_FOUND";

/// Name of [`ERR_ANTIVIRUS_UNAVAILABLE`], prefixed to the recorded message.
pub const ERR_ANTIVIRUS_UNAVAILABLE_NAME: &str = "ERR_ANTIVIRUS_UNAVAILABLE";

/// Environment override of the engine search path.
///
/// The same idea as `SEGMENTA_YT_DLP` in `media`: the tests point it at a tempdir with fake
/// engines, so no test needs the real ClamAV, no test inherits one from the host and no
/// test rewrites the process `PATH`.
pub const ANTIVIRUS_PATH_ENV: &str = "SEGMENTA_ANTIVIRUS_PATH";

/// Exit code ClamAV uses for "infected file found".
pub const INFECTION_EXIT_CODE: i32 = 1;

/// Label used in the recorded messages of a scan.
const SCANNER_LABEL: &str = "antivirus scanner";

/// Which ClamAV front end the daemon will run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanEngine {
    /// `clamdscan`: talks to a running `clamd` over its socket. Preferred.
    Clamdscan,
    /// `clamscan`: standalone scanner. The alternative when there is no daemon.
    Clamscan,
}

impl ScanEngine {
    /// Binary name of the engine.
    pub fn name(self) -> &'static str {
        match self {
            ScanEngine::Clamdscan => "clamdscan",
            ScanEngine::Clamscan => "clamscan",
        }
    }
}

/// A usable engine: which one it is and where it lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    /// The front end that will be executed.
    pub kind: ScanEngine,
    /// Absolute path of its binary.
    pub binary: PathBuf,
}

/// What one scan of one file means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanVerdict {
    /// The scanner produced a verdict and the file is clean.
    Clean,
    /// The scanner reported an infection: the file must be quarantined.
    Infected,
    /// The scanner ran but produced no verdict (timeout, error exit, crash).
    Failed,
    /// Antivirus is enabled and there is no usable engine.
    Unavailable,
}

/// Everything one scan leaves behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    /// Engine that ran, `None` when none was available.
    pub engine: Option<Engine>,
    /// Exit code of the scanner; `None` when it timed out, was killed or never started.
    pub exit_code: Option<i32>,
    /// Bounded, human-readable record; prefixed with the `ERR_ANTIVIRUS_*` name.
    pub message: String,
    /// Bounded captured output, kept for diagnostics.
    pub output: String,
}

impl ScanReport {
    /// What the daemon must do with the file.
    pub fn verdict(&self) -> ScanVerdict {
        match (self.engine.is_some(), self.exit_code) {
            (false, _) => ScanVerdict::Unavailable,
            (true, Some(0)) => ScanVerdict::Clean,
            (true, Some(INFECTION_EXIT_CODE)) => ScanVerdict::Infected,
            (true, _) => ScanVerdict::Failed,
        }
    }
}

/// Bounds of one scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    /// Deadline of the whole process.
    pub timeout: Duration,
    /// Bytes kept from each stream.
    pub capture_limit: usize,
}

impl Default for ScanOptions {
    fn default() -> ScanOptions {
        ScanOptions {
            timeout: Duration::from_secs(crate::store::DEFAULT_ANTIVIRUS_TIMEOUT_SECS),
            capture_limit: DEFAULT_CAPTURE_LIMIT,
        }
    }
}

/// Directories searched for an engine.
///
/// [`ANTIVIRUS_PATH_ENV`] when it is set and non-empty, `PATH` otherwise. Same shape as
/// `media::locate_tool`, which is what lets the tests use a fake engine.
pub fn engine_search_path() -> Vec<PathBuf> {
    let raw = match std::env::var_os(ANTIVIRUS_PATH_ENV) {
        Some(value) if !value.is_empty() => value,
        _ => std::env::var_os("PATH").unwrap_or_default(),
    };
    std::env::split_paths(&raw).collect()
}

/// Detect the engine in `dirs`: `clamdscan` first, `clamscan` after (OpenSpec 12 §5).
pub fn detect_engine_in(dirs: &[PathBuf]) -> Option<Engine> {
    for kind in [ScanEngine::Clamdscan, ScanEngine::Clamscan] {
        for dir in dirs {
            let binary = dir.join(kind.name());
            if is_executable(&binary) {
                return Some(Engine { kind, binary });
            }
        }
    }
    None
}

/// Detect the engine on this machine.
pub fn detect_engine() -> Option<Engine> {
    detect_engine_in(&engine_search_path())
}

/// Scan `file` with the engine detected now.
pub fn scan(file: &Path, options: &ScanOptions) -> ScanReport {
    scan_with(detect_engine(), file, options)
}

/// Scan `file` with a caller-chosen engine, `None` meaning "no engine available".
///
/// The scanner is invoked as `[binary, "--no-summary", file]`: an argv vector, never a
/// string a shell could re-parse, with the same deadline and bounded capture as the hook.
/// Exit code 0 is clean and 1 is an infection; every other ending — other code, signal,
/// timeout, spawn failure — is [`ScanVerdict::Failed`] and never deletes anything.
pub fn scan_with(engine: Option<Engine>, file: &Path, options: &ScanOptions) -> ScanReport {
    let Some(engine) = engine else {
        return ScanReport {
            engine: None,
            exit_code: None,
            message: truncate(
                &format!(
                    "{ERR_ANTIVIRUS_UNAVAILABLE_NAME}: neither clamdscan nor clamscan is available"
                ),
                MESSAGE_LIMIT,
            ),
            output: String::new(),
        };
    };
    let argv = vec![
        engine.binary.to_string_lossy().into_owned(),
        "--no-summary".to_string(),
        file.to_string_lossy().into_owned(),
    ];
    let outcome = run_process(&argv, options.timeout, options.capture_limit, SCANNER_LABEL);
    let exit_code = outcome.exit.code();
    let detail = last_line(&outcome.stdout)
        .or_else(|| last_line(&outcome.stderr))
        .unwrap_or_else(|| outcome.message.clone());
    let message = match exit_code {
        Some(0) => format!("{} found no infection", engine.kind.name()),
        Some(INFECTION_EXIT_CODE) => format!("{ERR_ANTIVIRUS_FOUND_NAME}: {detail}"),
        _ => format!("{ERR_ANTIVIRUS_UNAVAILABLE_NAME}: {}", outcome.message),
    };
    ScanReport {
        engine: Some(engine),
        exit_code,
        message: truncate(&message, MESSAGE_LIMIT),
        output: outcome.stderr,
    }
}

/// Move an infected file into `dir`, keeping its name and never overwriting anything.
///
/// Same filesystem: one `rename`. Different filesystem: a copy that is verified by length
/// and renamed into place before the original is removed, so a partial copy can never be
/// mistaken for the quarantined file. The result is not executable and is only readable by
/// its owner: a quarantined binary is evidence, not something to run.
pub fn quarantine(file: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let _ = set_mode(dir, 0o700);
    let name = file
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name"))?;
    let mut target = dir.join(name);
    let mut attempt = 1;
    while target.exists() {
        let mut candidate = name.to_os_string();
        candidate.push(format!(".{attempt}"));
        target = dir.join(candidate);
        attempt += 1;
    }
    match std::fs::rename(file, &target) {
        Ok(()) => {
            let _ = set_mode(&target, 0o600);
            Ok(target)
        }
        Err(err) if err.raw_os_error() == Some(libc::EXDEV) => {
            let mut staging = target.as_os_str().to_os_string();
            staging.push(".staging");
            let staging = PathBuf::from(staging);
            std::fs::copy(file, &staging)?;
            let source_len = std::fs::metadata(file)?.len();
            let copied_len = std::fs::metadata(&staging)?.len();
            if source_len != copied_len {
                let _ = std::fs::remove_file(&staging);
                return Err(std::io::Error::other(format!(
                    "quarantine copy is incomplete: {copied_len} of {source_len} bytes"
                )));
            }
            std::fs::rename(&staging, &target)?;
            std::fs::remove_file(file)?;
            let _ = set_mode(&target, 0o600);
            Ok(target)
        }
        Err(err) => Err(err),
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}
