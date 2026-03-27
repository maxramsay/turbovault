use std::sync::Arc;
use tempfile::TempDir;
use turbovault_core::prelude::*;
use turbovault_core::versioning::read_version_from_content;
use turbovault_tools::FileTools;
use turbovault_vault::VaultManager;

/// Create a FileTools backed by a temporary vault directory.
async fn setup() -> (TempDir, FileTools) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config = ServerConfig {
        vaults: vec![VaultConfig::builder("test", temp_dir.path())
            .build()
            .unwrap()],
        ..Default::default()
    };

    let manager = VaultManager::new(config).expect("Failed to create vault manager");
    manager
        .initialize()
        .await
        .expect("Failed to initialize vault");

    let tools = FileTools::new(Arc::new(manager));
    (temp_dir, tools)
}

#[tokio::test]
async fn test_write_new_note_gets_version_1() {
    let (_dir, tools) = setup().await;

    let (content, info) = tools
        .write_note_versioned("notes/new.md", "# Hello\nWorld", None, "alice")
        .await
        .expect("write_note_versioned should succeed for new file");

    assert_eq!(info.version, 1);
    assert_eq!(info.history.len(), 1);
    assert_eq!(info.history[0].by, "alice");

    // Read back from disk and verify
    let on_disk = tools.read_file("notes/new.md").await.unwrap();
    let disk_info = read_version_from_content(&on_disk);
    assert_eq!(disk_info.version, 1);
    assert_eq!(disk_info.history.len(), 1);

    // Content returned should match what's on disk
    assert_eq!(content, on_disk);
}

#[tokio::test]
async fn test_update_existing_note_bumps_version() {
    let (_dir, tools) = setup().await;

    // Create version 1
    let (_content_v1, info_v1) = tools
        .write_note_versioned("notes/update.md", "# V1\nOriginal", None, "alice")
        .await
        .unwrap();
    assert_eq!(info_v1.version, 1);

    // Update to version 2
    let (_content_v2, info_v2) = tools
        .write_note_versioned("notes/update.md", "Updated body", Some(1), "bob")
        .await
        .unwrap();
    assert_eq!(info_v2.version, 2);
    assert_eq!(info_v2.history.len(), 2);
    assert_eq!(info_v2.history[0].by, "alice");
    assert_eq!(info_v2.history[1].by, "bob");

    // Read back and verify
    let on_disk = tools.read_file("notes/update.md").await.unwrap();
    let disk_info = read_version_from_content(&on_disk);
    assert_eq!(disk_info.version, 2);
    assert_eq!(disk_info.history.len(), 2);
}

#[tokio::test]
async fn test_stale_update_returns_concurrency_error() {
    let (_dir, tools) = setup().await;

    // Create version 1
    tools
        .write_note_versioned("notes/stale.md", "# Body", None, "alice")
        .await
        .unwrap();

    // Update with wrong expected_version (0 instead of 1)
    let result = tools
        .write_note_versioned("notes/stale.md", "New body", Some(0), "bob")
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("version conflict"),
        "Error should mention version conflict, got: {msg}"
    );
    assert!(
        msg.contains("expected 0") && msg.contains("actual 1"),
        "Error should include expected and actual versions, got: {msg}"
    );
}

#[tokio::test]
async fn test_update_without_version_returns_validation_error() {
    let (_dir, tools) = setup().await;

    // Create version 1
    tools
        .write_note_versioned("notes/noversion.md", "# Body", None, "alice")
        .await
        .unwrap();

    // Update with expected_version=None (should fail)
    let result = tools
        .write_note_versioned("notes/noversion.md", "New body", None, "bob")
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("expected_version"),
        "Error should mention expected_version, got: {msg}"
    );
}
