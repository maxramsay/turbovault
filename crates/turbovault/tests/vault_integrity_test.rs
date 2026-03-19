mod helpers;

use helpers::TestVault;
use std::path::PathBuf;
use turbovault_batch::{BatchExecutor, BatchOperation};
use turbovault_core::models::LinkType;
use turbovault_tools::WriteMode;
use turbovault_vault::compute_hash;

// ---------------------------------------------------------------------------
// Frontmatter tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn frontmatter_complex_round_trip() {
    let vault = TestVault::new().await;

    let content = r#"---
title: "Complex Note"
tags:
  - rust
  - testing
  - obsidian
nested:
  key: value
  list:
    - one
    - two
date: 2025-01-15
draft: false
---

# Body

Some body content here.
"#;

    vault.write("complex.md", content).await;
    let read_back = vault.read("complex.md").await;
    assert_eq!(read_back, content, "Content should be byte-exact after round trip");

    let parsed = vault.parse("complex.md", &read_back);

    let fm = parsed.frontmatter.as_ref().expect("Should have frontmatter");
    assert_eq!(
        fm.data.get("title").and_then(|v| v.as_str()),
        Some("Complex Note")
    );

    let tags = fm.tags();
    assert_eq!(tags, vec!["rust", "testing", "obsidian"]);

    let nested = fm.data.get("nested").expect("Should have nested key");
    assert_eq!(
        nested.get("key").and_then(|v| v.as_str()),
        Some("value")
    );
}

#[tokio::test]
async fn frontmatter_malformed_no_corruption() {
    let vault = TestVault::new().await;

    // Missing closing --- delimiter
    let content = "---\ntitle: Broken\ntags:\n  - a\nSome body that looks like frontmatter ran away.\n";

    vault.write("malformed.md", content).await;
    let read_back = vault.read("malformed.md").await;
    assert_eq!(read_back, content, "Malformed frontmatter content must be preserved exactly");
}

#[tokio::test]
async fn frontmatter_unicode_preservation() {
    let vault = TestVault::new().await;

    let content = "---\ntitle: \"\u{d55c}\u{ad6d}\u{c5b4} \u{30c6}\u{30b9}\u{30c8} \u{1f680}\"\ntags:\n  - \"\u{4e2d}\u{6587}\"\n  - \"\u{e0b8}\u{e44}\u{e17}\u{e22}\"\n---\n\n# \u{c81c}\u{baa9}\n\nBody with \u{1f4a1} emoji and caf\u{e9} and na\u{ef}ve.\n";

    vault.write("unicode.md", content).await;
    let read_back = vault.read("unicode.md").await;
    assert_eq!(read_back, content, "Unicode content must survive round trip");

    let parsed = vault.parse("unicode.md", &read_back);
    let fm = parsed.frontmatter.as_ref().expect("Should have frontmatter");
    let title = fm.data.get("title").and_then(|v| v.as_str()).unwrap();
    assert!(title.contains("\u{d55c}\u{ad6d}\u{c5b4}"), "Korean text must be preserved");
    assert!(title.contains("\u{1f680}"), "Emoji must be preserved");
}

// ---------------------------------------------------------------------------
// Wikilink tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wikilink_all_variants() {
    let vault = TestVault::new().await;

    let content = r#"# Links

- [[simple]]
- [[folder/path]]
- [[note|alias]]
- [[note#heading]]
- [[note#^blockref]]
- ![[embed]]
- ![[image.png]]
"#;

    vault.write("links.md", content).await;
    let read_back = vault.read("links.md").await;
    let parsed = vault.parse("links.md", &read_back);

    let links = &parsed.links;
    assert!(links.len() >= 7, "Should parse at least 7 links, got {}", links.len());

    // Check that we have WikiLink and Embed types
    let wikilinks: Vec<_> = links.iter().filter(|l| matches!(l.type_, LinkType::WikiLink)).collect();
    let embeds: Vec<_> = links.iter().filter(|l| matches!(l.type_, LinkType::Embed)).collect();

    assert!(!wikilinks.is_empty(), "Should have WikiLink type links");
    assert!(!embeds.is_empty(), "Should have Embed type links");

    // Verify specific targets exist
    let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();
    assert!(targets.iter().any(|t| *t == "simple"), "Should have 'simple' target");
    assert!(targets.iter().any(|t| t.contains("folder/path")), "Should have 'folder/path' target");
}

#[tokio::test]
async fn wikilink_backlink_resolution() {
    let vault = TestVault::new().await;

    vault.write("alpha.md", "# Alpha\n\nLinks to [[beta]].\n").await;
    vault.write("beta.md", "# Beta\n\nLinks to [[alpha]].\n").await;

    vault.reinitialize().await;

    let backlinks_to_beta = vault.manager
        .get_backlinks(&PathBuf::from("beta.md"))
        .await
        .expect("get_backlinks should succeed");

    assert!(
        backlinks_to_beta.iter().any(|p| p.to_string_lossy().contains("alpha")),
        "alpha.md should be a backlink to beta.md, got: {:?}",
        backlinks_to_beta
    );

    let backlinks_to_alpha = vault.manager
        .get_backlinks(&PathBuf::from("alpha.md"))
        .await
        .expect("get_backlinks should succeed");

    assert!(
        backlinks_to_alpha.iter().any(|p| p.to_string_lossy().contains("beta")),
        "beta.md should be a backlink to alpha.md, got: {:?}",
        backlinks_to_alpha
    );
}

#[tokio::test]
async fn wikilink_in_code_blocks_ignored() {
    let vault = TestVault::new().await;

    let content = r#"# Note

A real link: [[real-target]]

```
[[code-block-link]]
```

Inline code: `[[inline-code-link]]`

Another real link: [[also-real]]
"#;

    vault.write("codelinks.md", content).await;
    let read_back = vault.read("codelinks.md").await;
    let parsed = vault.parse("codelinks.md", &read_back);

    let targets: Vec<&str> = parsed.links.iter().map(|l| l.target.as_str()).collect();

    assert!(targets.contains(&"real-target"), "Should parse real-target link");
    assert!(targets.contains(&"also-real"), "Should parse also-real link");
    assert!(!targets.contains(&"code-block-link"), "Should NOT parse link inside code block");
    assert!(!targets.contains(&"inline-code-link"), "Should NOT parse link inside inline code");
}

// ---------------------------------------------------------------------------
// Write mode tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_append_preserves_existing() {
    let vault = TestVault::new().await;

    vault.write("append.md", "Original content.\n").await;

    let ft = vault.file_tools();
    ft.write_file_with_mode("append.md", "Appended content.\n", WriteMode::Append)
        .await
        .expect("Append should succeed");

    let read_back = vault.read("append.md").await;
    assert!(read_back.contains("Original content."), "Original content must be preserved");
    assert!(read_back.contains("Appended content."), "Appended content must be present");

    // Original should come before appended
    let orig_pos = read_back.find("Original content.").unwrap();
    let append_pos = read_back.find("Appended content.").unwrap();
    assert!(orig_pos < append_pos, "Original content should come before appended content");
}

#[tokio::test]
async fn write_prepend_after_frontmatter() {
    let vault = TestVault::new().await;

    let original = "---\ntitle: Test\n---\n\nOriginal body.\n";
    vault.write("prepend.md", original).await;

    let ft = vault.file_tools();
    ft.write_file_with_mode("prepend.md", "Prepended line.\n", WriteMode::Prepend)
        .await
        .expect("Prepend should succeed");

    let read_back = vault.read("prepend.md").await;

    // Frontmatter must still be first
    assert!(read_back.starts_with("---"), "Frontmatter must remain at the start");
    assert!(read_back.contains("title: Test"), "Frontmatter content must be preserved");
    assert!(read_back.contains("Prepended line."), "Prepended content must be present");
    assert!(read_back.contains("Original body."), "Original body must be preserved");

    // Prepended content should appear before original body
    let prepend_pos = read_back.find("Prepended line.").unwrap();
    let body_pos = read_back.find("Original body.").unwrap();
    assert!(prepend_pos < body_pos, "Prepended content should come before original body");
}

#[tokio::test]
async fn write_overwrite_replaces_entirely() {
    let vault = TestVault::new().await;

    vault.write("overwrite.md", "Old content that should vanish.\n").await;
    vault.write("overwrite.md", "Brand new content.\n").await;

    let read_back = vault.read("overwrite.md").await;
    assert_eq!(read_back, "Brand new content.\n");
    assert!(!read_back.contains("Old content"), "Old content must be gone after overwrite");
}

// ---------------------------------------------------------------------------
// Edit tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edit_search_replace_basic() {
    let vault = TestVault::new().await;

    vault.write("edit.md", "Hello world.\n").await;

    let edits = "<<<<<<< SEARCH\nHello world.\n=======\nHello Rust.\n>>>>>>> REPLACE";

    let result = vault.manager
        .edit_file(&PathBuf::from("edit.md"), edits, None, false)
        .await
        .expect("Edit should succeed");

    assert!(result.success, "Edit should report success");
    assert_eq!(result.blocks_applied, 1);

    let read_back = vault.read("edit.md").await;
    assert!(read_back.contains("Hello Rust."), "Content should be replaced");
    assert!(!read_back.contains("Hello world."), "Old content should be gone");
}

#[tokio::test]
async fn edit_two_sequential_edits() {
    let vault = TestVault::new().await;

    vault.write("seq.md", "Line A.\nLine B.\nLine C.\n").await;

    // First edit
    let edit1 = "<<<<<<< SEARCH\nLine A.\n=======\nLine Alpha.\n>>>>>>> REPLACE";
    let r1 = vault.manager
        .edit_file(&PathBuf::from("seq.md"), edit1, None, false)
        .await
        .expect("First edit should succeed");
    assert!(r1.success);

    // Second edit using the new hash from first edit
    let edit2 = "<<<<<<< SEARCH\nLine B.\n=======\nLine Beta.\n>>>>>>> REPLACE";
    let r2 = vault.manager
        .edit_file(&PathBuf::from("seq.md"), edit2, Some(&r1.new_hash), false)
        .await
        .expect("Second edit should succeed");
    assert!(r2.success);

    let read_back = vault.read("seq.md").await;
    assert!(read_back.contains("Line Alpha."), "First edit should be applied");
    assert!(read_back.contains("Line Beta."), "Second edit should be applied");
    assert!(read_back.contains("Line C."), "Unedited line should remain");
}

#[tokio::test]
async fn edit_stale_hash_rejected() {
    let vault = TestVault::new().await;

    vault.write("stale.md", "Original.\n").await;

    let stale_hash = "0000000000000000000000000000000000000000000000000000000000000000";
    let edits = "<<<<<<< SEARCH\nOriginal.\n=======\nModified.\n>>>>>>> REPLACE";

    let result = vault.manager
        .edit_file(&PathBuf::from("stale.md"), edits, Some(stale_hash), false)
        .await;

    assert!(result.is_err(), "Edit with stale hash should be rejected");

    // Content should be unchanged
    let read_back = vault.read("stale.md").await;
    assert_eq!(read_back, "Original.\n", "Content must not change on rejected edit");
}

#[tokio::test]
async fn edit_partial_operation_detectable() {
    let vault = TestVault::new().await;

    vault.write("partial.md", "AAA\nBBB\nCCC\n").await;

    // First edit succeeds
    let edit1 = "<<<<<<< SEARCH\nAAA\n=======\nXXX\n>>>>>>> REPLACE";
    let r1 = vault.manager
        .edit_file(&PathBuf::from("partial.md"), edit1, None, false)
        .await
        .expect("First edit should succeed");
    assert!(r1.success);

    // Read current content and compute hash to prove state is detectable
    let after_first = vault.read("partial.md").await;
    let hash_after_first = compute_hash(&after_first);
    assert_eq!(hash_after_first, r1.new_hash, "Hash should match after first edit");

    // The content is now in a known intermediate state
    assert!(after_first.contains("XXX"), "First edit applied");
    assert!(after_first.contains("BBB"), "Second target still original");

    // Second edit with correct hash proves we can track partial state
    let edit2 = "<<<<<<< SEARCH\nBBB\n=======\nYYY\n>>>>>>> REPLACE";
    let r2 = vault.manager
        .edit_file(&PathBuf::from("partial.md"), edit2, Some(&hash_after_first), false)
        .await
        .expect("Second edit with correct hash should succeed");
    assert!(r2.success);

    let final_content = vault.read("partial.md").await;
    assert!(final_content.contains("XXX"));
    assert!(final_content.contains("YYY"));
    assert!(final_content.contains("CCC"));
}

// ---------------------------------------------------------------------------
// Batch tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn batch_all_succeed() {
    let vault = TestVault::new().await;
    let temp_dir = tempfile::TempDir::new().unwrap();

    let executor = BatchExecutor::new(vault.manager.clone(), temp_dir.path().to_path_buf());

    let ops = vec![
        BatchOperation::CreateNote {
            path: "batch1.md".to_string(),
            content: "# Batch 1\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "batch2.md".to_string(),
            content: "# Batch 2\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "batch3.md".to_string(),
            content: "# Batch 3\n".to_string(),
        },
    ];

    let result = executor.execute(ops).await.expect("Batch should execute");
    assert!(result.success, "All operations should succeed");
    assert_eq!(result.executed, 3);
    assert_eq!(result.total, 3);
    assert!(result.failed_at.is_none());

    // Verify all files exist
    let c1 = vault.read("batch1.md").await;
    let c2 = vault.read("batch2.md").await;
    let c3 = vault.read("batch3.md").await;
    assert!(c1.contains("Batch 1"));
    assert!(c2.contains("Batch 2"));
    assert!(c3.contains("Batch 3"));
}

#[tokio::test]
async fn batch_poisoned_operation_stops() {
    let vault = TestVault::new().await;
    let temp_dir = tempfile::TempDir::new().unwrap();

    let executor = BatchExecutor::new(vault.manager.clone(), temp_dir.path().to_path_buf());

    let ops = vec![
        BatchOperation::CreateNote {
            path: "good_first.md".to_string(),
            content: "Safe file.\n".to_string(),
        },
        // Path traversal attack
        BatchOperation::CreateNote {
            path: "../../etc/evil.md".to_string(),
            content: "Malicious.\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "good_third.md".to_string(),
            content: "Should not exist.\n".to_string(),
        },
    ];

    let result = executor.execute(ops).await.expect("Batch should return result");
    assert!(!result.success, "Batch should fail on path traversal");

    // First file should have been created before the failure
    let first = vault.read("good_first.md").await;
    assert!(first.contains("Safe file"), "First file should exist");

    // Third file should NOT exist since batch stopped at second
    let third_result = vault.file_tools().read_file("good_third.md").await;
    assert!(third_result.is_err(), "Third file should not exist after batch failure");
}

#[tokio::test]
async fn batch_partial_state_inspectable() {
    let vault = TestVault::new().await;
    let temp_dir = tempfile::TempDir::new().unwrap();

    let executor = BatchExecutor::new(vault.manager.clone(), temp_dir.path().to_path_buf());

    // Create a file, then try to delete a nonexistent file (will fail), leaving partial state
    vault.write("existing.md", "Existing content.\n").await;

    let ops = vec![
        BatchOperation::WriteNote {
            path: "existing.md".to_string(),
            content: "Updated content.\n".to_string(),
        },
        BatchOperation::DeleteNote {
            path: "nonexistent_file_that_does_not_exist.md".to_string(),
        },
    ];

    let result = executor.execute(ops).await.expect("Batch should return result");
    assert!(!result.success, "Batch should fail on missing file delete");
    assert_eq!(result.executed, 1, "One operation should have executed before failure");
    assert_eq!(result.failed_at, Some(1), "Failure should be at index 1");

    // The first operation's write should be committed - state is valid, not corrupted
    let content = vault.read("existing.md").await;
    assert_eq!(content, "Updated content.\n", "Partial state should be valid (first op committed)");
}

// ---------------------------------------------------------------------------
// Heading (patch_note) tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_heading_append_correct_section() {
    let vault = TestVault::new().await;

    let content = "# Top\n\nIntro text.\n\n## Section A\n\nA content.\n\n## Section B\n\nB content.\n";
    vault.write("sections.md", content).await;

    // Use edit to insert content under Section A
    let edits = "<<<<<<< SEARCH\nA content.\n=======\nA content.\n\nAppended under A.\n>>>>>>> REPLACE";

    let result = vault.manager
        .edit_file(&PathBuf::from("sections.md"), edits, None, false)
        .await
        .expect("Edit should succeed");
    assert!(result.success);

    let read_back = vault.read("sections.md").await;
    assert!(read_back.contains("Appended under A."), "Content should appear under Section A");

    // Section B should be untouched
    let section_b_pos = read_back.find("## Section B").unwrap();
    let appended_pos = read_back.find("Appended under A.").unwrap();
    assert!(appended_pos < section_b_pos, "Appended content should be before Section B");
}

#[tokio::test]
async fn patch_heading_in_code_block_ignored() {
    let vault = TestVault::new().await;

    let content = "# Real Heading\n\nSome text.\n\n```markdown\n## Fake Heading In Code\n```\n\n## Another Real Heading\n\nMore text.\n";
    vault.write("fake_headings.md", content).await;
    let read_back = vault.read("fake_headings.md").await;
    let parsed = vault.parse("fake_headings.md", &read_back);

    let heading_texts: Vec<&str> = parsed.headings.iter().map(|h| h.text.as_str()).collect();

    assert!(heading_texts.contains(&"Real Heading"), "Should parse 'Real Heading'");
    assert!(heading_texts.contains(&"Another Real Heading"), "Should parse 'Another Real Heading'");
    assert!(
        !heading_texts.contains(&"Fake Heading In Code"),
        "Should NOT parse heading inside code block, got: {:?}",
        heading_texts
    );
}

#[tokio::test]
async fn patch_heading_duplicate_names() {
    let vault = TestVault::new().await;

    let content = "# Title\n\n## Notes\n\nFirst notes section.\n\n## Notes\n\nSecond notes section.\n";
    vault.write("dupes.md", content).await;
    let read_back = vault.read("dupes.md").await;
    let parsed = vault.parse("dupes.md", &read_back);

    let notes_headings: Vec<_> = parsed.headings.iter().filter(|h| h.text == "Notes").collect();
    assert_eq!(
        notes_headings.len(),
        2,
        "Both duplicate headings should be parsed, got {}",
        notes_headings.len()
    );
}

// ---------------------------------------------------------------------------
// Move/rename tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn move_preserves_content() {
    let vault = TestVault::new().await;

    let content = "# Movable\n\nContent to preserve.\n";
    vault.write("original.md", content).await;

    let ft = vault.file_tools();
    ft.move_file("original.md", "subfolder/moved.md")
        .await
        .expect("Move should succeed");

    // New path should have the content
    let moved_content = vault.read("subfolder/moved.md").await;
    assert_eq!(moved_content, content, "Content must be preserved after move");

    // Old path should be gone
    let old_result = ft.read_file("original.md").await;
    assert!(old_result.is_err(), "Original path should not exist after move");
}

#[tokio::test]
async fn move_graph_detects_broken_links() {
    let vault = TestVault::new().await;

    vault.write("source.md", "# Source\n\nSee [[target]].\n").await;
    vault.write("target.md", "# Target\n\nTarget content.\n").await;

    vault.reinitialize().await;

    // Verify backlink exists before move
    let backlinks_before = vault.manager
        .get_backlinks(&PathBuf::from("target.md"))
        .await
        .expect("get_backlinks should work");
    assert!(
        backlinks_before.iter().any(|p| p.to_string_lossy().contains("source")),
        "source.md should link to target.md before move"
    );

    // Move target without updating links in source
    let ft = vault.file_tools();
    ft.move_file("target.md", "moved_target.md")
        .await
        .expect("Move should succeed");

    vault.reinitialize().await;

    // The link text in source.md still says [[target]], which is now broken
    let source_content = vault.read("source.md").await;
    assert!(
        source_content.contains("[[target]]"),
        "Link text should be unchanged after move (link is now broken)"
    );
}

#[tokio::test]
async fn move_interrupted_link_update() {
    let vault = TestVault::new().await;

    vault.write("linker.md", "# Linker\n\nSee [[linked-note]].\n").await;
    vault.write("linked-note.md", "# Linked\n\nContent.\n").await;

    vault.reinitialize().await;

    // Move the file but simulate interruption by NOT updating the link in linker.md
    let ft = vault.file_tools();
    ft.move_file("linked-note.md", "new-location.md")
        .await
        .expect("Move should succeed");

    vault.reinitialize().await;

    // linker.md still references [[linked-note]] but that file no longer exists
    let linker_content = vault.read("linker.md").await;
    let parsed = vault.parse("linker.md", &linker_content);

    let broken_link = parsed.links.iter().find(|l| l.target == "linked-note");
    assert!(
        broken_link.is_some(),
        "The link to 'linked-note' should still exist in the parse (it's broken but detectable)"
    );

    // The moved file should not be reachable at old path
    let old_read = ft.read_file("linked-note.md").await;
    assert!(old_read.is_err(), "Old path should be gone");

    // New location should have the content
    let new_content = vault.read("new-location.md").await;
    assert!(new_content.contains("Linked"), "Content should be at new location");
}

#[tokio::test]
async fn move_then_manual_link_update() {
    let vault = TestVault::new().await;

    vault.write("referrer.md", "# Referrer\n\nSee [[old-name]].\n").await;
    vault.write("old-name.md", "# Note\n\nContent here.\n").await;

    vault.reinitialize().await;

    // Move the file
    let ft = vault.file_tools();
    ft.move_file("old-name.md", "new-name.md")
        .await
        .expect("Move should succeed");

    // Manually update the link in referrer.md
    let edits = "<<<<<<< SEARCH\n[[old-name]]\n=======\n[[new-name]]\n>>>>>>> REPLACE";
    let result = vault.manager
        .edit_file(&PathBuf::from("referrer.md"), edits, None, false)
        .await
        .expect("Link update edit should succeed");
    assert!(result.success);

    vault.reinitialize().await;

    // Verify referrer now points to new-name
    let referrer_content = vault.read("referrer.md").await;
    assert!(referrer_content.contains("[[new-name]]"), "Link should be updated");
    assert!(!referrer_content.contains("[[old-name]]"), "Old link should be gone");

    // Verify backlinks resolve correctly
    let backlinks = vault.manager
        .get_backlinks(&PathBuf::from("new-name.md"))
        .await
        .expect("get_backlinks should work");
    assert!(
        backlinks.iter().any(|p| p.to_string_lossy().contains("referrer")),
        "referrer.md should be a backlink to new-name.md, got: {:?}",
        backlinks
    );
}
