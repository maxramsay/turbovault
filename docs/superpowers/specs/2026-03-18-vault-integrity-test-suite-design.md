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

**Pattern:** Each test creates a temporary vault via `tempfile::TempDir`, writes content using VaultManager, reads it back, parses it with TurboVault's OFM parser, and asserts the parsed structure matches expectations.

### Test Categories

#### 1. Frontmatter Integrity

| Test | Description |
|------|-------------|
| `frontmatter_complex_round_trip` | Write frontmatter with nested values, arrays, dates, special characters. Read back, parse, assert all keys/values survive. |
| `frontmatter_malformed_no_corruption` | Write malformed frontmatter (missing closing `---`, invalid YAML). Verify the file is still readable and body content isn't corrupted. |
| `frontmatter_unicode_preservation` | Write frontmatter with unicode characters (CJK, emoji, diacritics). Verify exact preservation. |

#### 2. Wikilink Integrity

| Test | Description |
|------|-------------|
| `wikilink_all_variants` | Write note with `[[simple]]`, `[[folder/path]]`, `[[note\|alias]]`, `[[note#heading]]`, `[[note#^blockref]]`, `![[embed]]`. Parse, assert all links detected with correct targets and aliases. |
| `wikilink_backlink_resolution` | Write two notes linking to each other. Initialize vault. Verify backlinks resolve correctly in both directions. |
| `wikilink_in_code_blocks_ignored` | Write wikilinks inside code blocks and inline code. Verify parser does not extract them as links. |

#### 3. Heading-Aware Operations

| Test | Description |
|------|-------------|
| `heading_append_correct_section` | Create note with H1-H4 headings. Append content under a specific H2. Read back, verify content is in the right section and adjacent sections are untouched. |
| `heading_append_last_section` | Append under the last heading in the file. Verify no trailing corruption. |
| `heading_nested_levels` | Append under a nested heading (H3 under H2). Verify content lands between the H3 and the next H3 or higher, not at the wrong level. |
| `heading_special_characters` | Heading with special characters (colons, dashes, unicode). Verify matching still works. |

#### 4. Edit Operations + Partial Edit Detection

| Test | Description |
|------|-------------|
| `edit_search_replace_basic` | Write note, apply SEARCH/REPLACE edit, verify replacement. |
| `edit_two_sequential_edits` | Apply two edits in sequence, verify both applied correctly. |
| `edit_stale_hash_rejected` | Apply edit with wrong expected hash. Verify rejection (TOCTOU protection). |
| `edit_partial_operation_detectable` | Write note with two sections that should be updated together. Apply only one update. Read back, verify the content hash differs from both the original and the expected final state — the inconsistency is detectable. |

#### 5. Move/Rename with Link Updates

| Test | Description |
|------|-------------|
| `move_updates_wikilinks` | Create note A linking to note B with `[[B]]`. Move B to new path. Read A, verify wikilink updated. |
| `move_graph_consistency` | After move, verify link graph has no broken links and backlinks reflect new path. |
| `move_preserves_content` | After move, verify the moved file's content is byte-identical to before the move. |

## Layer 2: External Round-Trip Tests

**Location:** `~/projects/vault-integrity-tests/` — standalone Python project, separate from TurboVault repo.

**Endpoints:**
- TurboVault: `http://vault.home.iramsay.com/mcp` (JSON-RPC over HTTP)
- Obsidian MCP: via Docker MCP gateway (same endpoint agents use)

**Test data location:** `Staging/_integrity-tests/` in the vault. All test files prefixed with `_test-{category}-{timestamp}.md`.

**Cleanup:** Each test cleans up on success. On failure, files remain for debugging. `--cleanup` flag purges all leftover test files.

### Test Categories

#### 1. Frontmatter Round-Trip (Cross-Tool)

| Test | Description |
|------|-------------|
| `tv_write_obsidian_read` | Write note with complex frontmatter via TurboVault. Read via Obsidian MCP. Assert content is byte-identical. |
| `obsidian_write_tv_read` | Write frontmatter via Obsidian MCP (`obsidian_patch_content` with frontmatter target). Read via TurboVault. Compare. |
| `frontmatter_tags_parity` | Write tags in frontmatter via TurboVault. Verify Obsidian MCP returns the same tags. |

#### 2. Wikilink Resolution Parity

| Test | Description |
|------|-------------|
| `cross_tool_link_creation` | Create two linked notes via TurboVault. Query backlinks via TurboVault. Read raw file via Obsidian MCP, verify wikilink text is correct. |
| `post_move_link_validity` | Move a note via TurboVault. Read the referencing note via Obsidian MCP. Verify wikilink was updated. |

#### 3. Heading-Aware Insert Parity

| Test | Description |
|------|-------------|
| `tv_patch_obsidian_verify` | Create structured note via TurboVault. Insert under heading via TurboVault's `patch_note`. Read via Obsidian MCP, verify content is in the right section. |
| `obsidian_patch_tv_verify` | Insert under heading via Obsidian's `obsidian_patch_content`. Read via TurboVault, verify consistency. |

#### 4. Edit + Partial Operation Detection

| Test | Description |
|------|-------------|
| `cross_tool_edit_visibility` | Write note via TurboVault. Apply edit via TurboVault. Read via Obsidian MCP, verify edit applied correctly. |
| `partial_operation_detection` | Write a note with two sections that should be updated together. Apply only one update via TurboVault. Read via both tools. Verify the inconsistency is detectable (e.g., a frontmatter `last_updated` field doesn't match content state). |

#### 5. Concurrent Access

| Test | Description |
|------|-------------|
| `tv_write_obsidian_immediate_read` | Write note via TurboVault. Immediately read via Obsidian MCP. Verify no stale cache — content matches. |
| `obsidian_write_tv_immediate_read` | Write via Obsidian MCP. Immediately read via TurboVault. Verify no stale cache. |

#### 6. Move/Rename Cross-Tool

| Test | Description |
|------|-------------|
| `tv_move_obsidian_verify` | Create notes A→B via TurboVault. Move B. Read A via Obsidian MCP, verify wikilink updated. Read B at new path via Obsidian MCP, verify exists. |

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
