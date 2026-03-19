//! Resolve the target vault from an incoming request.
//!
//! Handlers call [`resolve_vault`] to obtain the correct [`VaultManager`] for
//! the request. Resolution order:
//!
//! 1. If the request carries an `X-Vault` header, look up that vault name.
//! 2. Otherwise use the active vault from `MultiVaultManager`.
//!
//! `VaultManager` instances are created lazily and cached in
//! `AppState::vault_managers` for the lifetime of the server process.

use std::sync::Arc;

use axum::http::HeaderMap;
use turbovault_core::prelude::ServerConfig;
use turbovault_vault::VaultManager;

use crate::{errors::ApiError, state::AppState};

/// Resolve the vault for a request and return `(vault_name, Arc<VaultManager>)`.
///
/// Returns [`ApiError::VaultNotFound`] when the requested vault does not exist,
/// or [`ApiError::Internal`] when the manager cannot be initialised.
pub async fn resolve_vault(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(String, Arc<VaultManager>), ApiError> {
    // Determine vault name from X-Vault header or fall back to active vault.
    let vault_name = if let Some(header_val) = headers.get("X-Vault") {
        let name = header_val
            .to_str()
            .map_err(|_| ApiError::InvalidRequest("X-Vault header is not valid UTF-8".into()))?
            .to_owned();

        // Validate the vault exists before proceeding.
        if !state.multi_vault.vault_exists(&name).await {
            return Err(ApiError::VaultNotFound(name));
        }

        name
    } else {
        let active = state.multi_vault.get_active_vault().await;
        if active.is_empty() {
            return Err(ApiError::VaultNotFound(
                "No active vault configured".into(),
            ));
        }
        active
    };

    // Fast path: return cached manager if available.
    {
        let cache = state.vault_managers.read().await;
        if let Some(manager) = cache.get(&vault_name) {
            return Ok((vault_name, manager.clone()));
        }
    }

    // Slow path: build a VaultManager for this vault.
    let vault_config = state
        .multi_vault
        .get_vault_config(&vault_name)
        .await
        .map_err(|e| ApiError::VaultNotFound(format!("{}: {}", vault_name, e)))?;

    let mut server_config = ServerConfig::default();
    let mut vault_config = vault_config;
    // VaultManager::new() calls config.default_vault(), which requires is_default == true.
    vault_config.is_default = true;
    server_config.vaults = vec![vault_config];

    let manager = VaultManager::new(server_config)
        .map_err(|e| ApiError::Internal(format!("Failed to create vault manager: {}", e)))?;

    manager
        .initialize()
        .await
        .map_err(|e| ApiError::Internal(format!("Failed to initialize vault: {}", e)))?;

    let manager = Arc::new(manager);

    // Store in cache.
    {
        let mut cache = state.vault_managers.write().await;
        // Another task may have raced us — only insert if still absent.
        cache.entry(vault_name.clone()).or_insert(manager.clone());
    }

    Ok((vault_name, manager))
}
