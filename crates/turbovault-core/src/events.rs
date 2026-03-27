//! CloudEvents-compatible event envelope and event data types.
//!
//! These types are wire-compatible with `FleetEvent` from `fc-events` in the
//! fleet-control repo. We define them locally to avoid cross-repo Cargo
//! dependencies — the shared contract is CloudEvents JSON on NATS subjects.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Event envelope
// ---------------------------------------------------------------------------

/// CloudEvents v1.0 envelope carrying vault activity data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEvent {
    /// CloudEvents specification version — always `"1.0"`.
    pub specversion: String,
    /// Unique event ID (UUID v4).
    pub id: String,
    /// Dot-delimited event type, e.g. `"vault.note.created"`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event source URI — always `"fc.vault"`.
    pub source: String,
    /// Timestamp of event creation.
    pub time: DateTime<Utc>,
    /// Organisation that owns this vault.
    pub org_id: String,
    /// Optional distributed-tracing ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Event payload (type-erased JSON).
    pub data: serde_json::Value,
}

impl VaultEvent {
    /// Create a new event with the given type, org, and serialisable payload.
    pub fn new(event_type: &str, org_id: &str, data: &impl Serialize) -> Self {
        Self {
            specversion: "1.0".to_string(),
            id: Uuid::new_v4().to_string(),
            event_type: event_type.to_string(),
            source: "fc.vault".to_string(),
            time: Utc::now(),
            org_id: org_id.to_string(),
            trace_id: None,
            data: serde_json::to_value(data).expect("event data must be serialisable"),
        }
    }

    /// Attach a trace ID to this event.
    pub fn with_trace(mut self, trace_id: String) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// NATS subject for this event: `fc.{org_id}.vault.activity`.
    pub fn subject(&self) -> String {
        format!("fc.{}.vault.activity", self.org_id)
    }
}

// ---------------------------------------------------------------------------
// Read event data
// ---------------------------------------------------------------------------

/// A note was read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteReadEvent {
    pub path: String,
}

/// A search was executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEvent {
    pub query: String,
    pub result_count: usize,
}

/// Files were listed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEvent {
    pub directory: String,
    pub result_count: usize,
}

/// Links were queried.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinksEvent {
    pub path: String,
    pub direction: String,
    pub result_count: usize,
}

// ---------------------------------------------------------------------------
// Write event data
// ---------------------------------------------------------------------------

/// A new note was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteCreatedEvent {
    pub path: String,
    pub size_bytes: usize,
}

/// An existing note was updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteUpdatedEvent {
    pub path: String,
    pub version: u64,
    pub size_bytes: usize,
}

/// A note was moved/renamed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteMovedEvent {
    pub from_path: String,
    pub to_path: String,
}

/// A note was deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteDeletedEvent {
    pub path: String,
}

// ---------------------------------------------------------------------------
// Access failure
// ---------------------------------------------------------------------------

/// An access attempt was denied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessDeniedEvent {
    pub path: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Snapshot events
// ---------------------------------------------------------------------------

/// A snapshot was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotCreatedEvent {
    pub snapshot_id: String,
    pub path: String,
    pub version: u64,
}

/// A snapshot was restored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRestoredEvent {
    pub snapshot_id: String,
    pub path: String,
    pub restored_version: u64,
}

/// A snapshot was deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDeletedEvent {
    pub snapshot_id: String,
    pub path: String,
}

/// Snapshots were listed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotListedEvent {
    pub path: String,
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_generation() {
        let evt = VaultEvent::new(
            "vault.note.created",
            "acme-corp",
            &NoteCreatedEvent {
                path: "hello.md".into(),
                size_bytes: 42,
            },
        );
        assert_eq!(evt.subject(), "fc.acme-corp.vault.activity");
        assert_eq!(evt.specversion, "1.0");
        assert_eq!(evt.source, "fc.vault");
    }

    #[test]
    fn serialization_roundtrip() {
        let evt = VaultEvent::new(
            "vault.note.updated",
            "org-123",
            &NoteUpdatedEvent {
                path: "docs/readme.md".into(),
                version: 7,
                size_bytes: 1024,
            },
        );

        let json = serde_json::to_string(&evt).expect("serialize");
        let back: VaultEvent = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.event_type, "vault.note.updated");
        assert_eq!(back.org_id, "org-123");
        assert_eq!(back.specversion, "1.0");
        assert_eq!(back.source, "fc.vault");

        // Verify embedded data round-trips
        let data: NoteUpdatedEvent =
            serde_json::from_value(back.data).expect("deserialize data");
        assert_eq!(data.path, "docs/readme.md");
        assert_eq!(data.version, 7);
    }

    #[test]
    fn with_trace_attaches_id() {
        let evt = VaultEvent::new("vault.note.read", "org-1", &NoteReadEvent {
            path: "a.md".into(),
        })
        .with_trace("trace-abc-123".into());

        assert_eq!(evt.trace_id.as_deref(), Some("trace-abc-123"));
    }

    #[test]
    fn trace_id_omitted_when_none() {
        let evt = VaultEvent::new("vault.note.read", "org-1", &NoteReadEvent {
            path: "a.md".into(),
        });
        let json = serde_json::to_string(&evt).unwrap();
        assert!(!json.contains("trace_id"));
    }
}
