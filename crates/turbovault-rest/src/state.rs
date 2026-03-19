use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use turbovault_core::prelude::MultiVaultManager;
use turbovault_vault::VaultManager;

/// Configuration for the REST API
#[derive(Clone, Debug)]
pub struct RestConfig {
    /// Optional Bearer token for auth. None = allow all (LAN trust).
    pub api_token: Option<String>,
    /// Paths that reject write operations (e.g., "Focus Areas/Writing/")
    pub protected_paths: Vec<String>,
}

/// Shared state for all REST handlers
#[derive(Clone)]
pub struct AppState {
    pub multi_vault: Arc<MultiVaultManager>,
    pub config: RestConfig,
    pub start_time: std::time::Instant,
    /// Cache of initialized VaultManagers keyed by vault name
    pub vault_managers: Arc<RwLock<HashMap<String, Arc<VaultManager>>>>,
}
