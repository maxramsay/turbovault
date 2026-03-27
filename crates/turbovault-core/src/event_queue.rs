//! Disk-backed local event queue for NATS resilience.
//!
//! When NATS is unavailable, events are appended to a newline-delimited JSON
//! file on disk. When connectivity is restored the queue is drained and
//! replayed in order.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::events::VaultEvent;

/// A file-backed FIFO queue of [`VaultEvent`]s stored as newline-delimited JSON.
pub struct LocalEventQueue {
    path: PathBuf,
    lock: Mutex<()>,
}

impl LocalEventQueue {
    /// Create a new queue backed by `{data_dir}/event_queue.jsonl`.
    ///
    /// The parent directory is created if it does not exist.
    pub fn new(data_dir: &Path) -> Self {
        fs::create_dir_all(data_dir).expect("failed to create event queue directory");
        Self {
            path: data_dir.join("event_queue.jsonl"),
            lock: Mutex::new(()),
        }
    }

    /// Append an event to the queue.
    pub fn enqueue(&self, event: &VaultEvent) {
        let _guard = self.lock.lock().expect("queue lock poisoned");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .expect("failed to open event queue file");
        let mut line = serde_json::to_string(event).expect("failed to serialize event");
        line.push('\n');
        file.write_all(line.as_bytes())
            .expect("failed to write event to queue");
    }

    /// Drain all pending events, returning them in FIFO order, and truncate
    /// the queue file.
    pub fn drain(&self) -> Vec<VaultEvent> {
        let _guard = self.lock.lock().expect("queue lock poisoned");

        let events = if self.path.exists() {
            let file = fs::File::open(&self.path).expect("failed to open queue for drain");
            let reader = BufReader::new(file);
            reader
                .lines()
                .filter_map(|line| {
                    let line = line.expect("failed to read queue line");
                    if line.trim().is_empty() {
                        return None;
                    }
                    Some(serde_json::from_str(&line).expect("failed to deserialize queued event"))
                })
                .collect()
        } else {
            Vec::new()
        };

        // Truncate the file (or create empty) after successful read.
        if self.path.exists() {
            fs::write(&self.path, b"").expect("failed to truncate queue file");
        }

        events
    }

    /// Returns `true` if there are pending events in the queue.
    pub fn has_pending(&self) -> bool {
        let _guard = self.lock.lock().expect("queue lock poisoned");
        if !self.path.exists() {
            return false;
        }
        let metadata = fs::metadata(&self.path).unwrap_or_else(|_| {
            panic!("failed to stat queue file");
        });
        metadata.len() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NoteCreatedEvent;
    use tempfile::TempDir;

    fn make_event(path: &str) -> VaultEvent {
        VaultEvent::new(
            "vault.note.created",
            "test-org",
            &NoteCreatedEvent {
                path: path.to_string(),
                size_bytes: 100,
            },
        )
    }

    #[test]
    fn enqueue_and_drain() {
        let tmp = TempDir::new().unwrap();
        let q = LocalEventQueue::new(tmp.path());

        let evt = make_event("a.md");
        q.enqueue(&evt);

        assert!(q.has_pending());

        let drained = q.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].event_type, "vault.note.created");
        assert!(!q.has_pending());
    }

    #[test]
    fn multiple_enqueue_preserves_order() {
        let tmp = TempDir::new().unwrap();
        let q = LocalEventQueue::new(tmp.path());

        q.enqueue(&make_event("first.md"));
        q.enqueue(&make_event("second.md"));
        q.enqueue(&make_event("third.md"));

        let drained = q.drain();
        assert_eq!(drained.len(), 3);

        let paths: Vec<String> = drained
            .iter()
            .map(|e| {
                let data: NoteCreatedEvent = serde_json::from_value(e.data.clone()).unwrap();
                data.path
            })
            .collect();
        assert_eq!(paths, vec!["first.md", "second.md", "third.md"]);
    }

    #[test]
    fn drain_empty_queue() {
        let tmp = TempDir::new().unwrap();
        let q = LocalEventQueue::new(tmp.path());

        assert!(!q.has_pending());
        let drained = q.drain();
        assert!(drained.is_empty());
    }
}
