# Vault Integrity Test Suite — Design Spec

## Problem

TurboVault is replacing the Obsidian REST API as the vault MCP server for all agents. The vault is a shared knowledge space used by both humans (via Obsidian) and agents (via TurboVault). Before cutting over, we need confidence that TurboVault produces files that Obsidian reads correctly, and vice versa.

TurboVault currently performs no content validation on write — it accepts any string and writes it atomically. Parser failures after write are logged as warnings but don't block the write. This means an agent could write malformed frontmatter, broken wikilinks, or structurally invalid markdown without any error signal.

### Failure Modes

The primary risks are:

1. **Structural corruption** — malformed frontmatter that breaks Dataview/Tasks queries, broken wikilinks, invalid OFM syntax
2. **Silent data loss** — an agent overwrites a section thinking it's appending, or deletes links during an edit
3. **Partial operations** — an agent needs multiple MCP calls to complete an operation (e.g., move a file + update references), but context compression interrupts the sequence, leaving the vault in an inconsistent state

### Invariant

**Any file written by TurboVault must be a valid Obsidian vault file that Obsidian reads correctly, and vice versa.**

## Architecture

Two-layer test suite:

```
Layer 1: Rust Integration Tests (TurboVault repo)
    Engine-level validation using temporary vaults
    Runs with: cargo test
    Catches: engine bugs during development

Layer 2: External Round-Trip Tests (standalone Python project)
    Cross-tool validation against live MCP endpoints
    Runs on: Oracle or MaxDesk (access to both TurboVault + Obsidian MCP)
    Catches: transport issues, serialization bugs, cross-tool incompatibilities
```

Both layers are necessary. The Rust tests catch engine bugs early. The external tests prove the deployed system works end-to-end with real Obsidian.

## Layer 1: Rust Integration Tests

**Location:** `tests/vault_integrity_test.rs` in the TurboVault repo.

**Pattern:** Each test creates a temporary vault via `tempfile::TempDir`, writes content using VaultManager, reads it back, parses it with TurboVault's OFM parser, and asserts the parsed structure matches expectations. Tests are fully independent — no shared state between tests.

### Test Categories

#### 1. Frontmatter Integrity

| Test | Description |
|------|-------------|
| `frontmatter_complex_round_trip` | Write frontmatter with nested values, arrays, dates, special characters. Read back, parse, assert all keys/values survive. |
| `frontmatter_malformed_no_corruption` | Write malformed frontmatter (missing closing `---`, invalid YAML). Verify the file is still readable and body content isn't corrupted. |
| `frontmatter_unicode_preservation` | Write frontmatter with unicode characters (CJK, emoji, diacritics). Verify exact preservation. Also verify NFC vs NFD normalization forms both survive round-trip and don't produce false hash conflicts. |

#### 2. Wikilink Integrity

| Test | Description |
|------|-------------|
| `wikilink_all_variants` | Write note with `[[simple]]`, `[[folder/path]]`, `[[note\|alias]]`, `[[note#heading]]`, `[[note#^blockref]]`, `![[embed]]`. Parse, assert all links detected with correct targets and aliases. |
| `wikilink_backlink_resolution` | Write two notes linking to each other. Initialize vault. Verify backlinks resolve correctly in both directions. |
| `wikilink_in_code_blocks_ignored` | Write wikilinks inside fenced code blocks and inline code. Verify parser does not extract them as links. |

#### 3. Write Mode Operations

| Test | Description |
|------|-------------|
| `write_append_preserves_existing` | Write a note, then append via `write_note` with mode=append. Verify original content is untouched and new content follows it. |
| `write_prepend_after_frontmatter` | Write a note with frontmatter, then prepend. Verify new content appears after frontmatter but before original body. Frontmatter is intact. |
| `write_overwrite_replaces_entirely` | Write a note, then overwrite. Verify only the new content exists — no remnants of original. |

#### 4. Heading-Aware Operations (`patch_note`)

| Test | Description |
|------|-------------|
| `patch_heading_append_correct_section` | Create note with H1-H4 headings. Call `patch_note` with `target_type: heading`, `operation: append` under a specific H2. Read back, verify content is in the right section and adjacent sections are untouched. |
| `patch_heading_prepend` | Call `patch_note` with `operation: prepend` under a heading. Verify content appears immediately after the heading line, before existing section content. |
| `patch_heading_replace` | Call `patch_note` with `operation: replace`. Verify section content is replaced but the heading itself and other sections are untouched. |
| `patch_heading_last_section` | Append under the last heading in the file. Verify no trailing corruption. |
| `patch_heading_nested_levels` | Append under a nested heading (H3 under H2). Verify content lands between the H3 and the next H3 or higher, not at the wrong level. |
| `patch_heading_special_characters` | Heading with special characters (colons, dashes, unicode). Verify matching still works. |
| `patch_heading_in_code_block_ignored` | Target a heading text that also appears inside a fenced code block. Verify `patch_note` matches only the real heading, not the code block content. |
| `patch_heading_duplicate_names` | Two headings with the same text at different levels. Verify `patch_note` matches the first occurrence and documents this behavior. |
| `patch_block_reference` | Call `patch_note` with `target_type: block`, targeting a `^blockid`. Verify content is inserted at the correct location. |
| `patch_frontmatter_field` | Call `patch_note` with `target_type: frontmatter`, targeting a specific key. Verify the field is updated without corrupting other frontmatter or body content. |

#### 5. Edit Operations + Partial Edit Detection

| Test | Description |
|------|-------------|
| `edit_search_replace_basic` | Write note, apply SEARCH/REPLACE edit via `edit_note`, verify replacement. |
| `edit_two_sequential_edits` | Apply two edits in sequence, verify both applied correctly. |
| `edit_stale_hash_rejected` | Apply edit with wrong expected hash. Verify rejection (TOCTOU protection). |
| `edit_partial_operation_detectable` | Write note with two sections that should be updated together. Apply only one update. Read back, verify the content hash differs from both the original and the expected final state — the inconsistency is detectable. |

#### 6. Batch Operations + Partial Failure

| Test | Description |
|------|-------------|
| `batch_all_succeed` | Execute a batch of 3 file creates via `batch_execute`. Verify all files exist with correct content. |
| `batch_poisoned_operation_stops` | Execute a batch where the 2nd of 3 operations is invalid (e.g., writes to a path-traversal target). Verify the batch stops, the 1st operation's result is visible, and the 3rd operation was never executed. |
| `batch_partial_state_inspectable` | After a partial batch failure, verify the vault state is consistent — files that were written are valid, files that weren't written don't exist. No half-written files. |

#### 7. Move/Rename

**Note:** TurboVault's `move_note` currently does NOT update wikilinks in other files. It moves the file and warns that links are now broken. These tests validate current behavior. If link-update-on-move is implemented in the future, additional tests should be added.

| Test | Description |
|------|-------------|
| `move_preserves_content` | Move a note. Verify the moved file's content is byte-identical to before the move. Original path no longer exists. |
| `move_graph_detects_broken_links` | Create note A linking to note B. Move B. Verify the link graph correctly reports A's link to B as broken. |
| `move_then_manual_link_update` | Move B, then manually edit A to update the wikilink. Verify the graph shows no broken links after both steps. This simulates the multi-step operation an agent would perform. |
| `move_interrupted_link_update` | Move B (succeeds), but skip the link update in A. Verify: B exists at new path, A still has old link, graph reports broken link. This is the "partial operation" scenario — the vault is in a detectable inconsistent state. |

## Layer 2: External Round-Trip Tests

**Location:** `~/projects/vault-integrity-tests/` — standalone Python project, separate from TurboVault repo.

**Configuration:** Endpoints configured via environment variables or a `config.yaml` file:
```yaml
turbovault_url: http://vault.home.iramsay.com/mcp
obsidian_mcp: docker-mcp-gateway  # or direct HTTP endpoint
vault_test_path: Staging/_integrity-tests
```

**Endpoints:**
- TurboVault: `http://vault.home.iramsay.com/mcp` (JSON-RPC over HTTP)
- Obsidian MCP: via Docker MCP gateway (same endpoint agents use)

**Test data location:** `Staging/_integrity-tests/` in the vault. All test files prefixed with `_test-{category}-{timestamp}.md`.

**Test independence:** Each test creates its own files and cleans up after itself. No test depends on another test's output. Tests can run in any order.

**Cleanup:** Each test cleans up on success. On failure, files remain for debugging. `--cleanup` flag purges all leftover test files.

### Comparison Strategy

Cross-tool comparisons use **semantic equivalence**, not byte-identical matching. Specifically:
- Frontmatter: parsed YAML keys/values must match (tolerates whitespace/ordering differences)
- Body content: normalized comparison (strip trailing whitespace per line, normalize line endings)
- When semantic comparison fails, the test output shows the raw diff for debugging

### Test Categories

#### 1. Frontmatter Round-Trip (Cross-Tool)

| Test | Description |
|------|-------------|
| `tv_write_obsidian_read` | Write note with complex frontmatter via TurboVault. Read via Obsidian MCP. Assert semantic equivalence (parsed frontmatter keys/values match, body content matches after normalization). |
| `obsidian_write_tv_read` | Write frontmatter via Obsidian MCP (`obsidian_patch_content` with frontmatter target). Read via TurboVault. Compare. |
| `frontmatter_tags_parity` | Write tags in frontmatter via TurboVault. Verify Obsidian MCP returns the same tags. |

#### 2. Wikilink Resolution Parity

| Test | Description |
|------|-------------|
| `cross_tool_link_creation` | Create two linked notes via TurboVault. Query backlinks via TurboVault. Read raw file via Obsidian MCP, verify wikilink text is correct. |

#### 3. Heading-Aware Insert Parity

| Test | Description |
|------|-------------|
| `tv_patch_obsidian_verify` | Create structured note via TurboVault. Insert under heading via TurboVault's `patch_note`. Read via Obsidian MCP, verify content is in the right section. |
| `obsidian_patch_tv_verify` | Insert under heading via Obsidian's `obsidian_patch_content`. Read via TurboVault, verify consistency. |

#### 4. Edit + Partial Operation Detection

| Test | Description |
|------|-------------|
| `cross_tool_edit_visibility` | Write note via TurboVault. Apply edit via TurboVault. Read via Obsidian MCP, verify edit applied correctly. |
| `partial_move_detection` | Create notes A→B via TurboVault. Move B (succeeds). Skip link update in A. Read A via both tools. Verify both tools see the stale wikilink — the inconsistency is visible from either side. |

#### 5. Concurrent Access

| Test | Description |
|------|-------------|
| `tv_write_obsidian_immediate_read` | Write note via TurboVault. Read via Obsidian MCP after a 200ms delay (allows filesystem sync). Verify content matches. If mismatch, retry with 1s backoff up to 3 times before failing. |
| `obsidian_write_tv_immediate_read` | Write via Obsidian MCP. Read via TurboVault after 200ms delay. Verify content matches with same retry logic. |

#### 6. Move/Rename Cross-Tool

| Test | Description |
|------|-------------|
| `tv_move_obsidian_verify_content` | Create note B via TurboVault. Move B to new path. Read B at new path via Obsidian MCP, verify content is intact. Verify old path returns not found. |

### Output Format

```
[PASS] tv_write_obsidian_read (234ms)
[PASS] obsidian_write_tv_read (189ms)
[FAIL] frontmatter_tags_parity (312ms)
       Expected tags: ['infrastructure', 'homelab']
       TurboVault returned: ['infrastructure', 'homelab']
       Obsidian returned: ['infrastructure']
       Diff: Obsidian dropped 'homelab' tag

Results: 14/15 passed, 1 failed
```

Exit code 0 on all pass, non-zero on any failure.

## What This Unblocks

Once both layers pass:
1. Confidence to cut over agents from Obsidian MCP to TurboVault
2. Regression gate for future TurboVault changes (Rust tests in CI)
3. Deployment validation (external tests as pre-cutover or post-deploy check)

## What This Does NOT Cover

- **Performance testing** — this suite validates correctness, not speed
- **Plugin compatibility** — we test core Obsidian, not every community plugin's expectations
- **Multi-vault operations** — tests focus on single-vault integrity (our current setup)
- **Network resilience** — no testing of what happens when the MCP endpoint is unreachable mid-operation
- **Automatic link updates on move** — `move_note` currently does not update wikilinks in other files. Tests validate current behavior (broken links are detected). If this feature is added later, new tests should be written.

## Prerequisites

Before implementing this test suite, the following must be resolved:

1. **`patch_note` heading matching must be code-block-aware** — the current implementation matches headings inside fenced code blocks, which would cause content insertion inside code blocks. This is a bug that should be fixed before the heading tests are meaningful.
2. **Decide on `move_note` link update behavior** — if we want agents to safely move files, we either need `move_note` to update links atomically, or we need a documented two-step pattern (move + batch link update) with guidance for agents on how to handle interruption.
