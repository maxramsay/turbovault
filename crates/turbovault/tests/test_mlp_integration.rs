//! End-to-end integration test for FC Vault MLP:
//! versioning + snapshots working together.

use tempfile::TempDir;
use turbovault_core::snapshot::*;
use turbovault_core::versioning::*;
use turbovault_tools::snapshot_tools::SnapshotTools;

#[tokio::test]
async fn test_full_mlp_flow() {
    let tmp = TempDir::new().unwrap();
    let vault_path = tmp.path().to_path_buf();
    tokio::fs::create_dir_all(&vault_path).await.unwrap();

    // Create two notes - one tagged, one not
    let tagged_content =
        "---\ntitle: FC Roadmap\ntags: [fleet-control, roadmap]\n---\n# FC Roadmap\nOriginal content.";
    let untagged_content = "---\ntitle: Random Note\n---\n# Random\nSome content.";

    // 1. Create tagged note with version 1
    let (versioned_tagged, info1) = apply_version_create(tagged_content, "uisang");
    let tagged_path = vault_path.join("FC Roadmap.md");
    tokio::fs::write(&tagged_path, &versioned_tagged).await.unwrap();
    assert_eq!(info1.version, 1);
    assert_eq!(info1.history.len(), 1);
    assert_eq!(info1.history[0].by, "uisang");

    // Create untagged note
    let (versioned_untagged, _) = apply_version_create(untagged_content, "uisang");
    tokio::fs::write(vault_path.join("Random Note.md"), &versioned_untagged)
        .await
        .unwrap();

    // 2. Update tagged note -> version 2
    let current = tokio::fs::read_to_string(&tagged_path).await.unwrap();
    let (updated, info2) =
        apply_version_update(&current, "# FC Roadmap\nUpdated content.", 1, "yu-sin").unwrap();
    tokio::fs::write(&tagged_path, &updated).await.unwrap();
    assert_eq!(info2.version, 2);
    assert_eq!(info2.history.len(), 2);
    assert_eq!(info2.history[0].by, "uisang");
    assert_eq!(info2.history[1].by, "yu-sin");

    // 3. Stale update should fail
    let stale_result = apply_version_update(&updated, "# FC Roadmap\nStale.", 1, "bad-actor");
    assert!(stale_result.is_err());
    let conflict = stale_result.unwrap_err();
    assert_eq!(conflict.expected, 1);
    assert_eq!(conflict.actual, 2);

    // 4. Create full vault snapshot
    let snapshot_target = tmp.path().join("snapshots");
    let tools = SnapshotTools::new(vault_path.clone());
    let full_manifest = tools
        .create_snapshot(
            &SnapshotSelection::All,
            snapshot_target.to_str().unwrap(),
            "default",
            "uisang",
        )
        .await
        .unwrap();
    assert_eq!(full_manifest.note_count, 2);
    assert_eq!(full_manifest.format_version, 1);
    assert!(full_manifest.snapshot_id.ends_with("-full-vault"));

    // 5. Create tag-filtered snapshot
    let filtered_manifest = tools
        .create_snapshot(
            &SnapshotSelection::Tags {
                tags: vec!["fleet-control".into()],
            },
            snapshot_target.to_str().unwrap(),
            "default",
            "uisang",
        )
        .await
        .unwrap();
    assert_eq!(filtered_manifest.note_count, 1);
    assert_eq!(filtered_manifest.notes[0].version, 2); // Should have current version

    // 6. Delete the tagged note
    tokio::fs::remove_file(&tagged_path).await.unwrap();
    assert!(!tagged_path.exists());

    // 7. Restore from full snapshot (staging mode)
    let archive_path = snapshot_target.join(format!("{}.tar.gz", full_manifest.snapshot_id));
    let restored = tools
        .restore_snapshot(&archive_path, &RestoreMode::Staging)
        .await
        .unwrap();
    assert_eq!(restored.note_count, 2);

    // 8. Verify restored note has correct version and history
    let restore_dir = vault_path.join("_restore").join(&full_manifest.snapshot_id);
    let restored_content = tokio::fs::read_to_string(restore_dir.join("FC Roadmap.md"))
        .await
        .unwrap();
    let restored_info = read_version_from_content(&restored_content);
    assert_eq!(restored_info.version, 2);
    assert_eq!(restored_info.history.len(), 2);
    assert_eq!(restored_info.history[0].by, "uisang");
    assert_eq!(restored_info.history[1].by, "yu-sin");
}

#[tokio::test]
async fn test_snapshot_list_and_delete() {
    let tmp = TempDir::new().unwrap();
    let vault_path = tmp.path().to_path_buf();

    // Create a note
    let content = "---\ntitle: Test\ntags: [test]\n---\n# Test";
    let (versioned, _) = apply_version_create(content, "test");
    tokio::fs::write(vault_path.join("test.md"), &versioned)
        .await
        .unwrap();

    let snapshot_target = tmp.path().join("snapshots");
    let tools = SnapshotTools::new(vault_path);

    // Create two snapshots with different selections so IDs differ
    let m1 = tools
        .create_snapshot(
            &SnapshotSelection::All,
            snapshot_target.to_str().unwrap(),
            "default",
            "test1",
        )
        .await
        .unwrap();
    let m2 = tools
        .create_snapshot(
            &SnapshotSelection::Tags {
                tags: vec!["test".into()],
            },
            snapshot_target.to_str().unwrap(),
            "default",
            "test2",
        )
        .await
        .unwrap();

    // List should show 2
    let list = tools
        .list_snapshots(snapshot_target.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(list.len(), 2);

    // Delete first, list should show 1
    tools
        .delete_snapshot(snapshot_target.to_str().unwrap(), &m1.snapshot_id)
        .await
        .unwrap();
    let list2 = tools
        .list_snapshots(snapshot_target.to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(list2.len(), 1);
    assert_eq!(list2[0].snapshot_id, m2.snapshot_id);
}
