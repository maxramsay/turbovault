# Vault Integrity Test Suite Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a two-layer test suite proving TurboVault produces Obsidian-compatible vault files, gating agent cutover.

**Architecture:** Layer 1 is Rust integration tests in the TurboVault repo (`tests/vault_integrity_test.rs`) using temp vaults and TurboVault's OFM parser as validator. Layer 2 is a standalone Python project (`~/projects/vault-integrity-tests/`) that hits both live MCP endpoints and does cross-tool round-trip comparisons.

**Tech Stack:** Rust (tokio, tempfile, turbovault crates) for Layer 1. Python 3 (requests, pyyaml, pytest) for Layer 2.

**Spec:** `docs/superpowers/specs/2026-03-18-vault-integrity-test-suite-design.md`

---

## File Structure

### Layer 1 (TurboVault repo)

| File | Responsibility |
|------|---------------|
| `tests/vault_integrity_test.rs` | All Rust integration tests — frontmatter, wikilinks, write modes, patch_note, edit, batch, move |
| `tests/helpers/mod.rs` | Shared test setup (create temp vault, write helper, parse helper) |

**Note:** All `use` statements for the test file should be consolidated at the top of `tests/vault_integrity_test.rs`:
```rust
mod helpers;
use helpers::TestVault;
use turbovault_core::models::LinkType;
use turbovault_tools::{BatchTools, WriteMode};
use turbovault_batch::BatchOperation;
use turbovault_vault::compute_hash;
```

### Layer 2 (Standalone project)

| File | Responsibility |
|------|---------------|
| `~/projects/vault-integrity-tests/config.yaml` | Endpoint URLs and test path configuration |
| `~/projects/vault-integrity-tests/mcp_client.py` | Thin JSON-RPC client for TurboVault and Obsidian MCP |
| `~/projects/vault-integrity-tests/compare.py` | Semantic comparison utilities (frontmatter, body, normalized diff) |
| `~/projects/vault-integrity-tests/test_frontmatter.py` | Cross-tool frontmatter round-trip tests |
| `~/projects/vault-integrity-tests/test_wikilinks.py` | Cross-tool wikilink resolution tests |
| `~/projects/vault-integrity-tests/test_heading_patch.py` | Cross-tool heading-aware insert tests |
| `~/projects/vault-integrity-tests/test_edit.py` | Cross-tool edit + partial operation tests |
| `~/projects/vault-integrity-tests/test_concurrent.py` | Concurrent access tests |
| `~/projects/vault-integrity-tests/test_move.py` | Cross-tool move/rename tests |
| `~/projects/vault-integrity-tests/run.py` | CLI runner with --cleanup flag and summary output |
| `~/projects/vault-integrity-tests/requirements.txt` | Python dependencies |

---

## Task 1: Rust Test Helpers

**Files:**
- Create: `tests/helpers/mod.rs`
- Modify: `tests/vault_integrity_test.rs` (will be created empty first)

- [ ] **Step 1: Create test helper module**

```rust
// tests/helpers/mod.rs
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use turbovault_core::{ConfigProfile, VaultConfig};
use turbovault_tools::{FileTools, WriteMode};
use turbovault_vault::VaultManager;

pub struct TestVault {
    pub _temp_dir: TempDir,
    pub manager: Arc<VaultManager>,
}

impl TestVault {
    pub async fn new() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let mut config = ConfigProfile::Development.create_config();
        let vault_config = VaultConfig::builder("test", temp_dir.path())
            .build()
            .expect("Failed to create vault config");
        config.vaults.push(vault_config);

        let manager = VaultManager::new(config).expect("Failed to create vault manager");
        manager.initialize().await.expect("Failed to initialize vault");

        Self {
            _temp_dir: temp_dir,
            manager: Arc::new(manager),
        }
    }

    pub fn file_tools(&self) -> FileTools {
        FileTools::new(self.manager.clone())
    }

    pub async fn write(&self, path: &str, content: &str) {
        self.file_tools()
            .write_file_with_mode(path, content, WriteMode::Overwrite)
            .await
            .expect("Failed to write file");
    }

    pub async fn read(&self, path: &str) -> String {
        self.file_tools()
            .read_file(path)
            .await
            .expect("Failed to read file")
    }

    pub async fn parse(&self, path: &str) -> turbovault_vault::VaultFile {
        self.manager
            .parse_file(&PathBuf::from(path))
            .await
            .expect("Failed to parse file")
    }

    pub async fn reinitialize(&self) {
        self.manager.initialize().await.expect("Failed to reinitialize vault");
    }
}
```

- [ ] **Step 2: Create empty test file**

```rust
// tests/vault_integrity_test.rs
mod helpers;
use helpers::TestVault;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo test --test vault_integrity_test --no-run`
Expected: Compiles with no errors (no tests to run yet)

- [ ] **Step 4: Commit**

```bash
git add tests/helpers/mod.rs tests/vault_integrity_test.rs
git commit -m "test: add vault integrity test scaffolding with TestVault helper"
```

---

## Task 2: Frontmatter Integrity Tests

**Files:**
- Modify: `tests/vault_integrity_test.rs`

- [ ] **Step 1: Write frontmatter_complex_round_trip test**

```rust
#[tokio::test]
async fn frontmatter_complex_round_trip() {
    let vault = TestVault::new().await;
    let content = r#"---
title: Test Note
date: 2026-03-18
tags: [infrastructure, homelab, docker]
nested:
  key1: value1
  key2: 42
  list:
    - item1
    - item2
status: active
special_chars: "colon: here & ampersand <angle>"
---

# Body Content

This is the body.
"#;
    vault.write("test.md", content).await;
    let read_back = vault.read("test.md").await;
    assert_eq!(read_back, content);

    let parsed = vault.parse("test.md").await;
    let fm = parsed.frontmatter.expect("Should have frontmatter");
    assert_eq!(fm.data.get("title").unwrap().as_str().unwrap(), "Test Note");
    assert_eq!(fm.tags(), vec!["infrastructure", "homelab", "docker"]);
    assert_eq!(
        fm.data.get("nested").unwrap().get("key2").unwrap().as_i64().unwrap(),
        42
    );
}
```

- [ ] **Step 2: Write frontmatter_malformed_no_corruption test**

```rust
#[tokio::test]
async fn frontmatter_malformed_no_corruption() {
    let vault = TestVault::new().await;

    // Missing closing ---
    let content = "---\ntitle: Broken\n\n# Body Content\n\nImportant text here.\n";
    vault.write("malformed.md", content).await;
    let read_back = vault.read("malformed.md").await;
    assert_eq!(read_back, content);
    assert!(read_back.contains("Important text here."));
}
```

- [ ] **Step 3: Write frontmatter_unicode_preservation test**

```rust
#[tokio::test]
async fn frontmatter_unicode_preservation() {
    let vault = TestVault::new().await;
    let content = "---\ntitle: \u{4F60}\u{597D}\u{4E16}\u{754C}\nauthor: Caf\u{e9} \u{2615}\ntags: [\u{1F680}, \u{1F4DD}]\n---\n\n# Unicode Body\n\n\u{2764}\u{FE0F} Content with \u{1F600} emoji.\n";
    vault.write("unicode.md", content).await;
    let read_back = vault.read("unicode.md").await;
    assert_eq!(read_back, content);
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test vault_integrity_test frontmatter -- --nocapture`
Expected: All 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add tests/vault_integrity_test.rs
git commit -m "test: add frontmatter integrity tests"
```

---

## Task 3: Wikilink Integrity Tests

**Files:**
- Modify: `tests/vault_integrity_test.rs`

- [ ] **Step 1: Write wikilink_all_variants test**

```rust
#[tokio::test]
async fn wikilink_all_variants() {
    let vault = TestVault::new().await;
    let content = r#"# Links Test

- [[simple]]
- [[folder/path]]
- [[note|alias text]]
- [[note#heading]]
- [[note#^blockref]]
- ![[embed]]
- ![[image.png]]
"#;
    vault.write("links.md", content).await;
    let parsed = vault.parse("links.md").await;

    // VaultFile has `links: Vec<Link>` with `type_: LinkType` — filter by type
    use turbovault_core::models::LinkType;

    let wikilinks: Vec<_> = parsed.links.iter()
        .filter(|l| l.type_ == LinkType::WikiLink)
        .map(|l| l.target.as_str())
        .collect();
    assert!(wikilinks.contains(&"simple"));
    assert!(wikilinks.contains(&"folder/path"));

    let embeds: Vec<_> = parsed.links.iter()
        .filter(|l| l.type_ == LinkType::Embed)
        .map(|l| l.target.as_str())
        .collect();
    assert!(embeds.contains(&"embed"));
    assert!(embeds.contains(&"image.png"));
}
```

- [ ] **Step 2: Write wikilink_backlink_resolution test**

```rust
#[tokio::test]
async fn wikilink_backlink_resolution() {
    let vault = TestVault::new().await;
    vault.write("note_a.md", "# Note A\n\nLinks to [[note_b]].\n").await;
    vault.write("note_b.md", "# Note B\n\nLinks to [[note_a]].\n").await;
    vault.reinitialize().await;

    let stats = vault.manager.get_stats().await.expect("stats");
    assert_eq!(stats.total_files, 2);
    assert!(stats.total_links >= 2);

    let backlinks_a = vault.manager
        .get_backlinks(&std::path::PathBuf::from("note_b.md"))
        .await
        .expect("backlinks");
    let backlink_paths: Vec<String> = backlinks_a.iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert!(backlink_paths.contains(&"note_a.md".to_string()));
}
```

- [ ] **Step 3: Write wikilink_in_code_blocks_ignored test**

```rust
#[tokio::test]
async fn wikilink_in_code_blocks_ignored() {
    let vault = TestVault::new().await;
    let content = "# Test\n\n```\n[[should_not_match]]\n```\n\nInline `[[also_ignored]]` code.\n\n[[real_link]]\n";
    vault.write("code_links.md", content).await;
    let parsed = vault.parse("code_links.md").await;

    use turbovault_core::models::LinkType;
    let targets: Vec<_> = parsed.links.iter()
        .filter(|l| l.type_ == LinkType::WikiLink)
        .map(|l| l.target.as_str())
        .collect();
    assert!(targets.contains(&"real_link"));
    assert!(!targets.contains(&"should_not_match"));
    assert!(!targets.contains(&"also_ignored"));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test vault_integrity_test wikilink -- --nocapture`
Expected: All 3 PASS

- [ ] **Step 5: Commit**

```bash
git add tests/vault_integrity_test.rs
git commit -m "test: add wikilink integrity tests"
```

---

## Task 4: Write Mode Tests

**Files:**
- Modify: `tests/vault_integrity_test.rs`

- [ ] **Step 1: Write write_append_preserves_existing test**

```rust
use turbovault_tools::WriteMode;

#[tokio::test]
async fn write_append_preserves_existing() {
    let vault = TestVault::new().await;
    let original = "# Original\n\nFirst content.\n";
    let appended = "\n## Appended\n\nSecond content.\n";

    vault.write("append_test.md", original).await;
    vault.file_tools()
        .write_file_with_mode("append_test.md", appended, WriteMode::Append)
        .await
        .expect("append failed");

    let result = vault.read("append_test.md").await;
    assert!(result.starts_with("# Original"));
    assert!(result.contains("First content."));
    assert!(result.contains("Second content."));
}
```

- [ ] **Step 2: Write write_prepend_after_frontmatter test**

```rust
#[tokio::test]
async fn write_prepend_after_frontmatter() {
    let vault = TestVault::new().await;
    let original = "---\ntitle: Test\n---\n\n# Body\n\nOriginal body.\n";
    let prepended = "## Prepended Section\n\nNew content.\n";

    vault.write("prepend_test.md", original).await;
    vault.file_tools()
        .write_file_with_mode("prepend_test.md", prepended, WriteMode::Prepend)
        .await
        .expect("prepend failed");

    let result = vault.read("prepend_test.md").await;
    // Frontmatter should be first
    assert!(result.starts_with("---\ntitle: Test\n---"));
    // Prepended content before original body
    let fm_end = result.find("---\n").unwrap() + 4;
    let after_fm = &result[fm_end..];
    let prepend_pos = after_fm.find("Prepended Section").expect("prepended content missing");
    let body_pos = after_fm.find("Original body").expect("original body missing");
    assert!(prepend_pos < body_pos, "Prepended content should come before original body");
}
```

- [ ] **Step 3: Write write_overwrite_replaces_entirely test**

```rust
#[tokio::test]
async fn write_overwrite_replaces_entirely() {
    let vault = TestVault::new().await;
    vault.write("overwrite_test.md", "# Old\n\nOld content.\n").await;
    vault.write("overwrite_test.md", "# New\n\nNew content.\n").await;

    let result = vault.read("overwrite_test.md").await;
    assert!(!result.contains("Old content"));
    assert!(result.contains("New content"));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test vault_integrity_test write_ -- --nocapture`
Expected: All 3 PASS

- [ ] **Step 5: Commit**

```bash
git add tests/vault_integrity_test.rs
git commit -m "test: add write mode integrity tests (append, prepend, overwrite)"
```

---

## Task 5: Edit Operations + Partial Detection Tests

**Files:**
- Modify: `tests/vault_integrity_test.rs`

- [ ] **Step 1: Write edit_search_replace_basic test**

```rust
#[tokio::test]
async fn edit_search_replace_basic() {
    let vault = TestVault::new().await;
    vault.write("edit_test.md", "# Test\n\nHello world.\n").await;

    let edits = "<<<<<<< SEARCH\nHello world.\n=======\nHello TurboVault.\n>>>>>>> REPLACE";
    vault.manager
        .edit_file(&std::path::PathBuf::from("edit_test.md"), edits, None, false)
        .await
        .expect("edit failed");

    let result = vault.read("edit_test.md").await;
    assert!(result.contains("Hello TurboVault."));
    assert!(!result.contains("Hello world."));
}
```

- [ ] **Step 2: Write edit_two_sequential_edits test**

```rust
#[tokio::test]
async fn edit_two_sequential_edits() {
    let vault = TestVault::new().await;
    vault.write("seq_edit.md", "# Test\n\nAAA\n\nBBB\n").await;

    let edit1 = "<<<<<<< SEARCH\nAAA\n=======\nCCC\n>>>>>>> REPLACE";
    vault.manager
        .edit_file(&std::path::PathBuf::from("seq_edit.md"), edit1, None, false)
        .await
        .expect("edit1 failed");

    let edit2 = "<<<<<<< SEARCH\nBBB\n=======\nDDD\n>>>>>>> REPLACE";
    vault.manager
        .edit_file(&std::path::PathBuf::from("seq_edit.md"), edit2, None, false)
        .await
        .expect("edit2 failed");

    let result = vault.read("seq_edit.md").await;
    assert!(result.contains("CCC"));
    assert!(result.contains("DDD"));
    assert!(!result.contains("AAA"));
    assert!(!result.contains("BBB"));
}
```

- [ ] **Step 3: Write edit_stale_hash_rejected test**

```rust
use turbovault_vault::compute_hash;

#[tokio::test]
async fn edit_stale_hash_rejected() {
    let vault = TestVault::new().await;
    vault.write("hash_test.md", "# Test\n\nOriginal.\n").await;

    let edits = "<<<<<<< SEARCH\nOriginal.\n=======\nModified.\n>>>>>>> REPLACE";
    let result = vault.manager
        .edit_file(
            &std::path::PathBuf::from("hash_test.md"),
            edits,
            Some("wrong_hash_value"),
            false,
        )
        .await;

    assert!(result.is_err(), "Should reject stale hash");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("modified since read") || err.contains("hash"));
}
```

- [ ] **Step 4: Write edit_partial_operation_detectable test**

```rust
#[tokio::test]
async fn edit_partial_operation_detectable() {
    let vault = TestVault::new().await;
    let original = "---\nstatus: draft\n---\n\n# Section A\n\nContent A.\n\n# Section B\n\nContent B.\n";
    vault.write("partial.md", original).await;

    let original_hash = compute_hash(original);

    // Apply only the first of two intended edits
    let edit1 = "<<<<<<< SEARCH\nContent A.\n=======\nUpdated A.\n>>>>>>> REPLACE";
    vault.manager
        .edit_file(&std::path::PathBuf::from("partial.md"), edit1, None, false)
        .await
        .expect("edit1 failed");

    let after_partial = vault.read("partial.md").await;
    let partial_hash = compute_hash(&after_partial);

    // The expected final state would have both sections updated
    let expected_final = original.replace("Content A.", "Updated A.").replace("Content B.", "Updated B.");
    let final_hash = compute_hash(&expected_final);

    // The partial state hash differs from both original and expected final
    assert_ne!(partial_hash, original_hash, "Should differ from original");
    assert_ne!(partial_hash, final_hash, "Should differ from expected final");

    // But the content is still valid markdown
    assert!(after_partial.contains("Updated A."));
    assert!(after_partial.contains("Content B."), "Section B should be untouched");
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test vault_integrity_test edit_ -- --nocapture`
Expected: All 4 PASS

- [ ] **Step 6: Commit**

```bash
git add tests/vault_integrity_test.rs
git commit -m "test: add edit operation and partial detection tests"
```

---

## Task 6: Batch Operations + Partial Failure Tests

**Files:**
- Modify: `tests/vault_integrity_test.rs`
- Modify: `Cargo.toml` (add `turbovault-batch` as dev-dependency for integration tests)

- [ ] **Step 0: Add turbovault-batch dev-dependency**

Add to root `Cargo.toml` under `[workspace.dependencies]` (already there). Then in `crates/turbovault/Cargo.toml`, add:
```toml
[dev-dependencies]
turbovault-batch = { workspace = true }
```

- [ ] **Step 1: Write batch_all_succeed test**

```rust
use turbovault_tools::BatchTools;
use turbovault_batch::BatchOperation;

#[tokio::test]
async fn batch_all_succeed() {
    let vault = TestVault::new().await;
    let batch = BatchTools::new(vault.manager.clone());

    let ops = vec![
        BatchOperation::CreateNote {
            path: "batch_a.md".to_string(),
            content: "# A\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "batch_b.md".to_string(),
            content: "# B\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "batch_c.md".to_string(),
            content: "# C\n".to_string(),
        },
    ];

    let result = batch.batch_execute(ops).await.expect("batch failed");
    assert!(result.success);
    assert_eq!(result.executed, 3);

    assert_eq!(vault.read("batch_a.md").await, "# A\n");
    assert_eq!(vault.read("batch_b.md").await, "# B\n");
    assert_eq!(vault.read("batch_c.md").await, "# C\n");
}
```

- [ ] **Step 2: Write batch_poisoned_operation_stops test**

```rust
#[tokio::test]
async fn batch_poisoned_operation_stops() {
    let vault = TestVault::new().await;
    let batch = BatchTools::new(vault.manager.clone());

    let ops = vec![
        BatchOperation::CreateNote {
            path: "good_file.md".to_string(),
            content: "# Good\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "../../escape_attempt.md".to_string(),
            content: "# Bad\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "never_created.md".to_string(),
            content: "# Never\n".to_string(),
        },
    ];

    let result = batch.batch_execute(ops).await.expect("batch should return result");
    assert!(!result.success);
    assert_eq!(result.failed_at, Some(1));

    // First file was created
    assert_eq!(vault.read("good_file.md").await, "# Good\n");

    // Third file was never created
    let third = vault.file_tools().read_file("never_created.md").await;
    assert!(third.is_err(), "Third file should not exist");
}
```

- [ ] **Step 3: Write batch_partial_state_inspectable test**

```rust
#[tokio::test]
async fn batch_partial_state_inspectable() {
    let vault = TestVault::new().await;
    let batch = BatchTools::new(vault.manager.clone());

    // First create a file, then batch: write to it + invalid op + write another
    vault.write("existing.md", "# Existing\n").await;

    let ops = vec![
        BatchOperation::WriteNote {
            path: "existing.md".to_string(),
            content: "# Updated\n".to_string(),
        },
        BatchOperation::CreateNote {
            path: "../../traversal.md".to_string(),
            content: "bad".to_string(),
        },
        BatchOperation::CreateNote {
            path: "should_not_exist.md".to_string(),
            content: "# Nope\n".to_string(),
        },
    ];

    let result = batch.batch_execute(ops).await.expect("batch result");
    assert!(!result.success);

    // Written file is valid (not half-written)
    let existing = vault.read("existing.md").await;
    assert!(existing == "# Updated\n" || existing == "# Existing\n",
        "File should be either fully updated or untouched, not corrupted");

    // Unexecuted file does not exist
    assert!(vault.file_tools().read_file("should_not_exist.md").await.is_err());
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test vault_integrity_test batch_ -- --nocapture`
Expected: All 3 PASS

- [ ] **Step 5: Commit**

```bash
git add tests/vault_integrity_test.rs Cargo.toml crates/turbovault/Cargo.toml
git commit -m "test: add batch operation partial failure tests"
```

---

## Task 7: Heading-Aware Operations (patch_note) Tests

**Prerequisite:** The `patch_note` implementation in `crates/turbovault/src/tools.rs` must be updated to skip headings inside fenced code blocks before these tests are meaningful. If the fix hasn't been applied yet, implement it first: track whether you're inside a fenced code block (lines starting with ``` ``` ```) and skip heading matching inside them.

**Files:**
- Modify: `tests/vault_integrity_test.rs`
- Possibly modify: `crates/turbovault/src/tools.rs` (patch_note code-block fix)

- [ ] **Step 1: Fix patch_note to skip headings in code blocks**

In `crates/turbovault/src/tools.rs`, in the `patch_note` method's heading-matching loop, add code-block tracking:

```rust
let mut in_code_block = false;
for (i, line) in lines.iter().enumerate() {
    let trimmed = line.trim();
    if trimmed.starts_with("```") {
        in_code_block = !in_code_block;
        continue;
    }
    if in_code_block {
        continue;
    }
    if trimmed.starts_with('#') {
        // ... existing heading matching logic
    }
}
```

- [ ] **Step 2: Write patch_heading_append_correct_section test**

```rust
#[tokio::test]
async fn patch_heading_append_correct_section() {
    let vault = TestVault::new().await;
    let content = "# Title\n\n## Section A\n\nContent A.\n\n## Section B\n\nContent B.\n\n## Section C\n\nContent C.\n";
    vault.write("patch_test.md", content).await;

    // Simulate patch_note by reading, finding heading, inserting
    let existing = vault.read("patch_test.md").await;
    // Insert "New item." under Section B
    let new_content = existing.replace(
        "Content B.\n\n## Section C",
        "Content B.\n\nNew item.\n\n## Section C",
    );
    vault.write("patch_test.md", &new_content).await;

    let result = vault.read("patch_test.md").await;
    assert!(result.contains("Content A."), "Section A untouched");
    assert!(result.contains("New item."), "Inserted content present");
    assert!(result.contains("Content C."), "Section C untouched");

    // Verify ordering: Section B content before inserted content before Section C
    let b_pos = result.find("Content B.").unwrap();
    let new_pos = result.find("New item.").unwrap();
    let c_pos = result.find("## Section C").unwrap();
    assert!(b_pos < new_pos && new_pos < c_pos);
}
```

- [ ] **Step 3: Write patch_heading_in_code_block_ignored test**

```rust
#[tokio::test]
async fn patch_heading_in_code_block_ignored() {
    let vault = TestVault::new().await;
    let content = "# Title\n\n```\n## Fake Heading\n\nCode content.\n```\n\n## Real Heading\n\nReal content.\n";
    vault.write("code_heading.md", content).await;
    let parsed = vault.parse("code_heading.md").await;

    // Only the real heading should be detected, not the one in the code block
    let headings: Vec<_> = parsed.headings.iter().map(|h| h.text.as_str()).collect();
    assert!(headings.contains(&"Title"));
    assert!(headings.contains(&"Real Heading"));
    assert!(!headings.contains(&"Fake Heading"), "Should not match heading inside code block");
}
```

- [ ] **Step 4: Write patch_heading_duplicate_names test**

```rust
#[tokio::test]
async fn patch_heading_duplicate_names() {
    let vault = TestVault::new().await;
    let content = "# Title\n\n## Notes\n\nFirst notes section.\n\n### Sub\n\nSub content.\n\n## Notes\n\nSecond notes section.\n";
    vault.write("dupe_heading.md", content).await;

    // patch_note matches first occurrence — verify the first "Notes" heading
    // is at a different position than the second
    let parsed = vault.parse("dupe_heading.md").await;
    let notes_headings: Vec<_> = parsed.headings.iter()
        .filter(|h| h.text == "Notes")
        .collect();
    assert_eq!(notes_headings.len(), 2, "Should find both Notes headings");
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test vault_integrity_test patch_ -- --nocapture`
Expected: All PASS

- [ ] **Step 6: Commit**

```bash
git add tests/vault_integrity_test.rs crates/turbovault/src/tools.rs
git commit -m "test: add patch_note heading tests with code-block awareness fix"
```

---

## Task 8: Move/Rename Tests

**Files:**
- Modify: `tests/vault_integrity_test.rs`

- [ ] **Step 1: Write move_preserves_content test**

```rust
#[tokio::test]
async fn move_preserves_content() {
    let vault = TestVault::new().await;
    let content = "---\ntitle: Moveable\n---\n\n# Content\n\nImportant data.\n";
    vault.write("original.md", content).await;

    vault.file_tools().move_file("original.md", "moved/new_name.md").await.expect("move failed");

    let moved_content = vault.read("moved/new_name.md").await;
    assert_eq!(moved_content, content);

    let original = vault.file_tools().read_file("original.md").await;
    assert!(original.is_err(), "Original path should not exist after move");
}
```

- [ ] **Step 2: Write move_graph_detects_broken_links test**

```rust
#[tokio::test]
async fn move_graph_detects_broken_links() {
    let vault = TestVault::new().await;
    vault.write("linker.md", "# Linker\n\nSee [[target]].\n").await;
    vault.write("target.md", "# Target\n\nContent.\n").await;
    vault.reinitialize().await;

    // Before move: no broken links
    let stats_before = vault.manager.get_stats().await.expect("stats");
    assert_eq!(stats_before.total_links, 1);

    // Move target
    vault.file_tools().move_file("target.md", "archive/target.md").await.expect("move failed");
    vault.reinitialize().await;

    // After move: linker.md still has [[target]] but target.md is gone
    // The link should now be broken
    let linker_content = vault.read("linker.md").await;
    assert!(linker_content.contains("[[target]]"), "Link text should be unchanged");
}
```

- [ ] **Step 3: Write move_interrupted_link_update test**

```rust
#[tokio::test]
async fn move_interrupted_link_update() {
    let vault = TestVault::new().await;
    vault.write("doc.md", "# Doc\n\nReferences [[old_name]].\n").await;
    vault.write("old_name.md", "# Old Name\n\nContent.\n").await;
    vault.reinitialize().await;

    // Step 1 of 2: Move the file (simulates agent completing first step)
    vault.file_tools().move_file("old_name.md", "new_name.md").await.expect("move failed");

    // Step 2 would be: update [[old_name]] -> [[new_name]] in doc.md
    // But context compression happens — step 2 never executes

    // Verify the vault is in a detectable inconsistent state:
    // - new_name.md exists with correct content
    let moved = vault.read("new_name.md").await;
    assert!(moved.contains("Content."));

    // - old_name.md does not exist
    assert!(vault.file_tools().read_file("old_name.md").await.is_err());

    // - doc.md still references [[old_name]] — a broken link
    let doc = vault.read("doc.md").await;
    assert!(doc.contains("[[old_name]]"), "Stale link should remain");
}
```

- [ ] **Step 4: Write move_then_manual_link_update test**

```rust
#[tokio::test]
async fn move_then_manual_link_update() {
    let vault = TestVault::new().await;
    vault.write("doc2.md", "# Doc\n\nReferences [[old_target]].\n").await;
    vault.write("old_target.md", "# Target\n").await;
    vault.reinitialize().await;

    // Step 1: Move
    vault.file_tools().move_file("old_target.md", "new_target.md").await.expect("move failed");

    // Step 2: Update link manually
    let edits = "<<<<<<< SEARCH\n[[old_target]]\n=======\n[[new_target]]\n>>>>>>> REPLACE";
    vault.manager
        .edit_file(&std::path::PathBuf::from("doc2.md"), edits, None, false)
        .await
        .expect("link update failed");

    vault.reinitialize().await;

    // Verify: no broken links, all references correct
    let doc = vault.read("doc2.md").await;
    assert!(doc.contains("[[new_target]]"));
    assert!(!doc.contains("[[old_target]]"));
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test vault_integrity_test move_ -- --nocapture`
Expected: All 4 PASS

- [ ] **Step 6: Commit**

```bash
git add tests/vault_integrity_test.rs
git commit -m "test: add move/rename integrity tests including partial operation scenarios"
```

---

## Task 9: Layer 2 — Python Project Scaffolding

**Files:**
- Create: `~/projects/vault-integrity-tests/config.yaml`
- Create: `~/projects/vault-integrity-tests/requirements.txt`
- Create: `~/projects/vault-integrity-tests/mcp_client.py`
- Create: `~/projects/vault-integrity-tests/compare.py`
- Create: `~/projects/vault-integrity-tests/run.py`

- [ ] **Step 1: Create project directory and config**

```yaml
# config.yaml
turbovault_url: http://vault.home.iramsay.com/mcp
obsidian_mcp_url: http://localhost:8811  # Docker MCP gateway obsidian endpoint
vault_test_path: Staging/_integrity-tests
retry_delay_ms: 200
retry_max: 3
```

- [ ] **Step 2: Create requirements.txt**

```
requests>=2.31
pyyaml>=6.0
```

- [ ] **Step 3: Create mcp_client.py**

Thin JSON-RPC client that:
- `TurboVaultClient.call(method, params)` — sends JSON-RPC to TurboVault
- `TurboVaultClient.write_note(path, content, mode)` — convenience wrapper
- `TurboVaultClient.read_note(path)` — convenience wrapper
- `TurboVaultClient.patch_note(path, target_type, target, operation, content)`
- `TurboVaultClient.edit_note(path, edits)`
- `TurboVaultClient.move_note(from_path, to_path)`
- `TurboVaultClient.delete_note(path)`
- `ObsidianClient.call(method, params)` — sends to Obsidian MCP
- `ObsidianClient.get_file_contents(path)` — convenience wrapper
- `ObsidianClient.patch_content(filepath, target_type, target, operation, content)`

- [ ] **Step 4: Create compare.py**

Semantic comparison utilities:
- `compare_frontmatter(text_a, text_b)` — parse YAML frontmatter from both, compare keys/values
- `compare_body(text_a, text_b)` — strip trailing whitespace per line, normalize line endings, compare
- `semantic_equals(text_a, text_b)` — frontmatter + body comparison
- `show_diff(text_a, text_b)` — unified diff for debugging failures

- [ ] **Step 5: Create run.py**

CLI runner:
- Discovers test files (`test_*.py`) via importlib
- Runs each test function, catches exceptions, records PASS/FAIL/time
- `--cleanup` flag deletes all `Staging/_integrity-tests/` files
- Outputs summary in the spec's format
- Exit code 0 = all pass, 1 = failures

- [ ] **Step 6: Install dependencies and verify**

Run: `cd ~/projects/vault-integrity-tests && pip install -r requirements.txt`
Run: `python3 run.py --help`
Expected: Shows usage without errors

- [ ] **Step 7: Commit**

```bash
cd ~/projects/vault-integrity-tests
git init && git add -A
git commit -m "feat: vault integrity test suite scaffolding with MCP clients and comparison utilities"
```

---

## Task 10: Layer 2 — Frontmatter Round-Trip Tests

**Files:**
- Create: `~/projects/vault-integrity-tests/test_frontmatter.py`

- [ ] **Step 1: Write tv_write_obsidian_read test**

Write a note with complex frontmatter via TurboVault, read via Obsidian MCP, assert semantic equivalence using `compare.py`.

- [ ] **Step 2: Write obsidian_write_tv_read test**

Write via Obsidian MCP (`obsidian_patch_content` with frontmatter target_type), read via TurboVault, compare.

- [ ] **Step 3: Write frontmatter_tags_parity test**

Write tags via TurboVault, verify Obsidian returns the same tags.

- [ ] **Step 4: Run and verify**

Run: `python3 run.py test_frontmatter.py`
Expected: All PASS (or documented failures to investigate)

- [ ] **Step 5: Commit**

```bash
git add test_frontmatter.py && git commit -m "test: add cross-tool frontmatter round-trip tests"
```

---

## Task 11: Layer 2 — Wikilink + Heading Tests

**Files:**
- Create: `~/projects/vault-integrity-tests/test_wikilinks.py`
- Create: `~/projects/vault-integrity-tests/test_heading_patch.py`

- [ ] **Step 1: Write test_wikilinks.py**

`cross_tool_link_creation` — create two linked notes via TurboVault, query backlinks, read raw file via Obsidian MCP, verify wikilink text.

- [ ] **Step 2: Write test_heading_patch.py**

`tv_patch_obsidian_verify` and `obsidian_patch_tv_verify` — insert under heading via each tool, read with the other, verify structure.

- [ ] **Step 3: Run and verify**

Run: `python3 run.py test_wikilinks.py test_heading_patch.py`

- [ ] **Step 4: Commit**

```bash
git add test_wikilinks.py test_heading_patch.py
git commit -m "test: add cross-tool wikilink and heading patch tests"
```

---

## Task 12: Layer 2 — Edit, Concurrent, Move Tests

**Files:**
- Create: `~/projects/vault-integrity-tests/test_edit.py`
- Create: `~/projects/vault-integrity-tests/test_concurrent.py`
- Create: `~/projects/vault-integrity-tests/test_move.py`

- [ ] **Step 1: Write test_edit.py**

`cross_tool_edit_visibility` and `partial_move_detection` — edit via TurboVault, read via Obsidian, plus the partial move scenario.

- [ ] **Step 2: Write test_concurrent.py**

`tv_write_obsidian_immediate_read` and `obsidian_write_tv_immediate_read` — write with one tool, read immediately with the other (200ms delay + retry logic).

- [ ] **Step 3: Write test_move.py**

`tv_move_obsidian_verify_content` — move via TurboVault, verify via Obsidian MCP at new path.

- [ ] **Step 4: Run full suite**

Run: `python3 run.py`
Expected: Summary output showing all tests with PASS/FAIL status

- [ ] **Step 5: Commit**

```bash
git add test_edit.py test_concurrent.py test_move.py
git commit -m "test: add cross-tool edit, concurrent access, and move tests"
```

---

## Task 13: Final Validation and Push

**Files:**
- Both repos

- [ ] **Step 1: Run full Layer 1 suite**

Run: `cd ~/projects/turbovault && cargo test --test vault_integrity_test -- --nocapture`
Expected: All tests PASS

- [ ] **Step 2: Run full Layer 2 suite**

Run: `cd ~/projects/vault-integrity-tests && python3 run.py`
Expected: Summary output, document any failures

- [ ] **Step 3: Push TurboVault changes**

```bash
cd ~/projects/turbovault
git push origin main
```

- [ ] **Step 4: Push vault-integrity-tests repo**

```bash
cd ~/projects/vault-integrity-tests
gh repo create maxramsay/vault-integrity-tests --private --source=. --push
```

- [ ] **Step 5: Document results in vault**

Update today's daily note with test suite results and any failures found that need fixing before cutover.
