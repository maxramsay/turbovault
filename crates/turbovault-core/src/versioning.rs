//! Note versioning types and frontmatter helpers.
//!
//! Provides optimistic concurrency control for note writes. Every note gets a
//! `version` (monotonically increasing integer) and `history` (array of entries)
//! stored in YAML frontmatter. The content hash covers the body only (everything
//! below the frontmatter `---` delimiter), not the frontmatter itself.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// A single entry in the version history of a note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    /// Monotonically increasing version number.
    pub version: u64,
    /// SHA-256 hash of the body content at this version (`sha256:{hex}`).
    pub hash: String,
    /// Actor who made this change (user ID, service name, etc.).
    pub by: String,
    /// Timestamp of the change.
    pub at: DateTime<Utc>,
}

/// Version metadata for a note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteVersionInfo {
    /// Current version number.
    pub version: u64,
    /// Ordered history of changes.
    pub history: Vec<HistoryEntry>,
}

impl Default for NoteVersionInfo {
    fn default() -> Self {
        Self {
            version: 0,
            history: Vec::new(),
        }
    }
}

/// Error returned when a version conflict is detected during update.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionConflict {
    /// The version the caller expected.
    pub expected: u64,
    /// The actual current version of the note.
    pub actual: u64,
}

impl fmt::Display for VersionConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "version conflict: expected {}, actual {}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for VersionConflict {}

// ---------------------------------------------------------------------------
// Body / frontmatter extraction
// ---------------------------------------------------------------------------

/// Extract the body below the YAML frontmatter delimiters.
///
/// Frontmatter is defined as content between the first `---\n` at the start of
/// the file and the next `---\n` (or `---` at EOF). If no frontmatter is
/// present, the entire content is the body.
pub fn extract_body(content: &str) -> &str {
    if !content.starts_with("---") {
        return content;
    }
    // Find the closing delimiter. The opening `---` is at index 0; skip past
    // the first line to search for the closing one.
    let after_open = match content[3..].find('\n') {
        Some(i) => 3 + i + 1, // skip past the newline
        None => return "", // file is just `---` with no newline
    };
    match content[after_open..].find("\n---") {
        Some(i) => {
            let close_start = after_open + i + 4; // skip past `\n---`
            // Skip the newline after the closing `---` if present.
            if content[close_start..].starts_with('\n') {
                &content[close_start + 1..]
            } else {
                &content[close_start..]
            }
        }
        None => content, // no closing delimiter -> treat whole thing as body
    }
}

/// Compute a SHA-256 hash of the body content only (below frontmatter).
///
/// Returns a string in the format `sha256:{hex}`.
pub fn compute_content_hash(full_content: &str) -> String {
    let body = extract_body(full_content);
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{:x}", result)
}

// ---------------------------------------------------------------------------
// Frontmatter read / write helpers (serde_json::Map)
// ---------------------------------------------------------------------------

/// Read version info from a parsed frontmatter map.
///
/// Missing fields are treated as version 0 with empty history.
pub fn read_version_info(
    frontmatter: &serde_json::Map<String, serde_json::Value>,
) -> NoteVersionInfo {
    let version = frontmatter
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let history: Vec<HistoryEntry> = frontmatter
        .get("history")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    NoteVersionInfo { version, history }
}

/// Write version info into a parsed frontmatter map.
pub fn write_version_info(
    frontmatter: &mut serde_json::Map<String, serde_json::Value>,
    info: &NoteVersionInfo,
) {
    frontmatter.insert(
        "version".to_string(),
        serde_json::Value::Number(serde_json::Number::from(info.version)),
    );
    frontmatter.insert(
        "history".to_string(),
        serde_json::to_value(&info.history).expect("HistoryEntry is always serializable"),
    );
}

// ---------------------------------------------------------------------------
// Raw-content helpers
// ---------------------------------------------------------------------------

/// Parse YAML frontmatter from raw content and return version info.
///
/// If the content has no frontmatter or no version fields, returns defaults.
pub fn read_version_from_content(content: &str) -> NoteVersionInfo {
    let yaml = match extract_frontmatter_yaml(content) {
        Some(y) => y,
        None => return NoteVersionInfo::default(),
    };
    let map: serde_json::Map<String, serde_json::Value> = match serde_yaml::from_str(yaml) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return NoteVersionInfo::default(),
    };
    read_version_info(&map)
}

/// Create a new `HistoryEntry` with the current UTC timestamp.
pub fn new_history_entry(version: u64, hash: String, actor: String) -> HistoryEntry {
    HistoryEntry {
        version,
        hash,
        by: actor,
        at: Utc::now(),
    }
}

/// Create a new versioned note (version 1).
///
/// Returns the content with injected frontmatter and the resulting version info.
pub fn apply_version_create(content: &str, actor: &str) -> (String, NoteVersionInfo) {
    let hash = compute_content_hash(content);
    let entry = new_history_entry(1, hash, actor.to_string());
    let info = NoteVersionInfo {
        version: 1,
        history: vec![entry],
    };
    let new_content = inject_version_frontmatter(content, &info);
    (new_content, info)
}

/// Update an existing note with version enforcement (optimistic concurrency).
///
/// `expected_version` must match the current version in `current_content`.
/// On success, returns the new content and updated version info.
pub fn apply_version_update(
    current_content: &str,
    new_body: &str,
    expected_version: u64,
    actor: &str,
) -> Result<(String, NoteVersionInfo), VersionConflict> {
    let current_info = read_version_from_content(current_content);
    if current_info.version != expected_version {
        return Err(VersionConflict {
            expected: expected_version,
            actual: current_info.version,
        });
    }

    let new_version = current_info.version + 1;
    // Replace the body first so the hash covers the new body.
    let replaced = replace_body(current_content, new_body);
    let hash = compute_content_hash(&replaced);
    let entry = new_history_entry(new_version, hash, actor.to_string());

    let mut history = current_info.history;
    history.push(entry);

    let info = NoteVersionInfo {
        version: new_version,
        history,
    };
    let final_content = inject_version_frontmatter(&replaced, &info);
    Ok((final_content, info))
}

/// Replace the body of a note (below frontmatter) while preserving frontmatter.
pub fn replace_body(content: &str, new_body: &str) -> String {
    if !content.starts_with("---") {
        return new_body.to_string();
    }
    let after_open = match content[3..].find('\n') {
        Some(i) => 3 + i + 1,
        None => return new_body.to_string(),
    };
    match content[after_open..].find("\n---") {
        Some(i) => {
            let close_start = after_open + i + 4; // includes `\n---`
            let fm_end = if content[close_start..].starts_with('\n') {
                close_start + 1
            } else {
                close_start
            };
            let frontmatter_block = &content[..fm_end];
            format!("{}{}", frontmatter_block, new_body)
        }
        None => new_body.to_string(),
    }
}

/// Inject (or update) version fields in the YAML frontmatter of raw content.
///
/// If the content has existing frontmatter, the `version` and `history` keys
/// are updated in place. If there is no frontmatter, one is created.
pub fn inject_version_frontmatter(content: &str, info: &NoteVersionInfo) -> String {
    let (yaml_str, body) = if content.starts_with("---") {
        match extract_frontmatter_yaml(content) {
            Some(yaml) => (yaml.to_string(), extract_body(content)),
            None => (String::new(), content),
        }
    } else {
        (String::new(), content)
    };

    // Parse existing frontmatter or start fresh.
    let mut map: serde_json::Map<String, serde_json::Value> = if yaml_str.is_empty() {
        serde_json::Map::new()
    } else {
        match serde_yaml::from_str(&yaml_str) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    };

    write_version_info(&mut map, info);

    // Serialize back to YAML.
    let value = serde_json::Value::Object(map);
    let new_yaml =
        serde_yaml::to_string(&value).expect("serde_json::Value always serializes to YAML");

    format!("---\n{}---\n{}", new_yaml, body)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extract the raw YAML string between frontmatter delimiters.
fn extract_frontmatter_yaml(content: &str) -> Option<&str> {
    if !content.starts_with("---") {
        return None;
    }
    let after_open = content[3..].find('\n').map(|i| 3 + i + 1)?;
    let close_offset = content[after_open..].find("\n---")?;
    Some(&content[after_open..after_open + close_offset])
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE_WITH_FM: &str = "---\ntitle: Test Note\nversion: 3\nhistory:\n  - version: 1\n    hash: \"sha256:aaa\"\n    by: alice\n    at: \"2025-01-01T00:00:00Z\"\n  - version: 2\n    hash: \"sha256:bbb\"\n    by: bob\n    at: \"2025-06-15T12:00:00Z\"\n  - version: 3\n    hash: \"sha256:ccc\"\n    by: alice\n    at: \"2025-12-01T08:30:00Z\"\n---\nHello, world!\n";

    const NOTE_NO_VERSION: &str = "---\ntitle: Plain Note\ntags:\n  - test\n---\nSome body content.\n";

    const NOTE_NO_FM: &str = "Just a plain note with no frontmatter.\n";

    // --- Content hash tests ---

    #[test]
    fn test_compute_content_hash_excludes_frontmatter() {
        // Hash should be the same for identical bodies regardless of frontmatter.
        let a = "---\ntitle: A\n---\nBody text";
        let b = "---\ntitle: B\nversion: 5\n---\nBody text";
        assert_eq!(compute_content_hash(a), compute_content_hash(b));
        // And it should be a well-formed sha256: string.
        let hash = compute_content_hash(a);
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 7 + 64); // "sha256:" + 64 hex chars
    }

    #[test]
    fn test_compute_content_hash_no_frontmatter() {
        // Without frontmatter the entire content is the body.
        let hash = compute_content_hash(NOTE_NO_FM);
        assert!(hash.starts_with("sha256:"));
        // Different content -> different hash.
        assert_ne!(hash, compute_content_hash("Different content"));
    }

    // --- extract_body tests ---

    #[test]
    fn test_extract_body() {
        assert_eq!(extract_body(NOTE_WITH_FM), "Hello, world!\n");
        assert_eq!(extract_body(NOTE_NO_VERSION), "Some body content.\n");
        assert_eq!(extract_body(NOTE_NO_FM), NOTE_NO_FM);
    }

    // --- read_version_from_content tests ---

    #[test]
    fn test_read_version_from_content_with_version() {
        let info = read_version_from_content(NOTE_WITH_FM);
        assert_eq!(info.version, 3);
        assert_eq!(info.history.len(), 3);
        assert_eq!(info.history[0].by, "alice");
        assert_eq!(info.history[1].by, "bob");
        assert_eq!(info.history[2].hash, "sha256:ccc");
    }

    #[test]
    fn test_read_version_from_content_without_version() {
        let info = read_version_from_content(NOTE_NO_VERSION);
        assert_eq!(info.version, 0);
        assert!(info.history.is_empty());
    }

    #[test]
    fn test_read_version_from_content_no_frontmatter() {
        let info = read_version_from_content(NOTE_NO_FM);
        assert_eq!(info.version, 0);
        assert!(info.history.is_empty());
    }

    // --- apply_version_create ---

    #[test]
    fn test_apply_version_create() {
        let (content, info) = apply_version_create("---\ntitle: New\n---\nBody here", "alice");
        assert_eq!(info.version, 1);
        assert_eq!(info.history.len(), 1);
        assert_eq!(info.history[0].version, 1);
        assert_eq!(info.history[0].by, "alice");
        assert!(info.history[0].hash.starts_with("sha256:"));

        // The resulting content should round-trip.
        let rt_info = read_version_from_content(&content);
        assert_eq!(rt_info.version, 1);
        assert_eq!(rt_info.history.len(), 1);

        // Body should be preserved.
        assert_eq!(extract_body(&content), "Body here");
    }

    // --- apply_version_update ---

    #[test]
    fn test_apply_version_update_success() {
        // Start with a version-1 note.
        let (v1_content, _) = apply_version_create("---\ntitle: Note\n---\nOriginal body", "alice");

        let result = apply_version_update(&v1_content, "Updated body", 1, "bob");
        assert!(result.is_ok());
        let (v2_content, v2_info) = result.unwrap();
        assert_eq!(v2_info.version, 2);
        assert_eq!(v2_info.history.len(), 2);
        assert_eq!(v2_info.history[1].by, "bob");
        assert_eq!(extract_body(&v2_content), "Updated body");

        // Hash should cover the new body.
        let expected_hash = compute_content_hash(&v2_content);
        assert_eq!(v2_info.history[1].hash, expected_hash);
    }

    #[test]
    fn test_apply_version_update_conflict() {
        let (v1_content, _) = apply_version_create("---\ntitle: Note\n---\nBody", "alice");

        let result = apply_version_update(&v1_content, "New body", 99, "bob");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.expected, 99);
        assert_eq!(err.actual, 1);
    }

    // --- Serialization round-trip ---

    #[test]
    fn test_history_entry_serialization_roundtrip() {
        let entry = HistoryEntry {
            version: 42,
            hash: "sha256:abc123".to_string(),
            by: "test-actor".to_string(),
            at: "2025-06-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap(),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: HistoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, deserialized);

        // Also test YAML round-trip.
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let from_yaml: HistoryEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(entry, from_yaml);
    }
}
