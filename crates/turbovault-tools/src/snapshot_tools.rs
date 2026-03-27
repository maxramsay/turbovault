//! Snapshot create/restore/list/delete operations.
//!
//! [`SnapshotTools`] encapsulates the core logic for vault snapshots. It walks the
//! vault directory, applies selection filters, creates tar.gz archives containing
//! a manifest and note files, and supports staging or in-place restore.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use regex::Regex;
use sha2::{Digest, Sha256};
use tar::{Archive, Builder};
use tracing;
use walkdir::WalkDir;

use turbovault_core::snapshot::{
    BoundaryLink, RestoreMode, SnapshotManifest, SnapshotNote, SnapshotSelection,
};
use turbovault_core::versioning::{compute_content_hash, read_version_from_content};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during snapshot operations.
#[derive(Debug)]
pub enum SnapshotError {
    /// An I/O error occurred.
    Io(std::io::Error),
    /// A serialization/deserialization error occurred.
    Serialization(serde_json::Error),
    /// No notes matched the selection filter.
    NoMatchingNotes,
    /// The manifest.json was not found in the archive.
    ManifestNotFound,
    /// The requested snapshot was not found.
    NotFound(String),
    /// A path could not be resolved or is invalid.
    PathError,
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Io(e) => write!(f, "I/O error: {}", e),
            SnapshotError::Serialization(e) => write!(f, "serialization error: {}", e),
            SnapshotError::NoMatchingNotes => write!(f, "no notes matched the selection filter"),
            SnapshotError::ManifestNotFound => {
                write!(f, "manifest.json not found in snapshot archive")
            }
            SnapshotError::NotFound(id) => write!(f, "snapshot not found: {}", id),
            SnapshotError::PathError => write!(f, "path resolution error"),
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SnapshotError::Io(e) => Some(e),
            SnapshotError::Serialization(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SnapshotError {
    fn from(e: std::io::Error) -> Self {
        SnapshotError::Io(e)
    }
}

impl From<serde_json::Error> for SnapshotError {
    fn from(e: serde_json::Error) -> Self {
        SnapshotError::Serialization(e)
    }
}

// ---------------------------------------------------------------------------
// Collected note (internal)
// ---------------------------------------------------------------------------

/// A note collected during the vault walk, before filtering.
struct CollectedNote {
    /// Relative path from vault root (e.g. `subfolder/note.md`).
    relative_path: String,
    /// Raw file content.
    content: String,
    /// Tags extracted from YAML frontmatter.
    tags: Vec<String>,
    /// Version number from frontmatter (0 if absent).
    version: u64,
    /// Content hash (sha256:…).
    hash: String,
    /// File size in bytes.
    size_bytes: u64,
}

// ---------------------------------------------------------------------------
// SnapshotTools
// ---------------------------------------------------------------------------

/// Core snapshot operations for vault backup and restore.
pub struct SnapshotTools {
    vault_root: PathBuf,
}

impl SnapshotTools {
    /// Create a new `SnapshotTools` bound to the given vault root directory.
    ///
    /// The path is canonicalized to resolve symlinks (e.g. `/var` -> `/private/var`
    /// on macOS) so that `strip_prefix` works reliably during directory walks.
    pub fn new(vault_root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = vault_root.into();
        let canonical = root.canonicalize().unwrap_or(root);
        Self {
            vault_root: canonical,
        }
    }

    // -----------------------------------------------------------------------
    // create_snapshot
    // -----------------------------------------------------------------------

    /// Create a snapshot archive of the vault.
    ///
    /// 1. Walk the vault, collecting `.md` files (skipping hidden dirs and
    ///    reserved directories like `_restore`, `_snapshots`, `_history`).
    /// 2. Apply the selection filter.
    /// 3. Build a tar.gz archive containing the manifest and note files.
    pub async fn create_snapshot(
        &self,
        selection: &SnapshotSelection,
        target_dir: &str,
        org_id: &str,
        actor: &str,
    ) -> Result<SnapshotManifest, SnapshotError> {
        // Step 1: Collect all eligible notes.
        let all_notes = self.collect_notes().await?;

        // Step 2: Apply selection filter.
        let selected = Self::apply_filter(&all_notes, selection);
        if selected.is_empty() {
            return Err(SnapshotError::NoMatchingNotes);
        }

        // Step 3: Generate snapshot ID and build manifest.
        let snapshot_id = SnapshotManifest::generate_id(selection);
        let now = Utc::now();

        // Build the set of included note stems for boundary link detection.
        let included_stems: HashSet<String> = selected
            .iter()
            .map(|n| Self::stem_from_path(&n.relative_path))
            .collect();

        // Detect boundary links.
        let wikilink_re = Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]")
            .expect("wikilink regex is valid");
        let mut boundary_links = Vec::new();
        for note in &selected {
            for cap in wikilink_re.captures_iter(&note.content) {
                let target = cap[1].trim();
                let target_stem = target.to_string();
                if !included_stems.contains(&target_stem) {
                    boundary_links.push(BoundaryLink {
                        from: note.relative_path.clone(),
                        to: target_stem,
                        link_type: "wikilink".to_string(),
                        included: false,
                    });
                }
            }
        }

        // Compute aggregate hash.
        let mut hasher = Sha256::new();
        for note in &selected {
            hasher.update(note.hash.as_bytes());
        }
        let total_hash = format!("sha256:{:x}", hasher.finalize());

        let total_size_bytes: u64 = selected.iter().map(|n| n.size_bytes).sum();

        let snapshot_notes: Vec<SnapshotNote> = selected
            .iter()
            .map(|n| SnapshotNote {
                path: n.relative_path.clone(),
                version: n.version,
                hash: n.hash.clone(),
                size_bytes: n.size_bytes,
                tags: n.tags.clone(),
            })
            .collect();

        let manifest = SnapshotManifest {
            snapshot_id: snapshot_id.clone(),
            format_version: 1,
            org_id: org_id.to_string(),
            created_at: now,
            created_by: actor.to_string(),
            selection: selection.clone(),
            target: target_dir.to_string(),
            note_count: snapshot_notes.len(),
            total_size_bytes,
            total_hash,
            notes: snapshot_notes,
            boundary_links,
        };

        // Step 4: Create the tar.gz archive.
        let target_path = Path::new(target_dir);
        std::fs::create_dir_all(target_path)?;
        let archive_path = target_path.join(format!("{}.tar.gz", &snapshot_id));

        let file = std::fs::File::create(&archive_path)?;
        let enc = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(enc);

        // Write manifest.json.
        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        let manifest_bytes = manifest_json.as_bytes();
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(
            &mut header,
            format!("{}/manifest.json", &snapshot_id),
            manifest_bytes,
        )?;

        // Write each note.
        for note in &selected {
            let note_bytes = note.content.as_bytes();
            let mut note_header = tar::Header::new_gnu();
            note_header.set_size(note_bytes.len() as u64);
            note_header.set_mode(0o644);
            note_header.set_cksum();
            builder.append_data(
                &mut note_header,
                format!("{}/notes/{}", &snapshot_id, &note.relative_path),
                note_bytes,
            )?;
        }

        builder.into_inner()?.finish()?;

        tracing::info!(
            snapshot_id = %manifest.snapshot_id,
            note_count = manifest.note_count,
            "snapshot created"
        );

        Ok(manifest)
    }

    // -----------------------------------------------------------------------
    // restore_snapshot
    // -----------------------------------------------------------------------

    /// Restore notes from a snapshot archive.
    ///
    /// - **Staging**: extracts to `{vault_root}/_restore/{snapshot_id}/`.
    /// - **InPlace**: creates a safety snapshot first, then overwrites vault files.
    pub async fn restore_snapshot(
        &self,
        archive_path: &Path,
        mode: &RestoreMode,
    ) -> Result<SnapshotManifest, SnapshotError> {
        let manifest = Self::read_manifest_from_archive(archive_path)?;
        let snapshot_id = &manifest.snapshot_id;

        match mode {
            RestoreMode::Staging => {
                let restore_dir = self.vault_root.join("_restore").join(snapshot_id);
                std::fs::create_dir_all(&restore_dir)?;
                self.extract_notes_from_archive(archive_path, snapshot_id, &restore_dir)?;
            }
            RestoreMode::InPlace => {
                // Safety: create a pre-restore full snapshot.
                let safety_dir = self.vault_root.join("_snapshots").join("pre-restore");
                let safety_dir_str = safety_dir.to_string_lossy().to_string();
                self.create_snapshot(
                    &SnapshotSelection::All,
                    &safety_dir_str,
                    &manifest.org_id,
                    "pre-restore-safety",
                )
                .await?;

                // Overwrite vault files with snapshot contents.
                self.extract_notes_from_archive(archive_path, snapshot_id, &self.vault_root)?;
            }
        }

        tracing::info!(
            snapshot_id = %manifest.snapshot_id,
            mode = ?mode,
            "snapshot restored"
        );

        Ok(manifest)
    }

    // -----------------------------------------------------------------------
    // list_snapshots
    // -----------------------------------------------------------------------

    /// List all snapshot archives in the target directory, sorted newest-first.
    pub async fn list_snapshots(
        &self,
        target_dir: &str,
    ) -> Result<Vec<SnapshotManifest>, SnapshotError> {
        let dir = Path::new(target_dir);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut manifests = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("gz")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map_or(false, |s| s.ends_with(".tar"))
            {
                match Self::read_manifest_from_archive(&path) {
                    Ok(m) => manifests.push(m),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "skipping archive");
                    }
                }
            }
        }

        // Sort newest-first by created_at.
        manifests.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(manifests)
    }

    // -----------------------------------------------------------------------
    // delete_snapshot
    // -----------------------------------------------------------------------

    /// Delete a snapshot archive by its ID.
    pub async fn delete_snapshot(
        &self,
        target_dir: &str,
        snapshot_id: &str,
    ) -> Result<(), SnapshotError> {
        let archive_path = Path::new(target_dir).join(format!("{}.tar.gz", snapshot_id));
        if !archive_path.exists() {
            return Err(SnapshotError::NotFound(snapshot_id.to_string()));
        }
        std::fs::remove_file(&archive_path)?;
        tracing::info!(snapshot_id = %snapshot_id, "snapshot deleted");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // read_manifest_from_archive
    // -----------------------------------------------------------------------

    /// Read and parse `manifest.json` from a tar.gz snapshot archive.
    pub fn read_manifest_from_archive(archive_path: &Path) -> Result<SnapshotManifest, SnapshotError> {
        let file = std::fs::File::open(archive_path)?;
        let dec = GzDecoder::new(file);
        let mut archive = Archive::new(dec);

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_path_buf();
            if path.file_name().and_then(|n| n.to_str()) == Some("manifest.json") {
                let manifest: SnapshotManifest = serde_json::from_reader(&mut entry)?;
                return Ok(manifest);
            }
        }

        Err(SnapshotError::ManifestNotFound)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Walk the vault directory and collect all eligible `.md` files.
    async fn collect_notes(&self) -> Result<Vec<CollectedNote>, SnapshotError> {
        let vault_root = self.vault_root.clone();
        // walkdir is synchronous; run in a blocking context.
        tokio::task::spawn_blocking(move || {
            let mut notes = Vec::new();
            for entry in WalkDir::new(&vault_root)
                .into_iter()
                .filter_entry(|e| !Self::should_skip(e))
            {
                let entry = entry.map_err(|e| {
                    SnapshotError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    ))
                })?;

                if !entry.file_type().is_file() {
                    continue;
                }

                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                let relative = path
                    .strip_prefix(&vault_root)
                    .map_err(|_| SnapshotError::PathError)?
                    .to_string_lossy()
                    .to_string();

                let content = std::fs::read_to_string(path)?;
                let size_bytes = content.len() as u64;
                let hash = compute_content_hash(&content);
                let version_info = read_version_from_content(&content);
                let tags = Self::extract_tags_from_content(&content);

                notes.push(CollectedNote {
                    relative_path: relative,
                    content,
                    tags,
                    version: version_info.version,
                    hash,
                    size_bytes,
                });
            }
            Ok(notes)
        })
        .await
        .map_err(|e| {
            SnapshotError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("spawn_blocking failed: {}", e),
            ))
        })?
    }

    /// Determine whether a walkdir entry should be skipped.
    ///
    /// Depth 0 is the vault root itself — never skip it (temp dirs on macOS
    /// have names starting with `.`).
    fn should_skip(entry: &walkdir::DirEntry) -> bool {
        if entry.depth() == 0 {
            return false;
        }
        let name = entry.file_name().to_string_lossy();
        if entry.file_type().is_dir() {
            name.starts_with('.')
                || name == "_restore"
                || name == "_snapshots"
                || name == "_history"
        } else {
            false
        }
    }

    /// Apply the selection filter to collected notes.
    fn apply_filter<'a>(
        notes: &'a [CollectedNote],
        selection: &SnapshotSelection,
    ) -> Vec<&'a CollectedNote> {
        match selection {
            SnapshotSelection::All => notes.iter().collect(),
            SnapshotSelection::Tags { tags } => {
                let tag_set: HashSet<&str> = tags.iter().map(|s| s.as_str()).collect();
                notes
                    .iter()
                    .filter(|n| n.tags.iter().any(|t| tag_set.contains(t.as_str())))
                    .collect()
            }
        }
    }

    /// Extract the note stem (filename without extension and directory) from a
    /// relative path, for use in boundary link matching.
    fn stem_from_path(relative_path: &str) -> String {
        Path::new(relative_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Extract tags from raw note content by parsing YAML frontmatter.
    fn extract_tags_from_content(content: &str) -> Vec<String> {
        if !content.starts_with("---") {
            return Vec::new();
        }
        let after_open = match content[3..].find('\n') {
            Some(i) => 3 + i + 1,
            None => return Vec::new(),
        };
        let yaml_str = match content[after_open..].find("\n---") {
            Some(i) => &content[after_open..after_open + i],
            None => return Vec::new(),
        };

        let map: serde_json::Map<String, serde_json::Value> = match serde_yaml::from_str(yaml_str)
        {
            Ok(serde_json::Value::Object(m)) => m,
            _ => return Vec::new(),
        };

        match map.get("tags") {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Extract notes from a tar.gz archive into a destination directory.
    fn extract_notes_from_archive(
        &self,
        archive_path: &Path,
        snapshot_id: &str,
        dest: &Path,
    ) -> Result<(), SnapshotError> {
        let file = std::fs::File::open(archive_path)?;
        let dec = GzDecoder::new(file);
        let mut archive = Archive::new(dec);

        let notes_prefix = format!("{}/notes/", snapshot_id);

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_path_buf();
            let path_str = path.to_string_lossy().to_string();

            if let Some(relative) = path_str.strip_prefix(&notes_prefix) {
                if relative.is_empty() {
                    continue;
                }
                let target = dest.join(relative);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut out = std::fs::File::create(&target)?;
                std::io::copy(&mut entry, &mut out)?;
            }
        }

        Ok(())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a note file with optional frontmatter tags.
    fn write_note(dir: &Path, relative: &str, tags: &[&str], body: &str) {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut content = String::new();
        if !tags.is_empty() {
            content.push_str("---\ntags:\n");
            for tag in tags {
                content.push_str(&format!("  - {}\n", tag));
            }
            content.push_str("---\n");
        }
        content.push_str(body);
        std::fs::write(path, content).unwrap();
    }

    #[tokio::test]
    async fn test_create_full_snapshot() {
        let vault = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        write_note(vault.path(), "note1.md", &["project"], "Content 1");
        write_note(vault.path(), "note2.md", &["daily"], "Content 2");
        write_note(vault.path(), "sub/note3.md", &[], "Content 3");

        let tools = SnapshotTools::new(vault.path());
        let manifest = tools
            .create_snapshot(
                &SnapshotSelection::All,
                target.path().to_str().unwrap(),
                "org-test",
                "test-user",
            )
            .await
            .expect("create_snapshot should succeed");

        assert_eq!(manifest.note_count, 3);
        assert!(manifest.snapshot_id.ends_with("-full-vault"));
        assert_eq!(manifest.org_id, "org-test");
        assert_eq!(manifest.created_by, "test-user");

        // Archive file should exist.
        let archive = target
            .path()
            .join(format!("{}.tar.gz", &manifest.snapshot_id));
        assert!(archive.exists(), "archive file should exist");
    }

    #[tokio::test]
    async fn test_create_tag_filtered_snapshot() {
        let vault = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        write_note(vault.path(), "a.md", &["fleet-control"], "FC note A");
        write_note(
            vault.path(),
            "b.md",
            &["fleet-control", "project"],
            "FC note B",
        );
        write_note(vault.path(), "c.md", &["daily"], "Daily note");

        let tools = SnapshotTools::new(vault.path());
        let selection = SnapshotSelection::Tags {
            tags: vec!["fleet-control".to_string()],
        };
        let manifest = tools
            .create_snapshot(
                &selection,
                target.path().to_str().unwrap(),
                "org-test",
                "test-user",
            )
            .await
            .expect("filtered snapshot should succeed");

        assert_eq!(manifest.note_count, 2);
        // All included notes should have fleet-control tag.
        for note in &manifest.notes {
            assert!(
                note.tags.contains(&"fleet-control".to_string()),
                "note {} should have fleet-control tag",
                note.path
            );
        }
    }

    #[tokio::test]
    async fn test_snapshot_manifest_in_archive() {
        let vault = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        write_note(vault.path(), "test.md", &["tag1"], "Test body");

        let tools = SnapshotTools::new(vault.path());
        let original = tools
            .create_snapshot(
                &SnapshotSelection::All,
                target.path().to_str().unwrap(),
                "org-test",
                "actor",
            )
            .await
            .unwrap();

        // Read manifest back from archive.
        let archive_path = target
            .path()
            .join(format!("{}.tar.gz", &original.snapshot_id));
        let roundtrip = SnapshotTools::read_manifest_from_archive(&archive_path)
            .expect("should read manifest from archive");

        assert_eq!(roundtrip.snapshot_id, original.snapshot_id);
        assert_eq!(roundtrip.note_count, original.note_count);
        assert_eq!(roundtrip.org_id, original.org_id);
        assert_eq!(roundtrip.total_hash, original.total_hash);
    }

    #[tokio::test]
    async fn test_restore_staging() {
        let vault = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        write_note(vault.path(), "restore_me.md", &[], "Restore this content");
        write_note(
            vault.path(),
            "sub/nested.md",
            &["tag"],
            "Nested content",
        );

        let tools = SnapshotTools::new(vault.path());
        let manifest = tools
            .create_snapshot(
                &SnapshotSelection::All,
                target.path().to_str().unwrap(),
                "org",
                "actor",
            )
            .await
            .unwrap();

        let archive_path = target
            .path()
            .join(format!("{}.tar.gz", &manifest.snapshot_id));

        // Restore in staging mode.
        let restored = tools
            .restore_snapshot(&archive_path, &RestoreMode::Staging)
            .await
            .expect("staging restore should succeed");

        assert_eq!(restored.snapshot_id, manifest.snapshot_id);

        // Verify files exist in _restore dir.
        let restore_dir = vault
            .path()
            .join("_restore")
            .join(&manifest.snapshot_id);
        assert!(
            restore_dir.join("restore_me.md").exists(),
            "restore_me.md should be in staging"
        );
        assert!(
            restore_dir.join("sub/nested.md").exists(),
            "sub/nested.md should be in staging"
        );
    }

    #[tokio::test]
    async fn test_list_snapshots() {
        let vault = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        write_note(vault.path(), "note.md", &["alpha", "beta"], "Content");

        let tools = SnapshotTools::new(vault.path());
        let target_str = target.path().to_str().unwrap();

        // Create two snapshots with different selections so they get different IDs
        // (both run within the same second, so timestamp alone would collide).
        let _m1 = tools
            .create_snapshot(&SnapshotSelection::All, target_str, "org", "a")
            .await
            .unwrap();

        let _m2 = tools
            .create_snapshot(
                &SnapshotSelection::Tags {
                    tags: vec!["alpha".to_string()],
                },
                target_str,
                "org",
                "b",
            )
            .await
            .unwrap();

        let list = tools.list_snapshots(target_str).await.unwrap();
        assert_eq!(list.len(), 2, "should have 2 snapshots");
        // Newest first.
        assert!(list[0].created_at >= list[1].created_at);
    }

    #[tokio::test]
    async fn test_delete_snapshot() {
        let vault = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        write_note(vault.path(), "note.md", &[], "Content");

        let tools = SnapshotTools::new(vault.path());
        let target_str = target.path().to_str().unwrap();

        let manifest = tools
            .create_snapshot(&SnapshotSelection::All, target_str, "org", "a")
            .await
            .unwrap();

        // Delete it.
        tools
            .delete_snapshot(target_str, &manifest.snapshot_id)
            .await
            .expect("delete should succeed");

        // List should now be empty.
        let list = tools.list_snapshots(target_str).await.unwrap();
        assert!(list.is_empty(), "list should be empty after delete");
    }

    #[tokio::test]
    async fn test_no_matching_notes_error() {
        let vault = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();

        write_note(vault.path(), "note.md", &["actual-tag"], "Content");

        let tools = SnapshotTools::new(vault.path());
        let selection = SnapshotSelection::Tags {
            tags: vec!["nonexistent-tag".to_string()],
        };
        let result = tools
            .create_snapshot(
                &selection,
                target.path().to_str().unwrap(),
                "org",
                "actor",
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SnapshotError::NoMatchingNotes),
            "expected NoMatchingNotes, got: {}",
            err
        );
    }
}
