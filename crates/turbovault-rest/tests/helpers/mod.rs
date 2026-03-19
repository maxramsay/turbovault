use std::sync::Arc;
use turbovault_core::prelude::{MultiVaultManager, ServerConfig, VaultConfig};
use turbovault_rest::{RestConfig, router};

/// Populate a fresh temp vault with standard test fixtures and return the path.
async fn build_temp_vault() -> (tempfile::TempDir, Arc<MultiVaultManager>) {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let vault_path = tmp.path().to_path_buf();

    std::fs::write(vault_path.join("test.md"), "# Test\nHello world\n").unwrap();
    std::fs::create_dir_all(vault_path.join("Daily")).unwrap();
    std::fs::write(
        vault_path.join("Daily/2026-03-19.md"),
        "# Daily Note\nSome content\n",
    )
    .unwrap();
    std::fs::write(vault_path.join("another.md"), "# Another\nMore content\n").unwrap();

    // Create notes with wikilinks for link graph tests
    std::fs::create_dir_all(vault_path.join("notes")).unwrap();
    std::fs::write(
        vault_path.join("notes/A.md"),
        "# A\n\nLinks to [[B]] and [[C]]\n",
    )
    .unwrap();
    std::fs::write(vault_path.join("notes/B.md"), "# B\n\nLinks to [[A]]\n").unwrap();
    std::fs::write(vault_path.join("notes/C.md"), "# C\n\nNo outgoing links\n").unwrap();

    let server_config = ServerConfig::new();
    let manager = MultiVaultManager::empty(server_config).expect("failed to create manager");

    let vault_config = VaultConfig::builder("default", &vault_path)
        .build()
        .expect("failed to build vault config");

    manager.add_vault(vault_config).await.expect("failed to add vault");

    (tmp, Arc::new(manager))
}

/// Create a test REST router backed by a temporary vault directory.
///
/// Returns `(axum::Router, tempfile::TempDir)` — hold onto the `TempDir` so it
/// is not deleted for the duration of the test.
pub async fn test_app(api_token: Option<String>) -> (axum::Router, tempfile::TempDir) {
    let rest_config = RestConfig {
        api_token,
        protected_paths: vec![],
    };
    test_app_with_config(rest_config).await
}

/// Create a test REST router with a fully custom [`RestConfig`].
pub async fn test_app_with_config(config: RestConfig) -> (axum::Router, tempfile::TempDir) {
    let (tmp, manager) = build_temp_vault().await;
    let app = router(manager, config);
    (app, tmp)
}
