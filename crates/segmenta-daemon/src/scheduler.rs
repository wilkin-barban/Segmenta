//! Native queue scheduler: the single loop that decides which queues may run right now.
//!
//! OpenSpec 12 §2 is the contract. Every tick reads the persisted schedule of each queue
//! (`queue_schedule:<name>`, plus the legacy `schedule_start`/`schedule_stop` fields for
//! `main`), asks [`Manager::start_pending_for_queue`] to fill the free slots and runs the
//! `action_on_complete` of a queue that just drained.
//!
//! **Time source.** Local wall-clock time is read through `libc::localtime_r` instead of
//! adding `chrono`: the daemon already links `libc`, one call yields `tm_hour`/`tm_min`,
//! and a window is two integers compared against the minute of the day. A calendar crate
//! would add compile time and a dependency for arithmetic this module does not need.
//!
//! Nothing here sleeps in wall-clock time during a test: [`Clock`] and [`HostActions`]
//! are injected, and `QueueScheduler::tick` can be called directly.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{watch, Notify};

use crate::manager::Manager;
use crate::store::{parse_hh_mm, Settings};

/// Production tick: how often every queue is re-evaluated (OpenSpec 12 §2).
pub const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// Minutes in a day: a window is evaluated in local wall-clock minutes.
pub const MINUTES_PER_DAY: u32 = 24 * 60;

/// Injectable wall clock, so a test never depends on the host time or sleeps.
pub trait Clock: Send + Sync {
    /// Local wall-clock minute of the day, `0..MINUTES_PER_DAY`.
    fn minute_of_day(&self) -> u32;
}

/// Production clock: local time as the user sees it.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalClock;

impl Clock for LocalClock {
    fn minute_of_day(&self) -> u32 {
        // SAFETY: `time` and `localtime_r` only touch the pointers they are given; `tm`
        // is fully initialized before the call and `localtime_r` cannot fail for a valid
        // `time_t`. `tm_hour`/`tm_min` are in range by definition, and the modulo only
        // guards against a libc that reports a leap second as 60.
        unsafe {
            let now = libc::time(std::ptr::null_mut());
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&now, &mut tm).is_null() {
                return 0;
            }
            ((tm.tm_hour as u32) * 60 + tm.tm_min as u32) % MINUTES_PER_DAY
        }
    }
}

/// A daily window in local wall-clock minutes: inclusive start, exclusive stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// First minute the queue may run.
    pub start: u32,
    /// First minute the queue is closed again, up to [`MINUTES_PER_DAY`].
    pub stop: u32,
}

impl Window {
    /// Whether the window covers `minute`.
    pub fn is_open_at(&self, minute: u32) -> bool {
        let minute = minute % MINUTES_PER_DAY;
        if self.start == self.stop {
            // A zero-length window is read as "no restriction": the user typed the same
            // time twice, and a queue that never opens is never what they asked for.
            return true;
        }
        if self.start < self.stop {
            minute >= self.start && minute < self.stop
        } else {
            // Crosses midnight: `22:00–06:00` is open from 22:00 to 23:59 and 00:00 to
            // 05:59, so a single window covers both sides of the day boundary.
            minute >= self.start || minute < self.stop
        }
    }
}

/// Build the window of a queue from its two optional `HH:MM` bounds.
///
/// `None` means "no window": the queue may run at any time. A missing start opens at
/// `00:00` and a missing stop closes at `24:00`. A value that is not `HH:MM` is an error:
/// the caller must not persist a schedule the scheduler cannot evaluate.
pub fn window_from_bounds(
    start: Option<&str>,
    stop: Option<&str>,
) -> Result<Option<Window>, String> {
    if start.is_none() && stop.is_none() {
        return Ok(None);
    }
    let start_minute = match start {
        Some(value) => parse_hh_mm(value).ok_or_else(|| format!("`{value}` is not HH:MM"))?,
        None => 0,
    };
    let stop_minute = match stop {
        Some(value) => parse_hh_mm(value).ok_or_else(|| format!("`{value}` is not HH:MM"))?,
        None => MINUTES_PER_DAY,
    };
    Ok(Some(Window {
        start: start_minute,
        stop: stop_minute,
    }))
}

/// What to do once a queue has drained (`action_on_complete`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompleteAction {
    /// Do nothing.
    #[default]
    None,
    /// Suspend the machine.
    Suspend,
    /// Power the machine off.
    PowerOff,
    /// Stop the daemon through its existing shutdown channel.
    Exit,
}

impl CompleteAction {
    /// Parse the stored value; anything unknown is `none`, which is the documented
    /// default and keeps `queue.schedule` compatible with clients that send free text.
    pub fn parse(value: &str) -> CompleteAction {
        match value {
            "suspend" => CompleteAction::Suspend,
            "power_off" => CompleteAction::PowerOff,
            "exit" => CompleteAction::Exit,
            _ => CompleteAction::None,
        }
    }

    /// The literal persisted in settings and reported by the RPC layer.
    pub fn as_str(self) -> &'static str {
        match self {
            CompleteAction::None => "none",
            CompleteAction::Suspend => "suspend",
            CompleteAction::PowerOff => "power_off",
            CompleteAction::Exit => "exit",
        }
    }
}

/// Effective schedule of one queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueSchedule {
    /// Open window; `None` means the queue may run at any time.
    pub window: Option<Window>,
    /// Action run once when the queue drains.
    pub action: CompleteAction,
}

impl QueueSchedule {
    /// Whether the queue may start tasks at `minute`.
    pub fn is_open_at(&self, minute: u32) -> bool {
        self.window
            .map(|window| window.is_open_at(minute))
            .unwrap_or(true)
    }
}

/// Resolve the effective schedule of `queue`.
///
/// The persisted `queue_schedule:<name>` row wins; the `main` queue falls back to the
/// legacy `schedule_start`/`schedule_stop`/`action_on_complete` settings, one field at a
/// time, so a 0.1.x profile keeps working. A stored window the scheduler cannot evaluate
/// (only reachable for rows written before `queue.schedule` validated them) fails open
/// and is logged: a malformed value must never stall every download.
pub fn resolve_schedule(queue: &str, stored: Option<&Value>, settings: &Settings) -> QueueSchedule {
    let stored = stored.and_then(Value::as_object);
    let field = |name: &str| stored.and_then(|map| map.get(name)).and_then(Value::as_str);
    let legacy = queue == "main";

    let start = field("start_time").or(if legacy {
        settings.schedule_start.as_deref()
    } else {
        None
    });
    let stop = field("stop_time").or(if legacy {
        settings.schedule_stop.as_deref()
    } else {
        None
    });
    let action = field("action_on_complete")
        .map(CompleteAction::parse)
        .unwrap_or_else(|| {
            if legacy {
                CompleteAction::parse(&settings.action_on_complete)
            } else {
                CompleteAction::None
            }
        });

    let window = match window_from_bounds(start, stop) {
        Ok(window) => window,
        Err(err) => {
            tracing::warn!(queue = %queue, error = %err, "ignoring an invalid schedule window");
            None
        }
    };
    QueueSchedule { window, action }
}

/// System actions run when a queue drains, injectable so no test suspends or powers off.
pub trait HostActions: Send + Sync {
    /// Suspend the machine.
    fn suspend(&self) -> Result<(), String>;
    /// Power the machine off.
    fn power_off(&self) -> Result<(), String>;
}

/// Production implementation: the same `systemctl` verbs the user would run.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemActions;

impl HostActions for SystemActions {
    fn suspend(&self) -> Result<(), String> {
        systemctl("suspend")
    }

    fn power_off(&self) -> Result<(), String> {
        systemctl("poweroff")
    }
}

fn systemctl(verb: &str) -> Result<(), String> {
    let status = std::process::Command::new("systemctl")
        .arg(verb)
        .status()
        .map_err(|err| format!("cannot run systemctl {verb}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl {verb} exited with {status}"))
    }
}

/// The daemon's queue scheduler.
pub struct QueueScheduler {
    manager: Arc<Manager>,
    actions: Arc<dyn HostActions>,
    interval: Duration,
    wake: Arc<Notify>,
    /// Queues that held work since their last drain: the action runs once per drain and
    /// never on a queue that was already empty when the daemon started.
    armed: HashSet<String>,
}

impl QueueScheduler {
    /// Build a scheduler over an open manager.
    pub fn new(manager: Arc<Manager>, actions: Arc<dyn HostActions>, interval: Duration) -> Self {
        let wake = manager.scheduler_wake();
        QueueScheduler {
            manager,
            actions,
            interval,
            wake,
            armed: HashSet::new(),
        }
    }

    /// Run until the shutdown channel changes.
    ///
    /// A completion, failure or cancellation wakes the loop immediately; otherwise the
    /// next tick is `interval` away.
    pub async fn run(mut self, mut shutdown: watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            self.tick();
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(self.interval) => {}
            }
        }
        tracing::debug!("queue scheduler stopped");
    }

    /// Evaluate every queue once: start what fits, then run the action of a drained one.
    ///
    /// The tests call this directly, so no test ever waits for a tick.
    pub fn tick(&mut self) {
        let queues = match self.manager.queue_names() {
            Ok(queues) => queues,
            Err(err) => {
                tracing::warn!(error = %err, "scheduler cannot list the queues");
                return;
            }
        };
        for queue in queues {
            let schedule = self.manager.queue_schedule(&queue);
            if let Err(err) = self.manager.start_pending_for_queue(&queue) {
                tracing::warn!(queue = %queue, error = %err.message, "scheduler cannot start pending tasks");
            }
            // An error while counting is not evidence that the queue is empty: keep it
            // armed instead of running a system action on a guess.
            let has_work = self.manager.queue_has_work(&queue).unwrap_or(true);
            if has_work {
                self.armed.insert(queue);
                continue;
            }
            if !self.armed.remove(&queue) {
                continue;
            }
            self.run_action(&queue, schedule.action);
        }
    }

    fn run_action(&self, queue: &str, action: CompleteAction) {
        match action {
            CompleteAction::None => {}
            CompleteAction::Exit => {
                tracing::info!(queue = %queue, action = "exit", "queue drained");
                // The ordered shutdown the daemon already implements.
                self.manager.request_shutdown();
            }
            CompleteAction::Suspend => {
                tracing::info!(queue = %queue, action = "suspend", "queue drained");
                if let Err(err) = self.actions.suspend() {
                    tracing::warn!(queue = %queue, error = %err, "cannot suspend the machine");
                }
            }
            CompleteAction::PowerOff => {
                tracing::info!(queue = %queue, action = "power_off", "queue drained");
                if let Err(err) = self.actions.power_off() {
                    tracing::warn!(queue = %queue, error = %err, "cannot power the machine off");
                }
            }
        }
    }
}
