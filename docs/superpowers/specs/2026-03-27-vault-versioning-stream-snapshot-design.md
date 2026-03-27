# FC Vault MLP: Note Versioning, Activity Stream, Snapshot CLI

**Date:** 2026-03-27
**Author:** Uisang (Oracle) + Max
**Status:** Draft
**Scope:** FC Vault MLP — the minimum that enables safe vault cleanup and provides foundational observability

## Overview

Three capabilities that make the FC Vault safe to operate on and observable from day one:

1. **Note Versioning** — enforced optimistic concurrency with complete lineage in frontmatter
2. **Activity Stream** — every vault operation (reads, writes, access failures, backup operations) published to NATS
3. **Snapshot CLI** — query-driven snapshots with configurable storage targets and safe restore

These are the foundation for everything downstream: Vault Curator, diff/history, subscriptions, cross-org federation, knowledge marketplace.

## Principles Applied

| Principle | How it applies |
|-----------|---------------|
| **Tenet #0: Security is Job Zero** | Access failures published to activity stream. In-place restore creates automatic pre-restore snapshot. Event queue persists through outages — no silent event loss. |
| **Tenet #1: Reliability is #1** | Vault operations succeed even if NATS is temporarily down (local queue with drain). Versioning prevents data loss from concurrent writes. |
| **Tenet #3: No Single Points of Failure** | Snapshot CLI is application-level backup. Filesystem-level backup (NAS/rsync) is the separate fallback when the vault server itself is the failure. |
| **Tenet #4: Own Your Critical Dependencies** | This is our fork. We control the release cycle, the data model, and the event contracts. |
| **Tenet #7: Interfaces Over Implementations** | Snapshot CLI talks to the vault API, not the filesystem. Target URIs are the interface (`file://`, `s3://` later). The vault is the authority on its own state. |
| **Tenet #14: Ship, Then Harden** | MLP ships with single-org, LAN trust, `file://` targets. Multi-org isolation, auth, and remote targets are tracked as forward dependencies. |
| **LP: Think Big** | org_id on every data structure and event from day one, even though MLP is single-org. |
| **LP: Ownership** | In-place restore auto-creates a pre-restore snapshot. We protect users from data loss even from their own authorized operations. |
| **LP: Earn Trust** | Complete lineage in frontmatter. Every operation observable in the activity stream. No silent failures. |

---

## 1. Note Versioning

### Data Model

Every note carries version metadata in its YAML frontmatter:

```yaml
---
title: Some Note
date: 2026-03-27
tags: [fleet-control]
version: 3
history:
  - version: 1
    hash: a3f8c2d1e5b7...
    by: uisang
    at: 2026-03-27T14:00:00Z
  - version: 2
    hash: 7b2c91f0a4e3...
    by: yu-sin
    at: 2026-03-27T16:30:00Z
  - version: 3
    hash: e1d4f08c2b9a...
    by: uisang
    at: 2026-03-27T19:45:00Z
---

# Actual note content
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `version` | u64 | Current version number. Starts at 1, increments on every write. Convenience field for quick reads. |
| `history` | Array | Complete lineage of all versions. Each entry records version number, content hash, actor, and timestamp. |
| `history[].hash` | String | SHA-256 of the note content body (everything below the frontmatter delimiter). Computed at write time, never stored separately. |
| `history[].by` | String | Actor identifier — agent name, human ID, or `system`. |
| `history[].at` | String | ISO 8601 timestamp of the write. |

### Content Hash Computation

The hash covers the **content body only** — everything after the closing `---` of the frontmatter. Frontmatter (including version metadata itself) is excluded from the hash. This means:

- The hash is a pure function of the note's informational content
- Version metadata changes (which happen on every write) don't change the hash unless the content actually changed
- Auditors can verify integrity by recomputing: `SHA-256(content_below_frontmatter) == latest history entry hash`

### History Growth

The history array grows unbounded in the MLP. For heavily-edited notes, this is acceptable:

- YAML handles large arrays efficiently
- The mutation stream is the authoritative complete record regardless
- Frontmatter history is a convenience for quick lineage checks without querying the stream
- Future: the Vault Curator can manage history compaction by creating linked archive notes (e.g., `_history/Some Note - History v1-450.md`) and trimming the main note's history array with a pointer to the archive

### Write Behavior

All write operations follow this flow:

1. **Create:** Caller provides content. Vault sets `version: 1`, computes hash, creates history entry with actor and timestamp. No version parameter required on create.

2. **Update:** Caller provides content and `version` parameter (the version they believe is current).
   - Vault reads the current file, extracts `version` from frontmatter.
   - If caller's version matches current: write new content, set `version: N+1`, compute new hash, append history entry, update `updated_by`/`updated_at`.
   - If mismatch: return **409 Conflict** with the current version number. Caller must re-read and retry.

3. **Move:** Version increments. History entry records the move (same hash if content unchanged, new path noted in the activity stream event).

4. **Delete/Archive:** Final version and hash recorded in the history. The activity stream event captures the terminal state.

### Concurrency Control

Enforced optimistic concurrency — **not optional**. Every update must include the version parameter. This is a change from the current behavior where If-Match/ETag is optional.

Rationale: two agents reading v1 and both writing v2 is a real scenario (identified during design). The second write silently clobbering the first is unacceptable. 409 forces the losing writer to re-read and reconcile.

### Out-of-Band Edit Detection

When someone edits a note outside the vault API (e.g., directly in Obsidian):

- The frontmatter version won't increment (Obsidian doesn't know about our versioning)
- On next vault API read: compute the content hash, compare to the latest history entry's hash
- If they differ: the note was modified out-of-band
- The vault should detect this and:
  - Bump the version
  - Add a history entry with `by: "external"` and the current timestamp
  - Log to the activity stream as `vault.note.updated` with `actor: "external"`

This preserves lineage integrity even when the vault API isn't the sole writer.

### Existing Notes Migration

The vault contains ~1,633 notes without version metadata. On first access through the vault API after this feature ships:

- If a note lacks `version` in frontmatter: treat as version 0 (pre-versioning)
- On first write: set `version: 1`, create initial history entry with the current content hash and `by: "migration"`
- On read-only access: return `version: 0` and compute the hash on the fly. Do not modify the file on read.

This is a lazy migration — notes get versioned as they're touched, not in a bulk sweep.

---

## 2. Activity Stream

### Architecture

The FC Vault publishes every operation as a CloudEvents event to a single NATS JetStream subject per org.

```
FC Vault Server
    │
    ├─ on every operation
    │
    ▼
NATS JetStream: fc.{org}.vault.activity
    │
    │  (one primary stream, one authorized consumer)
    │
    ▼
FC Message Bus Service
    │
    ├──► fc.{org}.vault.event.metadata.>    (broad access)
    ├──► fc.{org}.vault.event.content.{tag} (scoped access)
    ├──► monitoring / alerting
    └──► future consumers
```

The vault publishes to one subject. The FC Message Bus Service reads that subject and routes to scoped downstream subjects based on RBAC policy. The vault does not know about downstream consumers or access control topology.

**Why the vault doesn't route:** The vault stays simple (one responsibility, one stream). Policy changes don't touch the vault. Scaling is independent. The primary stream has one authorized consumer (message bus). Security boundary is clear and auditable.

### Event Envelope

Uses the existing `FleetEvent` from `fc-events`:

```rust
FleetEvent {
    specversion: "1.0",
    id: "unique-event-id",
    event_type: "vault.note.updated",
    source: "fc.vault",
    time: "2026-03-27T19:45:00Z",
    org_id: OrgId("default"),
    trace_id: Some("trace-123"),
    data: { /* event-specific payload */ },
}
```

### Event Types

#### Read Events

No content included. Signal demand and access patterns.

| Event type | Data fields |
|-----------|-------------|
| `vault.note.read` | `path`, `actor`, `version_read` |
| `vault.note.search` | `query`, `result_count`, `result_paths`, `actor` |
| `vault.note.list` | `directory_path`, `result_count`, `actor` |
| `vault.note.links` | `path`, `direction` (forward/back), `result_count`, `actor` |

#### Write Events

Include version transitions. Update events include content diffs.

| Event type | Data fields |
|-----------|-------------|
| `vault.note.created` | `path`, `version` (1), `hash`, `actor`, `tags` |
| `vault.note.updated` | `path`, `version_before`, `version_after`, `hash_before`, `hash_after`, `diff`, `actor`, `tags` |
| `vault.note.moved` | `path_before`, `path_after`, `version`, `actor` |
| `vault.note.deleted` | `path`, `final_version`, `final_hash`, `actor` |
| `vault.note.archived` | `path`, `final_version`, `final_hash`, `actor` |

#### Access Failure Events

Security-relevant. Published on any denied operation.

| Event type | Data fields |
|-----------|-------------|
| `vault.access.denied` | `path` (if applicable), `operation` (read/write/delete/etc), `actor`, `reason` |

#### Backup Events

Published by the vault server when snapshot operations occur.

| Event type | Data fields |
|-----------|-------------|
| `vault.snapshot.created` | `snapshot_id`, `selection_criteria`, `target_location`, `note_count`, `total_hash`, `actor` |
| `vault.snapshot.restored` | `snapshot_id`, `restore_mode` (staging/in-place), `note_count`, `actor` |
| `vault.snapshot.deleted` | `snapshot_id`, `actor` |
| `vault.snapshot.listed` | `actor` |

### Publishing Pattern

Follows the established FC pattern. New struct in the vault codebase:

```rust
pub struct VaultEventPublisher {
    bus: Arc<RwLock<Option<EventBus>>>,
    queue: Arc<LocalEventQueue>,  // disk-backed queue for resilience
}

impl VaultEventPublisher {
    pub async fn publish(&self, event_type: &str, org_id: &str, data: impl Serialize) {
        let event = FleetEvent::new("fc.vault", event_type, OrgId(org_id.into()), &data);
        if let Some(bus) = self.bus.read().await.as_ref() {
            match bus.publish(&event).await {
                Ok(_) => { /* delivered */ },
                Err(e) => {
                    warn!(event_type, error = %e, "NATS publish failed, queuing locally");
                    self.queue.enqueue(event).await;
                }
            }
        } else {
            // No NATS connection at all — queue for later delivery
            self.queue.enqueue(event).await;
        }
    }
}
```

### Local Event Queue (Resilience)

The vault's NATS client maintains a local queue for events that fail to publish (NATS/message bus unavailable):

- On publish failure: event is queued in a local bounded buffer
- Buffer backed by disk persistence (append-only file) to survive vault restarts
- When NATS connectivity resumes: drain queue in order, oldest first
- Events are delivered in order — consumers see the complete history, delayed but intact
- Configurable queue size limit with disk spillover

**Rationale (Tenet #0):** If denied access attempts can be silently lost during a NATS outage, that's a security gap. The activity stream must be complete.

**Failure mode:** Vault operations always succeed regardless of NATS state. Publishing is async and non-blocking. The local queue absorbs temporary outages. Only a catastrophic scenario (queue full + disk full + prolonged outage) would lose events, and that would be logged locally as an alert.

### Activity Stream Security

The primary stream (`fc.{org}.vault.activity`) contains complete content diffs for write events. If you can read this stream, you can reconstruct any note. Therefore:

- The primary stream has exactly **one authorized consumer**: the FC Message Bus Service
- The message bus strips content for the metadata stream (`fc.{org}.vault.event.metadata.>`) — broader access
- The message bus applies tag/path-based RBAC for scoped content streams
- The vault publishes everything; the message bus decides who sees what

This separation means RBAC policy changes never require vault restarts or redeployment.

---

## 3. Snapshot CLI

### Binary Location

`tools/vault-snapshot/` in the fleet-control Cargo workspace. Built alongside `fleet-cli` and `fc-hostd`. Follows the same patterns (Clap 4, async Tokio runtime).

Binary name: `vault-snapshot`

### Commands

```
vault-snapshot create [OPTIONS]
vault-snapshot restore <SNAPSHOT_ID> [OPTIONS]
vault-snapshot list [OPTIONS]
vault-snapshot inspect <SNAPSHOT_ID>
vault-snapshot delete <SNAPSHOT_ID> [OPTIONS]
```

#### `create`

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--tags` | `Vec<String>` | None (whole vault) | Filter notes by frontmatter tags (OR logic) |
| `--target` | URI string | Server configured default | Where to write the archive. `file:///path/` for local/NAS. |
| `--org` | String | `default` | Organization scope |
| `--vault-url` | URL | `http://vault.home.iramsay.com:3000` | FC Vault server address |

**Flow:**
1. CLI sends `POST /v1/snapshots` to vault server with selection criteria and target URI
2. Vault server selects notes (whole vault or tag-filtered using existing `advanced_search`)
3. Vault server packages archive: notes with directory structure + manifest
4. Vault server writes archive to target location
5. Vault server publishes `vault.snapshot.created` event
6. Vault server returns manifest to CLI
7. CLI prints summary: snapshot ID, note count, size, location

#### `restore`

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--mode` | `staging` or `in-place` | `staging` | Restore strategy |
| `--org` | String | `default` | Organization scope |
| `--vault-url` | URL | configured default | FC Vault server address |

**Staging mode (default, 2-way door):**
- Vault server extracts snapshot to `_restore/{snapshot_id}/` inside the vault
- Nothing is overwritten. Human or agent reviews, then decides to merge or discard.
- CLI prints: "Restored to _restore/{snapshot_id}/. Review and merge manually, or delete to discard."

**In-place mode (1-way door):**
- CLI requires explicit `--mode in-place` flag
- CLI prompts for confirmation: "This will overwrite {N} notes. Type 'yes' to proceed."
- Vault server creates an automatic pre-restore snapshot before executing (Tenet #0 — protect against data loss from authorized operations)
- Vault server overwrites current notes with snapshot versions, bumping versions and recording history entries with `by: "restore:{snapshot_id}"`
- Vault server publishes `vault.snapshot.restored` event
- CLI prints: "Restored {N} notes in-place. Pre-restore snapshot saved as {auto_snapshot_id}."

#### `list`

Lists all snapshots accessible to the caller. Returns: snapshot ID, creation time, creator, selection criteria, note count, location.

#### `inspect`

Shows full manifest for a snapshot: all included notes with versions and hashes, boundary links, selection criteria.

#### `delete`

Deletes a snapshot archive. Publishes `vault.snapshot.deleted` event.

### Vault Server API Endpoints

New endpoints on the FC Vault server:

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/snapshots` | Create a snapshot. Body: selection criteria + target URI. Returns manifest. |
| `GET` | `/v1/snapshots` | List snapshots. Query params: `org_id`. |
| `GET` | `/v1/snapshots/{id}` | Get snapshot manifest. |
| `POST` | `/v1/snapshots/{id}/restore` | Restore a snapshot. Body: `mode` (staging/in-place). |
| `DELETE` | `/v1/snapshots/{id}` | Delete a snapshot archive. |

### Snapshot Archive Format

**Archive:** tar.gz containing:

```
{snapshot_id}/
  manifest.json
  notes/
    Focus Areas/
      Projects/
        FC Roadmap.md
        FC Architecture.md
    Cabinet/
      AI/
        Shared Agent Notes.md
```

Directory structure inside `notes/` mirrors the vault path structure exactly.

### Manifest Format

```json
{
  "snapshot_id": "20260327-194500-fleet-control",
  "format_version": 1,
  "org_id": "default",
  "created_at": "2026-03-27T19:45:00Z",
  "created_by": "uisang",
  "selection": {
    "type": "tags",
    "tags": ["fleet-control"]
  },
  "target": "file:///mnt/nas/vault-snapshots/",
  "note_count": 47,
  "total_size_bytes": 524288,
  "total_hash": "sha256:e4f8a1c2d3b5...",
  "notes": [
    {
      "path": "Focus Areas/Projects/FC Roadmap.md",
      "version": 12,
      "hash": "sha256:a3f8c2d1e5b7...",
      "size_bytes": 51536,
      "tags": ["fleet-control", "roadmap", "architecture"]
    }
  ],
  "boundary_links": [
    {
      "from": "Focus Areas/Projects/FC Roadmap.md",
      "to": "Hwabaek Council Architecture.md",
      "link_type": "wikilink",
      "included": false
    }
  ]
}
```

**`format_version`:** Allows future manifest format changes without breaking older snapshots.

**`boundary_links`:** Wikilinks that point from included notes to notes NOT in the snapshot. Makes it explicit what won't resolve if the snapshot is restored in isolation. Essential for export/import and cross-org sharing later.

**`total_hash`:** SHA-256 of all note hashes concatenated in sorted path order. Integrity check for the entire snapshot.

### Target URI Scheme

| Scheme | MLP | Later | Description |
|--------|-----|-------|-------------|
| `file://` | Yes | — | Local filesystem or NAS mount point |
| `s3://` | No | Post-MLP | S3-compatible object store |
| `fc://` | No | Post-MLP | Cross-org FC instance |

The vault server resolves the URI and writes to the appropriate backend. `file://` is the only writer implemented in the MLP. The URI scheme is the extension point (Tenet #7).

Default target is configured in the vault server's config file. CLI `--target` overrides.

### Org Scoping

Every snapshot operation is org-scoped:

- `create`: only includes notes the org is authorized to access
- `restore`: only restores into the org's scope
- `list`: only shows snapshots belonging to the org
- `delete`: only deletes snapshots belonging to the org

MLP operates as single-org (`default`). The data model and API carry `org_id` from day one so multi-org is a policy change, not a structural one.

---

## 4. Implementation Considerations

### Where Code Lives

| Component | Location | Rationale |
|-----------|----------|-----------|
| Note versioning (frontmatter read/write, version enforcement) | FC Vault (`/Users/max/projects/turbovault`) | Core vault behavior |
| VaultEventPublisher + event types | FC Vault, using `fc-events` crate as dependency | Follows FC service pattern |
| Local event queue (disk-backed) | FC Vault | Vault-owned resilience |
| Snapshot API endpoints | FC Vault | Vault is authority on its own state |
| Snapshot CLI binary | Fleet Control (`/Users/max/projects/fleet-control/tools/vault-snapshot/`) | Follows `fleet-cli` pattern |
| Snapshot archive writer (`file://`) | FC Vault | Server-side, not CLI-side |

### Dependencies to Add

FC Vault gains dependencies on:
- `fc-events` crate (for `FleetEvent`, `EventBus`)
- `fc-common` crate (for `OrgId`)
- `async-nats` (transitive via `fc-events`)

These are workspace-internal dependencies from fleet-control. This means the FC Vault either:
- **(A)** Joins the fleet-control Cargo workspace, or
- **(B)** Depends on fc-events/fc-common as path dependencies or published crates

Decision needed during implementation planning.

### Existing Code Affected

| Current code | Change |
|-------------|--------|
| All write tool handlers (`write_note`, `patch_note`, `move_file`, `delete_note`) | Add version enforcement + history append + event publish |
| All read tool handlers (`read_note`, `search`, `list_files`, `get_backlinks`, etc.) | Add event publish (read events) |
| Frontmatter parser | Support reading/writing `version` and `history` fields |
| YAML frontmatter serialization | Preserve existing fields while adding version metadata |
| HTTP/MCP server startup | Initialize NATS connection + VaultEventPublisher |
| Config/CLI flags | Add NATS URL, org_id, default snapshot target |

### Testing Strategy

| Layer | What to test | How |
|-------|-------------|-----|
| Version enforcement | Concurrent writes, version mismatch → 409, hash computation, history append | Unit tests + integration tests |
| Event publishing | Correct event type/data for each operation, queue behavior on NATS failure | Unit tests with mock EventBus + integration tests with embedded NATS |
| Snapshot create | Whole vault, tag-filtered, manifest correctness, boundary link detection, archive integrity | Integration tests with test vault |
| Snapshot restore | Staging extraction, in-place with auto-backup, version history entries | Integration tests |
| Out-of-band detection | File modified outside API, version bump on next access | Integration tests |
| Migration | Pre-versioning notes get version 0/1 on access | Integration tests |

---

## 5. Forward Dependencies (Not in MLP)

Captured here so they're tracked, not forgotten (Tenet #9: Manage Debt, Don't Ignore It).

| Dependency | What it enables | Urgency |
|-----------|----------------|---------|
| **Multi-org isolation spec** | Shared infrastructure with org-scoped vault access. Currently org_id is in the data model but isolation is not enforced. Needs a cross-service design pass covering all FC services. | High — design soon |
| **FC Identity integration** | Auth for snapshot CLI and vault API. Currently LAN trust. | Medium — before exposing beyond LAN |
| **Message Bus stream routing** | Scoped downstream subjects (metadata vs. content, tag-based filtering). Currently the vault publishes; nobody consumes. | Medium — when first downstream consumer exists |
| **`s3://` target writer** | Object store snapshots for off-site DR. | Low — `file://` with NAS mount is sufficient |
| **Graph-neighborhood selection** | Snapshot a note + N hops of related notes. | Low — tag selection covers MLP use cases |
| **Search-based selection** | Snapshot arbitrary search result sets. | Low |
| **Date-range selection** | Snapshot notes modified in a time window. | Low |
| **Export/import across vaults** | Reconciliation, link resolution, namespace handling. | Low — builds on snapshot format |
| **Tag AND logic in search** | Currently OR only. AND would help for precise snapshot selection. | Low — OR is sufficient for MLP |
| **Metadata filtering in MCP tool** | `advanced_search` accepts tags but not frontmatter filters, despite internal support. Quick win. | Low — not blocking |

---

## 6. Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Version storage | Frontmatter (not sidecar DB) | Single source of truth. No sync problem. File IS the state. |
| Version type | Monotonic integer | Total ordering for stream consumers. "v7 > v6" is trivial. |
| Hash scope | Content body only (below frontmatter) | Metadata changes don't affect content hash. Clean audit. |
| History storage | Unbounded array in frontmatter | Simple. Curator handles compaction later if needed. |
| NATS connection | Direct (vault is an FC service) | Vault Curator requires FC. Vault is part of the fleet. |
| Stream topology | One primary subject, message bus routes | Vault stays simple. Policy changes don't touch vault. |
| Event queue on failure | Local disk-backed queue with drain | No silent event loss. Security requirement. |
| Snapshot creation | Server-side (vault writes to target) | Vault is authority. No streaming through HTTP. |
| Snapshot target | URI scheme (`file://`, extensible) | Interface over implementation. |
| Restore safety | Auto pre-restore snapshot on in-place | Protect against data loss from authorized ops. |
| CLI location | fleet-control workspace | Follows fleet-cli pattern. |
| Org scoping | org_id on everything, single-org MLP | Think Big. Multi-org is a policy change, not structural. |

## 7. Not in Scope

- Multi-org isolation enforcement (forward dependency — needs cross-service spec)
- Authentication/authorization on vault API or CLI (LAN trust for MLP)
- Message bus stream routing implementation (vault publishes; routing is message bus concern)
- Semantic search / embeddings
- Governed writes / policy enforcement on vault mutations
- Cross-vault export/import and reconciliation
- Subscription feeds and push notifications
- Knowledge marketplace
