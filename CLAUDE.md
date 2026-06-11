@RTK.md

# Backend Architecture (src-tauri/src)

The Rust/Tauri backend is organised into **vertical domain slices**. Each slice owns its own layer files rather than grouping by type across the whole codebase.

## Slice layout

| Slice | Path | Responsibility |
|---|---|---|
| `library` | `src/library/` | Folder management, image indexing, file-watching, DB image CRUD |
| `people` | `src/people/` | Faces, subjects, clustering, merge suggestions, constraints |
| `tags` | `src/tags/` | Tag CRUD, subject–tag junctions, tag-aware search helpers |
| `search` | `src/search/` | Vector index, embeddings repo, text normalisation, semantic search |
| `media` | `src/media/` | Preview/thumbnail generation, face-crop helpers |
| `settings` | `src/settings/` | Key/value settings repo |
| `pipeline` | `src/pipeline/` | Processing queue, pipeline orchestration |
| `vision` | `src/vision/` | ONNX model loading, image/text embedding engine |
| `db` | `src/db/` | SQLite pool init, schema migrations, sqlite-vec registration only |
| `app` | `src/app/` | Tauri builder, AppState, command handler registry |

## Per-slice layers

Each slice follows this file pattern (not all files are required in every slice):

- `commands.rs` — `#[tauri::command]` IPC handlers
- `repo.rs` — SQL queries (sqlx)
- `models.rs` — slice-local structs/enums
- `service.rs` — business logic that coordinates repo calls

## Rules

- **Domain queries live in the slice `repo.rs`**, not in `db/`.
- `db/mod.rs` contains only `init_db`, `ensure_sqlite_vec_registered`, `BASE_SCHEMA`, and `VERSIONED_MIGRATIONS`.
- Cross-slice access goes via the public API of the target slice (`crate::library::repo::get_image_by_id`, etc.) — never reach into another slice's internals.
- `#[tauri::command]` handlers must be referenced at their definition site in `app/mod.rs`; `pub use` re-exports do **not** propagate `__cmd__*` symbols.

# Tooling discipline
- NEVER issue filler/no-op commands (e.g. `echo p1`, `echo probe`, repeated `sleep`) to "flush" or poll for delayed tool output. If a tool result comes back empty or the harness seems laggy, wait for the real result or re-issue the single substantive command once — do not spam. Wasting tokens on probe commands is not acceptable.

## Error-handling discipline
- AFTER any command that creates a resource (Notion page, git branch, API call),
  immediately verify success by inspecting the actual output/ID before using it
  in the next command.
- NEVER fabricate IDs or assume a previous step succeeded because the next step
  didn't error — check the create-step output explicitly.
- When steps have dependencies, run them SEQUENTIALLY, not in parallel.
  A cascade-cancelled batch wastes more tokens than waiting.
- If a foundational step fails (e.g., `ntn api` returns an error), STOP. Do not
  proceed with downstream steps until the root failure is understood and fixed.
