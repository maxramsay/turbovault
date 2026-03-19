//! Trash manifest CRUD for soft-delete lifecycle.
//!
//! The manifest lives at `.trash/.manifest.json` inside the vault directory.
//! It tracks every soft-deleted note so it can be restored or purge-requested.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrashEntry {
    pub original_path: String,
    pub trash_path: String,
    pub deleted_at: String, // ISO 8601
    pub orphaned_links: Vec<String>, // paths of notes that linked to the deleted note
    pub permanent_delete_requested: Option<String>, // ISO 8601 or null
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct TrashManifest {
    pub entries: Vec<TrashEntry>,
}

impl TrashManifest {
    /// Load manifest from `.trash/.manifest.json`. Returns empty manifest if
    /// the file does not exist.
    pub async fn load(vault_path: &Path) -> Result<Self, std::io::Error> {
        let manifest_path = vault_path.join(".trash").join(".manifest.json");
        if !manifest_path.exists() {
            return Ok(Self::default());
        }
        let data = tokio::fs::read_to_string(&manifest_path).await?;
        let manifest: TrashManifest = serde_json::from_str(&data).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        Ok(manifest)
    }

    /// Write manifest to `.trash/.manifest.json`, creating `.trash/` if needed.
    pub async fn save(&self, vault_path: &Path) -> Result<(), std::io::Error> {
        let trash_dir = vault_path.join(".trash");
        tokio::fs::create_dir_all(&trash_dir).await?;
        let manifest_path = trash_dir.join(".manifest.json");
        let data = serde_json::to_string_pretty(self).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e)
        })?;
        tokio::fs::write(&manifest_path, data).await
    }

    /// Add an entry and persist.
    pub async fn add_entry(
        &mut self,
        entry: TrashEntry,
        vault_path: &Path,
    ) -> Result<(), std::io::Error> {
        self.entries.push(entry);
        self.save(vault_path).await
    }

    /// Remove an entry by trash_path, returning the removed entry if found.
    pub fn remove_entry(&mut self, trash_path: &str) -> Option<TrashEntry> {
        let idx = self
            .entries
            .iter()
            .position(|e| e.trash_path == trash_path)?;
        Some(self.entries.remove(idx))
    }

    /// Look up an entry by trash_path.
    pub fn find_entry(&self, trash_path: &str) -> Option<&TrashEntry> {
        self.entries.iter().find(|e| e.trash_path == trash_path)
    }

    /// Mark an entry as purge-requested. Returns true if found.
    pub fn mark_purge_requested(&mut self, trash_path: &str) -> bool {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.trash_path == trash_path)
        {
            entry.permanent_delete_requested =
                Some(chrono::Utc::now().to_rfc3339());
            true
        } else {
            false
        }
    }
}
