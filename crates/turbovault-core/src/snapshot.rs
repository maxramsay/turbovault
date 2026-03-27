//! Snapshot manifest types and selection model.
//!
//! Defines the structures written as `manifest.json` inside snapshot tar.gz
//! archives. Used by snapshot creation/restore tools, REST endpoints, and the CLI.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The manifest stored inside every snapshot archive as `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    /// Unique snapshot identifier (timestamp + selection suffix).
    pub snapshot_id: String,
    /// Schema version of this manifest format.
    pub format_version: u32,
    /// Organisation that owns the vault.
    pub org_id: String,
    /// When the snapshot was created.
    pub created_at: DateTime<Utc>,
    /// Actor who initiated the snapshot.
    pub created_by: String,
    /// What was included in the snapshot.
    pub selection: SnapshotSelection,
    /// Target label (e.g. storage backend or path).
    pub target: String,
    /// Number of notes in the snapshot.
    pub note_count: usize,
    /// Total size of all note content in bytes.
    pub total_size_bytes: u64,
    /// Aggregate hash over all note hashes.
    pub total_hash: String,
    /// Individual note entries.
    pub notes: Vec<SnapshotNote>,
    /// Links that cross the snapshot boundary.
    pub boundary_links: Vec<BoundaryLink>,
}

/// Describes what was selected for inclusion in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SnapshotSelection {
    /// The entire vault.
    #[serde(rename = "all")]
    All,
    /// A subset of the vault filtered by tags.
    #[serde(rename = "tags")]
    Tags { tags: Vec<String> },
}

/// Metadata for a single note inside a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotNote {
    /// Relative path within the vault.
    pub path: String,
    /// Note version at time of snapshot.
    pub version: u64,
    /// Content hash of the note body.
    pub hash: String,
    /// Size of the note content in bytes.
    pub size_bytes: u64,
    /// Tags present on this note.
    pub tags: Vec<String>,
}

/// A link that crosses the snapshot boundary (one end included, one not).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryLink {
    /// Source note path.
    pub from: String,
    /// Target note path.
    pub to: String,
    /// Type of link (e.g. "wikilink", "embed").
    pub link_type: String,
    /// Whether the target note is included in the snapshot.
    pub included: bool,
}

/// Request payload for creating a new snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotCreateRequest {
    /// If provided, only include notes matching these tags.
    pub tags: Option<Vec<String>>,
    /// Optional target label override.
    pub target: Option<String>,
    /// Optional organisation ID override.
    pub org_id: Option<String>,
}

/// Request payload for restoring a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRestoreRequest {
    /// How to apply the restored notes.
    pub mode: RestoreMode,
}

/// Controls how restored notes are written back to the vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RestoreMode {
    /// Write restored notes to a staging area for review.
    Staging,
    /// Overwrite existing notes in place.
    InPlace,
}

impl SnapshotManifest {
    /// Generate a snapshot ID from the current timestamp and selection.
    ///
    /// Format: `YYYYMMDD-HHMMSS-{suffix}` where suffix is `full-vault` for
    /// `All` or the tag names joined by `-` for `Tags`.
    pub fn generate_id(selection: &SnapshotSelection) -> String {
        let now = Utc::now();
        let timestamp = now.format("%Y%m%d-%H%M%S");
        let suffix = match selection {
            SnapshotSelection::All => "full-vault".to_string(),
            SnapshotSelection::Tags { tags } => tags.join("-"),
        };
        format!("{}-{}", timestamp, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_id_all() {
        let id = SnapshotManifest::generate_id(&SnapshotSelection::All);
        assert!(id.ends_with("-full-vault"), "ID should end with -full-vault, got: {}", id);
        // Format: YYYYMMDD-HHMMSS-full-vault
        assert!(id.len() > 20, "ID should be longer than 20 chars, got: {}", id);
    }

    #[test]
    fn test_generate_id_tags() {
        let selection = SnapshotSelection::Tags {
            tags: vec!["project".to_string(), "active".to_string()],
        };
        let id = SnapshotManifest::generate_id(&selection);
        assert!(
            id.ends_with("-project-active"),
            "ID should end with tag names joined by -, got: {}",
            id
        );
    }

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let manifest = SnapshotManifest {
            snapshot_id: "20260327-120000-full-vault".to_string(),
            format_version: 1,
            org_id: "org-test".to_string(),
            created_at: Utc::now(),
            created_by: "test-user".to_string(),
            selection: SnapshotSelection::Tags {
                tags: vec!["docs".to_string()],
            },
            target: "local".to_string(),
            note_count: 2,
            total_size_bytes: 4096,
            total_hash: "sha256:abc123".to_string(),
            notes: vec![
                SnapshotNote {
                    path: "notes/one.md".to_string(),
                    version: 3,
                    hash: "sha256:aaa".to_string(),
                    size_bytes: 2048,
                    tags: vec!["docs".to_string()],
                },
                SnapshotNote {
                    path: "notes/two.md".to_string(),
                    version: 1,
                    hash: "sha256:bbb".to_string(),
                    size_bytes: 2048,
                    tags: vec!["docs".to_string()],
                },
            ],
            boundary_links: vec![BoundaryLink {
                from: "notes/one.md".to_string(),
                to: "external/ref.md".to_string(),
                link_type: "wikilink".to_string(),
                included: false,
            }],
        };

        let json = serde_json::to_string_pretty(&manifest).expect("serialize");
        let roundtrip: SnapshotManifest =
            serde_json::from_str(&json).expect("deserialize");

        assert_eq!(roundtrip.snapshot_id, manifest.snapshot_id);
        assert_eq!(roundtrip.format_version, manifest.format_version);
        assert_eq!(roundtrip.org_id, manifest.org_id);
        assert_eq!(roundtrip.note_count, manifest.note_count);
        assert_eq!(roundtrip.total_size_bytes, manifest.total_size_bytes);
        assert_eq!(roundtrip.notes.len(), 2);
        assert_eq!(roundtrip.boundary_links.len(), 1);
        assert_eq!(roundtrip.boundary_links[0].included, false);
    }

    #[test]
    fn test_restore_mode_deserialization() {
        let staging: RestoreMode =
            serde_json::from_str("\"staging\"").expect("parse staging");
        assert_eq!(staging, RestoreMode::Staging);

        let in_place: RestoreMode =
            serde_json::from_str("\"in-place\"").expect("parse in-place");
        assert_eq!(in_place, RestoreMode::InPlace);
    }
}
