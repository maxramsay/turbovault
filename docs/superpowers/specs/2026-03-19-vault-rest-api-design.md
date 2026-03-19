# Vault REST API v1 — Design Spec

**Date**: 2026-03-19
**Status**: Approved
**Author**: Max + Uisang

## Problem

Agents that cannot speak MCP natively (e.g., Scout on OpenClaw) need HTTP REST access to the vault. The retired Obsidian REST API was tightly coupled to Obsidian's implementation. We need a vault-native REST API that is implementation-agnostic, versioned, and shares the same engine as the MCP tools.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| API style | Vault-native, not Obsidian-compatible | Agents talk to "the vault," not to "Obsidian." Decouples from upstream. |
| Versioning | `/v1/` prefix | Enables non-breaking evolution. New versions coexist. |
| Crate | `turbovault-rest` (new, dedicated) | Versioned API deserves its own boundary. REST and MCP are peers. |
| Port | Same as MCP (3000) | Single endpoint, no operational overhead. Routes don't conflict. |
| Naming | No "TurboVault" in API surface | Implementation-agnostic. MCP server renamed from `turbovault` to `vault`. |
| Delete model | Soft delete (trash) by default | Permanent delete deferred to future curator agent. |
| Content negotiation | JSON and raw markdown for writes | Avoids escaping bugs when LLMs construct curl commands. |
| Auth | Optional Bearer token, env-configured | LAN trust for now. Proper auth via Fleet Control later. |
| Multi-vault | Default to active vault, `X-Vault` header for override | Engine supports multi-vault; REST API must expose this. |
| Concurrency | Optional `If-Match` header for optimistic locking | Prevents silent overwrites when MCP and REST operate on same notes. |
| Pagination | `limit`/`offset` on all list endpoints | Vault has 4000+ files; unbounded responses are a risk. |

## Architecture

```
Port 3000 (single Axum server)
├── POST /mcp              → MCP JSON-RPC (TurboMCP, existing)
├── GET  /sse              → MCP SSE stream (TurboMCP, existing)
└── /v1/                   → REST API (turbovault-rest, new)
    ├── Notes CRUD
    ├── Search
    ├── Navigation (links, files, periodic, recent)
    ├── Trash lifecycle
    ├── Batch operations
    └── Health
```

Both interfaces share the same `MultiVaultManager` and vault engine (`FileTools`, `SearchEngine`, `GraphTools`, etc.). No separate logic — REST handlers are thin wrappers that call engine functions and format HTTP responses.

### Router Merge

TurboMCP's builder exposes `into_axum_router()` (`turbomcp-server/src/builder.rs`), which returns a standard Axum Router for the MCP endpoints. The REST crate exports its own Router. Both are merged in `main.rs` via Axum's `.merge()`. This replaces the current `run_http()` call which owns the full server lifecycle.

### Multi-Vault Targeting

REST requests target the active vault by default. To target a specific vault, include an `X-Vault: {name}` header. If the named vault does not exist, return `404` with error code `VAULT_NOT_FOUND`. The response envelope's `vault` field always indicates which vault was used.

### Optimistic Concurrency

Read responses include a `hash` field (content hash). Write endpoints (`PUT`, `POST`, `PATCH`, `DELETE`) accept an optional `If-Match` header containing the expected hash. If provided and the current content hash does not match, the server returns `409 CONFLICT` with error code `HASH_MISMATCH`. If omitted, the write proceeds unconditionally (last-writer-wins).

### Pagination

List endpoints (`search`, `files`, `recent`, `trash`) accept `?limit=N&offset=N` query parameters. Defaults: `limit=50`, `offset=0`. Responses include `count` (total matching items) and `has_more` (boolean) alongside the data array.

## Endpoints

All `{*path}` parameters use Axum catch-all syntax to support nested vault paths (e.g., `Focus Areas/Projects/Bootstrap.md`).

### Notes

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/notes/{*path}` | Read note content. Returns content + hash. |
| `PUT` | `/v1/notes/{*path}` | Create or overwrite note. |
| `POST` | `/v1/notes/{*path}` | Append to existing note. Returns `404` if note does not exist (append-only, no implicit creation — use `PUT` for creation). |
| `PATCH` | `/v1/notes/{*path}` | Heading-aware insert (patch). Requires `target_type`, `target`, `operation` fields. |
| `DELETE` | `/v1/notes/{*path}` | Soft delete (move to `.trash/`). |
| `GET` | `/v1/notes-info/{*path}` | Metadata only (size, modified, frontmatter presence). No content. Separate path prefix avoids catch-all route conflict with `/v1/notes/{*path}`. |

### Search

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/search?q={query}&limit=N&offset=N` | Full-text BM25 search. Returns path, title, snippet, score per result. Paginated. |

### Navigation

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/files/{*path}` | List files and subdirectories at path. |
| `GET` | `/v1/files` | List files and subdirectories at vault root. |
| `GET` | `/v1/periodic/{period}` | Get periodic note (daily, weekly, monthly). Optional `?date=YYYY-MM-DD`. Returns content + path. |
| `GET` | `/v1/recent?days=N&limit=N&offset=N` | Recently modified notes. Defaults: days=7, limit=50. Returns path, modified timestamp, size per entry. Paginated. |
| `GET` | `/v1/links/{*path}/backlinks` | Notes that link TO this note. |
| `GET` | `/v1/links/{*path}/forward` | Notes this note links TO. |

Note: `/v1/links/{*path}/backlinks` and `/v1/links/{*path}/forward` use a fixed suffix after the catch-all. In Axum, this requires separate route registration with the suffix as part of the catch-all parsing — the handler extracts the path and strips the suffix.

### Trash Lifecycle

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/trash?limit=N&offset=N` | List trashed notes with metadata (original path, deleted date, orphaned links). Paginated. |
| `POST` | `/v1/restore/{*trash-path}` | Restore note from trash to original location. |
| `POST` | `/v1/trash/{*path}/request-purge` | Request permanent deletion. **Logs request, returns 202 Accepted.** Does not execute — awaits curator agent. Uses POST (not DELETE) because the operation is non-destructive. |

### Batch

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/batch/read` | Read multiple notes in one request. Body: `{"paths": ["path1.md", "path2.md"]}`. Max 50 paths per request. |

### Health

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/health` | Health check. Always open (no auth). Returns vault name, note count, uptime. |

## Delete & Restore Behavior

### Soft Delete (default)

1. File moved to `.trash/{original-path}` with timestamp suffix for collision handling.
2. Entry written to `.trash/.manifest.json` recording original path, deletion time, and orphaned links.
3. Search and graph indexes updated immediately — trashed files excluded from queries.
4. Response includes `orphaned_links` (list of notes with now-broken wikilinks) and `moved_to` (trash path).

### Restore

1. File moved from `.trash/` back to original location (read from manifest).
2. Manifest entry removed.
3. Search and graph indexes updated.
4. Response includes `restored_to` and `previously_orphaned_links` so caller can verify link integrity.

### Permanent Delete (deferred)

1. `POST /v1/trash/{path}/request-purge` logs the request and marks the manifest entry with `permanent_delete_requested` timestamp.
2. Returns `202 Accepted` with status `pending`.
3. No file is deleted. Awaits future curator agent to process.

### Trash Manifest

```json
{
  "entries": [
    {
      "original_path": "Focus Areas/Projects/Old Note.md",
      "trash_path": ".trash/Focus Areas/Projects/Old Note.md.1710853200",
      "deleted_at": "2026-03-19T15:30:00Z",
      "orphaned_links": [
        "Daily/2026-03-19.md",
        "Focus Areas/Projects/Core Infrastructure Bootstrap.md"
      ],
      "permanent_delete_requested": null
    }
  ]
}
```

## Content Negotiation (Write Endpoints)

Write endpoints (`PUT`, `POST`, `PATCH` on `/v1/notes/{path}`) accept two content types:

### `Content-Type: application/json`

For `PUT` (create/overwrite):

```json
{
  "content": "# Note Title\n\nBody text with [[wikilinks]]."
}
```

For `POST` (append — note must already exist):

```json
{
  "content": "\n## New Section\n\nAppended content."
}
```

For `PATCH`:

```json
{
  "target_type": "heading",
  "target": "Session Notes",
  "operation": "append",
  "content": "New content under this heading."
}
```

### `Content-Type: text/markdown`

Raw markdown body. The HTTP method determines the operation: `PUT` = create/overwrite, `POST` = append.

For `PATCH` with raw markdown: query params `?target_type=heading&target=Session+Notes&operation=append`.

### Response

All responses are always `Content-Type: application/json` using the standard envelope.

## Response Envelope

All responses use a consistent JSON structure:

### Success

```json
{
  "vault": "default",
  "operation": "read_note",
  "success": true,
  "data": {
    "path": "Daily/2026-03-19.md",
    "content": "...",
    "hash": "abc123..."
  },
  "count": null,
  "took_ms": 12
}
```

### Error

```json
{
  "success": false,
  "error": {
    "code": "NOT_FOUND",
    "message": "Note not found: Focus Areas/Projects/Nonexistent.md"
  }
}
```

### Error Codes

| Situation | HTTP Status | Error Code |
|-----------|-------------|------------|
| Note not found | 404 | `NOT_FOUND` |
| Vault not found (bad `X-Vault` header) | 404 | `VAULT_NOT_FOUND` |
| Invalid path (traversal, `.obsidian/`, etc.) | 400 | `INVALID_PATH` |
| Protected path (`Focus Areas/Writing/`) | 403 | `FORBIDDEN` |
| Missing required field | 400 | `INVALID_REQUEST` |
| Auth token missing/invalid | 401 | `UNAUTHORIZED` |
| Hash mismatch (`If-Match` failed) | 409 | `HASH_MISMATCH` |
| Conflict (write to trashed note, etc.) | 409 | `CONFLICT` |
| Vault engine error | 500 | `INTERNAL_ERROR` |

## Authentication

- **Mechanism**: Bearer token via `Authorization: Bearer <token>` header.
- **Configuration**: `VAULT_API_TOKEN` environment variable. If unset, all requests allowed (LAN trust mode).
- **Scope**: All `/v1/` endpoints except `/v1/health`.
- **Future**: Replace with scoped tokens / JWT as part of Fleet Control Architecture.

## Business Rules

v1 hardcodes one rule as proof of concept:

- **`Focus Areas/Writing/` protection**: Write operations (PUT, POST, PATCH, DELETE) to paths under `Focus Areas/Writing/` return `403 FORBIDDEN`. Read operations are allowed.

Future: Externalize rules via Fleet Control workflow engine. Rules become configurable, not hardcoded.

## Crate Structure

```
crates/turbovault-rest/
├── Cargo.toml
├── src/
│   ├── lib.rs              ← exports router() builder function
│   ├── v1/
│   │   ├── mod.rs          ← v1 route tree
│   │   ├── notes.rs        ← GET/PUT/POST/PATCH/DELETE + info
│   │   ├── search.rs       ← GET search
│   │   ├── files.rs        ← GET directory listing
│   │   ├── periodic.rs     ← GET periodic notes
│   │   ├── links.rs        ← GET backlinks, forward links
│   │   ├── trash.rs        ← GET trash, POST restore, POST request-purge
│   │   ├── batch.rs        ← POST batch read
│   │   └── health.rs       ← GET health
│   ├── auth.rs             ← Bearer token middleware (Axum layer)
│   ├── errors.rs           ← Error types → HTTP status + error code mapping
│   └── content.rs          ← Content-type negotiation (JSON vs text/markdown)
└── tests/
    ├── test_notes_crud.rs   ← Read, write, overwrite, append lifecycle
    ├── test_notes_patch.rs  ← Heading-aware patch operations
    ├── test_trash.rs        ← Delete, restore, permanent delete request
    ├── test_search.rs       ← Search queries and result format
    ├── test_links.rs        ← Backlinks, forward links
    ├── test_batch.rs        ← Batch read
    ├── test_auth.rs         ← Token enforcement, health bypass
    └── test_errors.rs       ← Error codes, protected paths, invalid input
```

### Dependencies

```toml
[dependencies]
axum = "0.8"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.6" }
turbovault-core = { path = "../turbovault-core" }
turbovault-tools = { path = "../turbovault-tools" }
turbovault-vault = { path = "../turbovault-vault" }
turbovault-graph = { path = "../turbovault-graph" }
```

### Integration in main.rs

```rust
use turbovault_rest::RestConfig;

// Existing MCP router
let mcp_router = server.builder().into_axum_router();

// REST router (shares vault engine)
let rest_config = RestConfig {
    api_token: std::env::var("VAULT_API_TOKEN").ok(),
    protected_paths: vec!["Focus Areas/Writing/".to_string()],
};
let rest_router = turbovault_rest::router(server.multi_vault(), rest_config);

// Merge on same port
let app = mcp_router.merge(rest_router);
let listener = TcpListener::bind(&addr).await?;
axum::serve(listener, app).await?;
```

## Testing Strategy

### Layer 1: Unit Tests (turbovault-rest crate)

Test each handler in isolation using Axum's `TestClient`. Mock vault engine where needed. Validates:
- Request parsing (JSON and markdown content types)
- Content-type negotiation
- Error mapping (engine errors → HTTP status codes)
- Response envelope formatting
- Auth middleware (token present/absent/invalid)
- Protected path enforcement

### Layer 2: Integration Tests (turbovault-rest crate)

Spin up real Axum server with a temporary vault directory. Make actual HTTP requests. Validates:
- Full CRUD lifecycle (create → read → update → delete → verify gone)
- Trash lifecycle (delete → list trash → restore → verify restored)
- Search indexing (write note → search → find it)
- Graph queries (write notes with wikilinks → query backlinks/forward links)
- Batch read (multiple notes in one request)
- Concurrent access (multiple requests to same note)

### Layer 3: Cross-Interface Tests (extend Python suite)

Write via REST, read via MCP. Write via MCP, read via REST. Validates:
- Both interfaces see identical vault state
- Content written by one interface is correctly formatted for the other
- Search indexes are consistent across interfaces
- Graph state is consistent across interfaces

This is the most critical layer — it catches divergence between the two interfaces.

## Backlog (Out of Scope for v1)

| Item | Description | Tracked In |
|------|-------------|------------|
| **Vault Curator Agent** | Agent for vault hygiene: broken links, stale notes, trash cleanup, permanent deletion | Daily note 2026-03-19 |
| **Auth & Authorization** | Scoped tokens, per-agent permissions, audit logging via Fleet Control | Daily note 2026-03-19, Fleet Control Architecture |
| **Business Rules / Workflow** | Externalized rules engine, approval workflows for destructive operations | Daily note 2026-03-19, Fleet Control Architecture |
| **Batch Write** | Atomic multi-note writes via REST (complex — MCP handles this today) | — |
| **Edit (search/replace)** | In-note text replacement via REST | — |
| **Restore link repair** | Automatically repair orphaned links on restore | — |
| **Rate limiting** | Per-client rate limiting to prevent runaway agent loops | — |
| **CORS policy** | Configure CORS if browser-based clients are added | — |
| **Frontmatter endpoint** | Dedicated read/write for frontmatter without full note content | — |
