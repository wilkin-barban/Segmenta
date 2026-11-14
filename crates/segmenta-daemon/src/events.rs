//! Event bus: one monotonic `seq_id` per daemon and a bounded broadcast channel.
//!
//! The bus also owns the tombstones of the tasks a housekeeping call is removing, so the
//! gate that keeps a removed task quiet lives next to the only function that publishes.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::Value;
use tokio::sync::broadcast;

/// Event bus shared by the manager and every IPC connection.
pub struct EventBus {
    seq: AtomicU64,
    tx: broadcast::Sender<crate::rpc::EventEnvelope>,
    /// Ids a housekeeping call is removing or has already removed.
    tombstones: Mutex<HashSet<String>>,
}

impl EventBus {
    /// Create a bus with the 1024-event buffer of OpenSpec 07.
    pub fn new() -> EventBus {
        let (tx, _) = broadcast::channel(crate::ipc::EVENT_BUFFER);
        EventBus {
            seq: AtomicU64::new(0),
            tx,
            tombstones: Mutex::new(HashSet::new()),
        }
    }

    /// Subscribe a client.
    pub fn subscribe(&self) -> broadcast::Receiver<crate::rpc::EventEnvelope> {
        self.tx.subscribe()
    }

    /// Number of live subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Silence every event about `task_id` until [`EventBus::untombstone`].
    ///
    /// A batch delete stops a worker and then removes its row and its file. Whatever the
    /// stopped worker still had in flight describes a task the caller already erased, so
    /// the bus drops it instead of publishing a transition about a row that no longer
    /// exists. The manager sets the mark while it holds the lock that owns the in-flight
    /// set, which is the same lock `Manager::start_task` reads before it spawns a worker:
    /// no publisher and no new writer can slip in between the stop and the deletion.
    pub fn tombstone(&self, task_id: &str) {
        self.tombstones
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(task_id.to_string());
    }

    /// Lift a tombstone, because the removal it guarded did not happen.
    pub fn untombstone(&self, task_id: &str) -> bool {
        self.tombstones
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(task_id)
    }

    /// Whether this task's events are currently dropped.
    pub fn is_tombstoned(&self, task_id: &str) -> bool {
        self.tombstones
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .contains(task_id)
    }

    /// Publish one event with the next sequence number.
    ///
    /// An event about a tombstoned task is dropped before it consumes a sequence number:
    /// the stream stays dense and no client ever sees a task announce anything after the
    /// call that removed it returned.
    pub fn publish(&self, event_type: &str, task_id: Option<&str>, data: Value) {
        if let Some(id) = task_id {
            if self.is_tombstoned(id) {
                tracing::debug!(
                    task_id = %id,
                    event_type,
                    "event dropped: the task is being removed"
                );
                return;
            }
        }
        let seq_id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let envelope = crate::rpc::EventEnvelope {
            seq_id,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0),
            event_type: event_type.to_string(),
            task_id: task_id.map(str::to_string),
            data,
        };
        let _ = self.tx.send(envelope);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        EventBus::new()
    }
}
