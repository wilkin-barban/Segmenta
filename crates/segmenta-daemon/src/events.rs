//! Event bus: one monotonic `seq_id` per daemon and a bounded broadcast channel.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::sync::broadcast;

/// Event bus shared by the manager and every IPC connection.
pub struct EventBus {
    seq: AtomicU64,
    tx: broadcast::Sender<crate::rpc::EventEnvelope>,
}

impl EventBus {
    /// Create a bus with the 1024-event buffer of OpenSpec 07.
    pub fn new() -> EventBus {
        let (tx, _) = broadcast::channel(crate::ipc::EVENT_BUFFER);
        EventBus {
            seq: AtomicU64::new(0),
            tx,
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

    /// Publish one event with the next sequence number.
    pub fn publish(&self, event_type: &str, task_id: Option<&str>, data: Value) {
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
