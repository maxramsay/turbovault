# Vault REST API v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a versioned REST API (`/v1/`) to TurboVault that runs alongside MCP on the same port, enabling agents that cannot speak MCP to access the vault via standard HTTP.

**Architecture:** New `turbovault-rest` crate exports an Axum Router merged with TurboMCP's MCP router in `main.rs`. REST handlers are thin wrappers calling the same vault engine (`FileTools`, `SearchEngine`, `GraphTools`) used by MCP tools. Trash lifecycle uses `.trash/` directory with a JSON manifest.

**Tech Stack:** Rust, Axum 0.8, serde/serde_json, tokio, tower-http. Depends on existing workspace crates: turbovault-core, turbovault-tools, turbovault-vault, turbovault-graph.

**Spec:** `docs/superpowers/specs/2026-03-19-vault-rest-api-design.md`

---

## File Map

### New Files (turbovault-rest crate)

| File | Responsibility |
|------|---------------|
| `crates/turbovault-rest/Cargo.toml` | Crate dependencies and metadata |
| `crates/turbovault-rest/src/lib.rs` | Public API: `router()` function, `RestConfig` struct |
| `crates/turbovault-rest/src/state.rs` | Shared Axum state: vault manager reference, config |
| `crates/turbovault-rest/src/errors.rs` | `ApiError` enum → HTTP status + JSON error response |
| `crates/turbovault-rest/src/response.rs` | `ApiResponse<T>` envelope, serialization helpers |
| `crates/turbovault-rest/src/content.rs` | Content-type negotiation: parse JSON or raw markdown from request body |
| `crates/turbovault-rest/src/auth.rs` | Bearer token middleware (Axum layer) |
| `crates/turbovault-rest/src/vault_resolver.rs` | Extract vault manager from `X-Vault` header or active vault |
| `crates/turbovault-rest/src/pagination.rs` | `PaginationParams` extractor, `PaginatedResponse` wrapper |
| `crates/turbovault-rest/src/v1/mod.rs` | V1 route tree assembly |
| `crates/turbovault-rest/src/v1/health.rs` | `GET /v1/health` |
| `crates/turbovault-rest/src/v1/notes.rs` | `GET/PUT/POST/PATCH/DELETE /v1/notes/{*path}` |
| `crates/turbovault-rest/src/v1/notes_info.rs` | `GET /v1/notes-info/{*path}` |
| `crates/turbovault-rest/src/v1/search.rs` | `GET /v1/search` |
| `crates/turbovault-rest/src/v1/files.rs` | `GET /v1/files`, `GET /v1/files/{*path}` |
| `crates/turbovault-rest/src/v1/periodic.rs` | `GET /v1/periodic/{period}` |
| `crates/turbovault-rest/src/v1/recent.rs` | `GET /v1/recent` |
| `crates/turbovault-rest/src/v1/links.rs` | `GET /v1/links/{*path}/backlinks`, `GET /v1/links/{*path}/forward` |
| `crates/turbovault-rest/src/v1/trash.rs` | `GET /v1/trash`, `POST /v1/restore/{*path}`, `POST /v1/trash/{*path}/request-purge` |
| `crates/turbovault-rest/src/v1/batch.rs` | `POST /v1/batch/read` |
| `crates/turbovault-rest/src/trash_manifest.rs` | Trash manifest read/write (`TrashManifest`, `TrashEntry`) |
| `crates/turbovault-rest/tests/helpers/mod.rs` | Test helpers: create temp vault, build test app, make requests |
| `crates/turbovault-rest/tests/test_health.rs` | Health endpoint tests |
| `crates/turbovault-rest/tests/test_notes_crud.rs` | Notes read/write/append/delete lifecycle |
| `crates/turbovault-rest/tests/test_notes_patch.rs` | Heading-aware patch tests |
| `crates/turbovault-rest/tests/test_trash.rs` | Trash lifecycle tests |
| `crates/turbovault-rest/tests/test_search.rs` | Search endpoint tests |
| `crates/turbovault-rest/tests/test_links.rs` | Backlinks/forward links tests |
| `crates/turbovault-rest/tests/test_batch.rs` | Batch read tests |
| `crates/turbovault-rest/tests/test_auth.rs` | Auth middleware tests |
| `crates/turbovault-rest/tests/test_errors.rs` | Error codes, protected paths, invalid input |

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace root) | Add `crates/turbovault-rest` to workspace members, add `turbovault-rest` to workspace dependencies |
| `crates/turbovault/Cargo.toml` | Add `turbovault-rest` dependency |
| `crates/turbovault/src/bin/main.rs` | Replace `run_http()` with builder pattern: `into_axum_router()` + REST router merge + manual `axum::serve()` |

---

## Task 1: Crate Scaffolding + Compilation

**Files:**
- Create: `crates/turbovault-rest/Cargo.toml`
- Create: `crates/turbovault-rest/src/lib.rs`
- Create: `crates/turbovault-rest/src/state.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml for turbovault-rest**

```toml
[package]
name = "turbovault-rest"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
license.workspace = true
description = "Versioned REST API for TurboVault vault access"

[dependencies]
axum = "0.8"
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tower-http = { version = "0.6", features = ["trace"] }
tower = "0.5"
chrono.workspace = true
sha2.workspace = true
log.workspace = true
thiserror.workspace = true
turbovault-core = { path = "../turbovault-core" }
turbovault-tools = { path = "../turbovault-tools" }
turbovault-vault = { path = "../turbovault-vault" }
turbovault-graph = { path = "../turbovault-graph" }

[dev-dependencies]
tokio = { workspace = true, features = ["test-util"] }
tempfile.workspace = true
axum-test = "16"
```

- [ ] **Step 2: Create state.rs with shared state**

```rust
use std::sync::Arc;
use turbovault_core::prelude::MultiVaultManager;

/// Configuration for the REST API
#[derive(Clone, Debug)]
pub struct RestConfig {
    /// Optional Bearer token for auth. None = allow all (LAN trust).
    pub api_token: Option<String>,
    /// Paths that reject write operations (e.g., "Focus Areas/Writing/")
    pub protected_paths: Vec<String>,
}

/// Shared state for all REST handlers
#[derive(Clone)]
pub struct AppState {
    pub multi_vault: Arc<MultiVaultManager>,
    pub config: RestConfig,
    pub start_time: std::time::Instant,
}
```

- [ ] **Step 3: Create lib.rs with placeholder router**

```rust
pub mod state;

use axum::Router;
use state::{AppState, RestConfig};
use std::sync::Arc;
use turbovault_core::prelude::MultiVaultManager;

pub use state::RestConfig;

/// Build the REST API router. Merge with MCP router in main.rs.
pub fn router(multi_vault: Arc<MultiVaultManager>, config: RestConfig) -> Router {
    let state = AppState {
        multi_vault,
        config,
        start_time: std::time::Instant::now(),
    };

    Router::new().with_state(state)
}
```

- [ ] **Step 4: Add to workspace**

Add `"crates/turbovault-rest"` to `members` in root `Cargo.toml`.
Add `turbovault-rest = { version = "1.2.8", path = "crates/turbovault-rest" }` to `[workspace.dependencies]`.

- [ ] **Step 5: Verify compilation**

Run: `cd ~/projects/turbovault && cargo check -p turbovault-rest`
Expected: Compiles successfully.

- [ ] **Step 6: Commit**

```bash
git add crates/turbovault-rest/ Cargo.toml
git commit -m "feat(rest): scaffold turbovault-rest crate with state and empty router"
```

---

## Task 2: Error Types + Response Envelope

**Files:**
- Create: `crates/turbovault-rest/src/errors.rs`
- Create: `crates/turbovault-rest/src/response.rs`
- Create: `crates/turbovault-rest/src/pagination.rs`

- [ ] **Step 1: Create errors.rs**

Define `ApiError` enum with variants for each error code from the spec (`NotFound`, `VaultNotFound`, `InvalidPath`, `Forbidden`, `InvalidRequest`, `Unauthorized`, `HashMismatch`, `Conflict`, `InternalError`). Implement `IntoResponse` for Axum — each variant maps to an HTTP status code and returns the JSON error envelope.

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Note not found: {0}")]
    NotFound(String),
    #[error("Vault not found: {0}")]
    VaultNotFound(String),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Hash mismatch: content has changed")]
    HashMismatch,
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

impl ApiError {
    fn status_code(&self) -> StatusCode { /* match each variant */ }
    fn error_code(&self) -> &'static str { /* "NOT_FOUND", "VAULT_NOT_FOUND", etc. */ }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "success": false,
            "error": {
                "code": self.error_code(),
                "message": self.to_string(),
            }
        });
        (self.status_code(), axum::Json(body)).into_response()
    }
}
```

- [ ] **Step 2: Create response.rs**

Define `ApiResponse<T>` that matches the spec's `StandardResponse` envelope. Include a `respond()` helper that constructs the envelope with vault name, operation, timing, and optional count/has_more.

```rust
use serde::Serialize;

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub vault: String,
    pub operation: String,
    pub success: bool,
    pub data: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    pub took_ms: u64,
}
```

- [ ] **Step 3: Create pagination.rs**

Define `PaginationParams` as an Axum query extractor with `limit` (default 50, max 200) and `offset` (default 0). Define `paginate<T>()` helper that slices a vec and returns `(items, total_count, has_more)`.

- [ ] **Step 4: Wire into lib.rs**

Add `pub mod errors; pub mod response; pub mod pagination;` to lib.rs.

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p turbovault-rest`
Expected: Compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/turbovault-rest/src/
git commit -m "feat(rest): add error types, response envelope, and pagination"
```

---

## Task 3: Auth Middleware + Content Negotiation + Vault Resolver

**Files:**
- Create: `crates/turbovault-rest/src/auth.rs`
- Create: `crates/turbovault-rest/src/content.rs`
- Create: `crates/turbovault-rest/src/vault_resolver.rs`

- [ ] **Step 1: Create auth.rs**

Axum middleware layer. If `RestConfig.api_token` is `Some`, check `Authorization: Bearer <token>` header on every request. If missing or wrong, return `ApiError::Unauthorized`. If `api_token` is `None`, pass through (LAN trust mode). Implement as a Tower layer so it can be selectively applied (health endpoint skips it).

- [ ] **Step 2: Create content.rs**

Define `NoteContent` struct with `content: String` field. Define `extract_note_content()` that checks `Content-Type` header:
- `application/json`: parse JSON body, extract `content` field
- `text/markdown` (or missing): use raw body as content string
Return `ApiError::InvalidRequest` if neither parses.

For PATCH, define `PatchRequest` with `target_type`, `target`, `operation`, `content` fields. Same content-type negotiation — JSON body or query params + raw body.

- [ ] **Step 3: Create vault_resolver.rs**

Define `resolve_vault()` function that takes the `AppState` and request headers. Check for `X-Vault` header — if present, look up that vault in `MultiVaultManager`. If not present, use the active vault. Return `(vault_name, Arc<VaultManager>)` or `ApiError::VaultNotFound`.

**Key implementation detail**: `MultiVaultManager` stores `VaultConfig` objects, not `VaultManager` instances. The existing MCP tools in `tools.rs` use `self.get_vault_manager()` which lazily creates/caches `VaultManager`. Study how `get_vault_pair()` in `crates/turbovault/src/tools.rs` resolves a `VaultManager` from `MultiVaultManager` and replicate that pattern. `FileTools::new()`, `SearchEngine::new()`, and `GraphTools` all require `Arc<VaultManager>`.

- [ ] **Step 4: Wire into lib.rs**

Add module declarations.

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p turbovault-rest`

- [ ] **Step 6: Commit**

```bash
git add crates/turbovault-rest/src/
git commit -m "feat(rest): add auth middleware, content negotiation, vault resolver"
```

---

## Task 4: Health Endpoint + Route Tree + main.rs Integration

This is the critical task — it validates the full stack from HTTP request through to response, including the Axum router merge with MCP.

**Files:**
- Create: `crates/turbovault-rest/src/v1/mod.rs`
- Create: `crates/turbovault-rest/src/v1/health.rs`
- Create: `crates/turbovault-rest/tests/helpers/mod.rs`
- Create: `crates/turbovault-rest/tests/test_health.rs`
- Modify: `crates/turbovault-rest/src/lib.rs`
- Modify: `crates/turbovault/Cargo.toml`
- Modify: `crates/turbovault/src/bin/main.rs`

- [ ] **Step 1: Write test_health.rs**

Test that `GET /v1/health` returns 200 with vault name and uptime. Test that it works without auth token (health is always open).

- [ ] **Step 2: Create v1/health.rs handler**

```rust
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    let vault_name = /* get active vault name */;
    let note_count = /* count files in vault */;
    Json(ApiResponse::new(vault_name, "health", json!({
        "status": "ok",
        "uptime_seconds": uptime,
        "note_count": note_count,
    })))
}
```

- [ ] **Step 3: Create v1/mod.rs route tree**

```rust
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/v1/health", get(health::health))
        // Other routes added in later tasks
}
```

- [ ] **Step 4: Wire routes into lib.rs router()**

Apply auth middleware to all routes except health. Return the composed router.

- [ ] **Step 5: Create test helpers**

`tests/helpers/mod.rs`: function to create a temp vault directory with a few test notes, build the `AppState`, and return an `axum_test::TestServer`.

- [ ] **Step 6: Run health test**

Run: `cargo test -p turbovault-rest test_health`
Expected: PASS

- [ ] **Step 7: Modify main.rs for router merge**

In `crates/turbovault/src/bin/main.rs`, for the `http` transport branch:
1. Add `turbovault-rest` to `crates/turbovault/Cargo.toml` dependencies.
2. Replace `server.run_http(&addr).await?` with the builder pattern.
   **IMPORTANT**: `builder()` consumes `server`, so extract `multi_vault()` first:
   ```rust
   let multi_vault = server.multi_vault(); // Arc clone before builder() consumes server
   let rest_config = RestConfig {
       api_token: std::env::var("VAULT_API_TOKEN").ok(),
       protected_paths: vec!["Focus Areas/Writing/".to_string()],
   };
   let rest_router = turbovault_rest::router(multi_vault, rest_config);
   let mcp_router = server.builder().into_axum_router(); // consumes server
   let app = mcp_router.merge(rest_router);
   let listener = TcpListener::bind(&addr).await?;
   axum::serve(listener, app).await?;
   ```
3. **Verify SSE**: Check if `into_axum_router()` includes the SSE route (`GET /sse`). If not, the SSE handler must be manually added to the merged router. Check `turbomcp-server/src/builder.rs` — if only `POST /` and `POST /mcp` are registered, SSE is lost. If SSE is not needed for current MCP clients (they use HTTP POST), document the omission.

- [ ] **Step 8: Verify the merged server compiles**

Run: `cargo build -p turbovault --features http`
Expected: Compiles.

- [ ] **Step 9: Commit**

```bash
git add crates/turbovault-rest/ crates/turbovault/
git commit -m "feat(rest): health endpoint + main.rs router merge"
```

---

## Task 5: Notes Read + Info Endpoints

**Files:**
- Create: `crates/turbovault-rest/src/v1/notes.rs`
- Create: `crates/turbovault-rest/src/v1/notes_info.rs`
- Create: `crates/turbovault-rest/tests/test_notes_crud.rs`
- Modify: `crates/turbovault-rest/src/v1/mod.rs`

- [ ] **Step 1: Write failing test — read existing note**

Create a temp vault with `test.md`. `GET /v1/notes/test.md` should return 200 with content and hash.

- [ ] **Step 2: Write failing test — read nonexistent note**

`GET /v1/notes/nonexistent.md` should return 404 with `NOT_FOUND`.

- [ ] **Step 3: Write failing test — notes-info**

`GET /v1/notes-info/test.md` should return 200 with size, modified time, `has_frontmatter`, but no content.

- [ ] **Step 4: Implement notes.rs GET handler**

Extract `{*path}` from URL. Call `resolve_vault()`. Call `FileTools::read_file()`. Compute SHA-256 hash. Return `ApiResponse` with content + hash. Set `ETag` response header to hash value.

- [ ] **Step 5: Implement notes_info.rs GET handler**

Extract `{*path}`. Get file metadata (size, mtime) via `std::fs::metadata`. Check for frontmatter by reading first few bytes. Return metadata without content.

- [ ] **Step 6: Add routes to v1/mod.rs**

```rust
.route("/v1/notes/{*path}", get(notes::read_note))
.route("/v1/notes-info/{*path}", get(notes_info::get_info))
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p turbovault-rest test_notes`
Expected: All PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): GET /v1/notes and /v1/notes-info endpoints"
```

---

## Task 6: Notes Write (PUT + POST)

**Files:**
- Modify: `crates/turbovault-rest/src/v1/notes.rs`
- Modify: `crates/turbovault-rest/tests/test_notes_crud.rs`

- [ ] **Step 1: Write failing test — PUT creates new note**

`PUT /v1/notes/Staging/new.md` with JSON body `{"content": "# New Note"}`. Should return 200. Then `GET` should return the content.

- [ ] **Step 2: Write failing test — PUT overwrites existing note**

Create `existing.md`, PUT with new content. GET should return new content.

- [ ] **Step 3: Write failing test — POST appends to existing note**

Create note with "Line 1". POST with "Line 2". GET should contain both.

- [ ] **Step 4: Write failing test — POST to nonexistent returns 404**

`POST /v1/notes/nope.md` should return 404.

- [ ] **Step 5: Write failing test — PUT with text/markdown content type**

`PUT /v1/notes/raw.md` with `Content-Type: text/markdown` and raw body. Should work identically.

- [ ] **Step 6: Implement PUT handler**

Extract path. Parse content via `content.rs` (JSON or markdown). Call `FileTools::write_file()` with overwrite mode. Return response with path and hash.

- [ ] **Step 7: Implement POST handler**

Same as PUT but call `FileTools::write_file()` with append mode. First check file exists — if not, return `ApiError::NotFound`.

- [ ] **Step 8: Add routes**

```rust
.route("/v1/notes/{*path}", get(notes::read_note).put(notes::create_note).post(notes::append_note))
```

- [ ] **Step 9: Run tests**

Run: `cargo test -p turbovault-rest test_notes_crud`
Expected: All PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): PUT and POST /v1/notes for create/overwrite/append"
```

---

## Task 7: Notes Patch (PATCH)

**Files:**
- Modify: `crates/turbovault-rest/src/v1/notes.rs`
- Create: `crates/turbovault-rest/tests/test_notes_patch.rs`

- [ ] **Step 1: Write failing test — patch append under heading**

Create note with `## Section A` heading. PATCH with `target_type=heading`, `target=Section A`, `operation=append`, `content=New text`. GET should show new text under Section A.

- [ ] **Step 2: Write failing test — patch with text/markdown content type**

Same operation but `Content-Type: text/markdown` with patch params in query string.

- [ ] **Step 3: Implement PATCH handler**

Extract path and `PatchRequest` (JSON body or query params + raw body). Call the vault's patch_note engine function (same one MCP's `patch_note` tool uses). Return response.

- [ ] **Step 4: Run tests**

Run: `cargo test -p turbovault-rest test_notes_patch`
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): PATCH /v1/notes for heading-aware insert"
```

---

## Task 8: Trash Engine (Manifest + Soft Delete + Restore)

This is the biggest task — it implements new functionality (trash manifest) not present in the existing MCP tools.

**Files:**
- Create: `crates/turbovault-rest/src/trash_manifest.rs`
- Create: `crates/turbovault-rest/src/v1/trash.rs`
- Create: `crates/turbovault-rest/tests/test_trash.rs`
- Modify: `crates/turbovault-rest/src/v1/notes.rs` (DELETE handler)
- Modify: `crates/turbovault-rest/src/v1/mod.rs`

- [ ] **Step 1: Write failing test — delete moves to trash**

Create `test.md`. DELETE it. Verify file is gone from vault. Verify file exists in `.trash/`. Verify response contains `moved_to` and `orphaned_links`.

- [ ] **Step 2: Write failing test — trash list shows deleted note**

After deleting, GET `/v1/trash` should list the trashed note with metadata.

- [ ] **Step 3: Write failing test — restore returns note**

After deleting, POST `/v1/restore/{trash-path}` should restore the file. GET on original path should work again.

- [ ] **Step 4: Write failing test — request-purge returns 202**

After deleting, POST `/v1/trash/{path}/request-purge` should return 202 with `status: pending`. File should still exist in trash.

- [ ] **Step 5: Implement trash_manifest.rs**

Define `TrashEntry` and `TrashManifest` structs matching the spec's JSON format. Implement:
- `TrashManifest::load(vault_path)` — read `.trash/.manifest.json`, create if missing
- `TrashManifest::add_entry(entry)` — add and save
- `TrashManifest::remove_entry(trash_path)` — remove and save
- `TrashManifest::mark_purge_requested(trash_path)` — set timestamp
- `TrashManifest::list()` — return all entries

- [ ] **Step 6: Implement DELETE handler in notes.rs**

1. Check file exists (404 if not)
2. Compute orphaned links via `GraphTools::get_backlinks()`
3. Move file to `.trash/{original-path}.{timestamp}`
4. Add manifest entry
5. Return response with `orphaned_links` and `moved_to`

- [ ] **Step 7: Implement trash.rs handlers**

- `list_trash`: Load manifest, paginate, return entries.
- `restore`: Find manifest entry, move file back, remove entry, return `restored_to`.
- `request_purge`: Find manifest entry, mark timestamp, log, return 202.

- [ ] **Step 8: Add routes**

```rust
.route("/v1/notes/{*path}", get(...).put(...).post(...).patch(...).delete(notes::delete_note))
.route("/v1/trash", get(trash::list_trash))
.route("/v1/restore/{*path}", post(trash::restore))
.route("/v1/request-purge/{*path}", post(trash::request_purge))
```

- [ ] **Step 9: Run tests**

Run: `cargo test -p turbovault-rest test_trash`
Expected: All PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): soft delete, trash manifest, restore, and request-purge"
```

---

## Task 9: Search Endpoint

**Files:**
- Create: `crates/turbovault-rest/src/v1/search.rs`
- Create: `crates/turbovault-rest/tests/test_search.rs`
- Modify: `crates/turbovault-rest/src/v1/mod.rs`

- [ ] **Step 1: Write failing test — search finds note by content**

Create note with "quantum computing". GET `/v1/search?q=quantum`. Should return the note in results with path, title, snippet, score.

- [ ] **Step 2: Write failing test — search with pagination**

Create 5 notes. Search with `limit=2&offset=0`. Should return 2 results with `has_more: true`.

- [ ] **Step 3: Implement search.rs handler**

Extract `q`, `limit`, `offset` from query params. Call `SearchEngine::search()`. Map results to response format. Apply pagination.

- [ ] **Step 4: Run tests, commit**

Run: `cargo test -p turbovault-rest test_search`

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): GET /v1/search with BM25 full-text search"
```

---

## Task 10: Navigation Endpoints (Files, Periodic, Recent)

**Files:**
- Create: `crates/turbovault-rest/src/v1/files.rs`
- Create: `crates/turbovault-rest/src/v1/periodic.rs`
- Create: `crates/turbovault-rest/src/v1/recent.rs`
- Modify: `crates/turbovault-rest/src/v1/mod.rs`

- [ ] **Step 1: Write failing tests for files listing**

Create files in nested dirs. `GET /v1/files` lists root. `GET /v1/files/subdir` lists subdir contents. Returns file names, sizes, types (file/directory).

- [ ] **Step 2: Implement files.rs**

Use the same directory listing logic as the MCP `list_files` tool. Paginate results.

- [ ] **Step 3: Write failing test for periodic note**

Create `Daily/2026-03-19.md`. `GET /v1/periodic/daily` should return it. `GET /v1/periodic/daily?date=2026-03-19` should return it.

- [ ] **Step 4: Implement periodic.rs**

Same logic as MCP `get_periodic_note`. Parse period type, optional date param.

- [ ] **Step 5: Write failing test for recent changes**

Create 3 notes at different times. `GET /v1/recent?days=7&limit=2` should return 2 most recent with timestamps.

- [ ] **Step 6: Implement recent.rs**

Same logic as MCP `get_recent_changes`. Sort by mtime descending. Paginate.

- [ ] **Step 7: Add all routes, run tests, commit**

Run: `cargo test -p turbovault-rest test_files test_periodic test_recent`

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): files listing, periodic notes, and recent changes endpoints"
```

---

## Task 11: Links Endpoints (Backlinks + Forward)

**Files:**
- Create: `crates/turbovault-rest/src/v1/links.rs`
- Create: `crates/turbovault-rest/tests/test_links.rs`
- Modify: `crates/turbovault-rest/src/v1/mod.rs`

- [ ] **Step 1: Write failing test — backlinks**

Create `A.md` with `[[B]]`. `GET /v1/links/B.md/backlinks` should return `A.md`.

- [ ] **Step 2: Write failing test — forward links**

`GET /v1/links/A.md/forward` should return `B.md`.

- [ ] **Step 3: Implement links.rs**

**Routing decision**: Axum's `{*path}` captures the rest of the URL, so `/v1/links/{*path}/backlinks` is not possible as a single route. Use flattened routes instead:

```rust
.route("/v1/backlinks/{*path}", get(links::backlinks))
.route("/v1/forward-links/{*path}", get(links::forward_links))
```

This deviates from the spec's `/v1/links/{*path}/backlinks` pattern. The spec should be updated to match. The functionality is identical — only the URL shape changes.

Call `GraphTools` for backlink/forward link resolution.

- [ ] **Step 4: Run tests, commit**

Run: `cargo test -p turbovault-rest test_links`

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): backlinks and forward links endpoints"
```

---

## Task 12: Batch Read

**Files:**
- Create: `crates/turbovault-rest/src/v1/batch.rs`
- Create: `crates/turbovault-rest/tests/test_batch.rs`
- Modify: `crates/turbovault-rest/src/v1/mod.rs`

- [ ] **Step 1: Write failing test — batch read**

Create 3 notes. POST `/v1/batch/read` with `{"paths": ["a.md", "b.md", "c.md"]}`. Should return all 3 with content and hashes.

- [ ] **Step 2: Write failing test — batch with nonexistent path**

Include a nonexistent path in the batch. That entry should have `error` field instead of `content`. Other entries should still succeed (partial success).

- [ ] **Step 3: Write failing test — batch over 50 limit**

POST with 51 paths. Should return 400 `INVALID_REQUEST`.

- [ ] **Step 4: Implement batch.rs**

Parse JSON body. Validate count <= 50. Read each path via `FileTools::read_file()`. Collect results — successes and failures side by side.

- [ ] **Step 5: Run tests, commit**

Run: `cargo test -p turbovault-rest test_batch`

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): POST /v1/batch/read for multi-note retrieval"
```

---

## Task 13: Cross-Cutting Concerns (Auth, If-Match, Protected Paths)

**Files:**
- Create: `crates/turbovault-rest/tests/test_auth.rs`
- Create: `crates/turbovault-rest/tests/test_errors.rs`
- Modify: handlers as needed

- [ ] **Step 1: Write failing test — auth required when token set**

Build app with `api_token: Some("secret")`. Request without token → 401. Request with wrong token → 401. Request with correct token → 200.

- [ ] **Step 2: Write failing test — health bypasses auth**

With token set, `GET /v1/health` without token → 200 (not 401).

- [ ] **Step 3: Write failing test — If-Match prevents stale write**

Read note (get hash). Modify note directly on filesystem. PUT with old hash in `If-Match` → 409 `HASH_MISMATCH`. PUT with correct hash → 200.

- [ ] **Step 4: Write failing test — protected path**

PUT to `Focus Areas/Writing/story.md` → 403 `FORBIDDEN`. GET same path → 200 (reads allowed).

- [ ] **Step 5: Implement If-Match checking**

In PUT/POST/PATCH/DELETE handlers, check for `If-Match` header. If present, read current file hash and compare. Return `ApiError::HashMismatch` if different.

- [ ] **Step 6: Implement protected path checking**

In write handlers (PUT/POST/PATCH/DELETE), check if path starts with any `RestConfig.protected_paths` entry. Return `ApiError::Forbidden` if matched.

- [ ] **Step 7: Run all tests**

Run: `cargo test -p turbovault-rest`
Expected: All PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/turbovault-rest/
git commit -m "feat(rest): auth middleware, If-Match concurrency, protected paths"
```

---

## Task 14: Integration Test — Full CRUD + Trash Lifecycle

**Files:**
- Modify: `crates/turbovault-rest/tests/test_notes_crud.rs`

- [ ] **Step 1: Write end-to-end lifecycle test**

Single test that exercises the full lifecycle:
1. `PUT /v1/notes/Staging/lifecycle-test.md` — create
2. `GET /v1/notes/Staging/lifecycle-test.md` — read, save hash
3. `POST /v1/notes/Staging/lifecycle-test.md` — append
4. `GET /v1/notes/Staging/lifecycle-test.md` — verify appended content
5. `PATCH /v1/notes/Staging/lifecycle-test.md` — patch under heading
6. `GET /v1/notes-info/Staging/lifecycle-test.md` — metadata check
7. `DELETE /v1/notes/Staging/lifecycle-test.md` — soft delete
8. `GET /v1/notes/Staging/lifecycle-test.md` — 404
9. `GET /v1/trash` — verify in trash
10. `POST /v1/restore/...` — restore
11. `GET /v1/notes/Staging/lifecycle-test.md` — 200 again

- [ ] **Step 2: Run test**

Run: `cargo test -p turbovault-rest lifecycle`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/turbovault-rest/
git commit -m "test(rest): end-to-end CRUD + trash lifecycle integration test"
```

---

## Task 15: Build, Deploy, and Smoke Test

**Files:**
- Modify: Docker build (if needed)

- [ ] **Step 1: Build release binary**

Run: `cd ~/projects/turbovault && cargo build --release --features http`
Expected: Compiles successfully.

- [ ] **Step 2: Build Docker image**

Build `local/turbovault:1.3.0` with the REST API included.

- [ ] **Step 3: Deploy on MaxDesk**

Update the TurboVault container on MaxDesk to `1.3.0`. Verify MCP still works (test from Oracle with `mcp__vault__read_note`). Verify REST works:

```bash
curl -s http://vault.home.iramsay.com/v1/health | jq .
curl -s http://vault.home.iramsay.com/v1/notes/Daily/2026-03-19.md | jq .data.path
curl -s -X PUT -H "Content-Type: text/markdown" \
  -d '# REST API Test' \
  http://vault.home.iramsay.com/v1/notes/Staging/rest-api-test.md | jq .
curl -s -X DELETE http://vault.home.iramsay.com/v1/notes/Staging/rest-api-test.md | jq .
```

- [ ] **Step 4: Commit version bump**

```bash
git commit -m "chore: bump version to 1.3.0 with REST API"
```

- [ ] **Step 5: Update vault documentation**

Update the Centralized Vault MCP Server project note and MCP Server Configuration with the new REST API availability.

---

## Task Dependency Summary

```
Task 1 (scaffolding)
  → Task 2 (errors + response)
    → Task 3 (auth + content + vault resolver)
      → Task 4 (health + main.rs merge)  ← validates full stack
        → Task 5 (notes read)
          → Task 6 (notes write)
            → Task 7 (notes patch)
            → Task 8 (trash lifecycle)  ← biggest task
          → Task 9 (search)
          → Task 10 (files/periodic/recent)
          → Task 11 (links)
          → Task 12 (batch)
        → Task 13 (cross-cutting: auth, if-match, protected paths)
      → Task 14 (integration test)
    → Task 15 (build + deploy + smoke test)
```

Tasks 5-12 can be parallelized after Task 4 completes. Task 13 depends on write handlers existing (Tasks 6-8). Task 14 depends on all endpoints being implemented.

---

## Known Limitations & Future Work

1. **Trash manifest is REST-only**: The existing MCP `delete_note` tool does a hard filesystem delete, not soft delete with trash manifest. The trash engine (`trash_manifest.rs`) lives in the `turbovault-rest` crate for now. If MCP delete behavior should be unified with REST (soft delete), the trash manifest should be extracted to `turbovault-core` or `turbovault-vault`. This is a cross-interface consistency concern flagged for future work.

2. **Route deviations from spec**: Links endpoints use `/v1/backlinks/{*path}` and `/v1/forward-links/{*path}` instead of the spec's `/v1/links/{*path}/backlinks` due to Axum catch-all routing constraints. Similarly, request-purge uses `/v1/request-purge/{*path}` instead of `/v1/trash/{*path}/request-purge`. The spec should be updated to match.

3. **SSE endpoint**: If `into_axum_router()` does not include the SSE route, MCP SSE clients will break. Verify during Task 4 and add SSE manually if needed.

4. **Edge case tests to add during implementation**: path traversal attacks (`../../`), `.obsidian/` path rejection, write to trashed note (409 CONFLICT), empty batch read array, `X-Vault` with nonexistent vault.

5. **`ETag` on write responses**: Write responses should include `ETag` header with the new content hash so callers can use it for subsequent `If-Match` operations.
