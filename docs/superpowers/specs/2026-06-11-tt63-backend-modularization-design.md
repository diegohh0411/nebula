# TT-63 — Restructure Rust/Tauri backend into domain vertical-slice modules

**Notion task:** TT-63 (`37ce954d-b476-81f6-8102-dd955ad71fd4`)
**Date:** 2026-06-11
**Status:** Approved design

## Problem

The backend (`src-tauri/src/`, 26 files, ~9k LOC) grew into a flat namespace. Domain
logic, IPC handlers, and persistence are smeared across single-purpose files with no
grouping by domain. The epicenter is **`db.rs` — 2,644 lines, ~70 functions** — mixing
*every* persistence concern in one file: folders, images, the processing queue,
embeddings, subjects, faces, tags, settings, merge suggestions, face-graph edges, and the
embedding cache. IPC commands live in `commands.rs` **except** settings commands, which
live in `settings.rs`. A capable AI agent can navigate this; a human cannot see the
domains, and new code has nowhere obvious to go, so the soup deepens.

Two domains are already well-structured and are the model to follow: `models/` (ML model
registry) and `pipeline/` (the processing engine).

## Goal

Reorganize into **vertical slices by domain** — the "group by feature, not by type"
philosophy the Angular frontend already follows. **No behavior change.** This is a pure
move/regroup refactor, executed incrementally so each step compiles and ships on its own.

## Architecture decision (settled during brainstorming)

**Vertical slices by domain**, not horizontal layers by tech concern. Rationale: it mirrors
Angular feature modules (a feature folder owns its components + service + models), which is
the mental model the team already uses. Horizontal layering (all commands in one folder,
all services in another) is the anti-pattern Angular's own style guide steers away from.

### Per-slice internal convention (the rule that prevents re-rot)

Every slice follows the same internal layering, so the codebase is self-similar — learn one
folder, you know them all. Angular analogy in parentheses:

| File | Responsibility | Hard rule |
|---|---|---|
| `commands.rs` | Tauri IPC boundary (component event handler) | **No business logic.** Deserialize args, call service, map errors. |
| `service.rs` | Business logic / orchestration (`@Injectable()` service) | May call its own `repo` and other slices' public APIs. |
| `repo.rs` | Persistence (HttpClient data layer) | **The only place SQL lives.** |
| `models.rs` | Types / DTOs (TS `interface`) | Serde-serializable structs returned to the frontend. |

Small slices (`tags`, `settings`) may omit `service.rs` until they need one (YAGNI). A
command may call `repo` directly when there is genuinely no logic.

## Target module tree

```
src-tauri/src/
  main.rs                 (unchanged)
  lib.rs                  (shrinks to: `mod` declarations + `pub use app::run`)

  app/
    mod.rs                Tauri Builder, setup(), invoke_handler registry
    state.rs              AppState struct

  library/                photo catalog
    commands.rs           add_folder, remove_folder, list_folders, list_images
    service.rs            ingestion orchestration (if needed)
    repo.rs               folders + images queries (from db.rs)
    indexer.rs            filesystem walk + ingest (from indexer.rs)
    watcher.rs            fs change watching (from watcher.rs)
    models.rs             Folder, FolderWithCount, Image, DbImage

  media/                  decode / resize / cached artifacts
    commands.rs           prioritize_previews, get_face_crop, set_subject_thumbnail
    preview.rs            PreviewService, queue, governor (from preview.rs)
    thumbnail.rs          thumbnail + face-crop gen + path helpers (from thumbnail.rs)

  search/                 semantic + deterministic search
    commands.rs           search
    service.rs            search_images, build_search_results (from search.rs)
    repo.rs               embeddings + embedding-cache queries (from db.rs)
    vector_index.rs       VectorIndex trait, FlatIndex, snapshot (from vector_index.rs)
    text.rs               normalize, like_pattern, matches_tokens (from db.rs)
    math.rs               f32<->bytes, cosine_similarity (from embedder.rs)
    models.rs             SearchResult

  people/                 faces, subjects, clustering, merges
    commands.rs           list_subjects, name_subject, list_faces, merge_subjects, ...
    service.rs            assignment / merge orchestration
    repo.rs               subjects + faces + face-edge queries (from db.rs)
    face_store.rs         sqlite-vec KNN over face vectors (from face_store.rs)
    face_quality.rs       frontality / sharpness / composite (from face_quality.rs)
    clustering.rs         union-find, must/cannot-link, merge suggestions (from clustering.rs)
    models.rs             Subject, Face, MergeSuggestion, SubjectDetail

  tags/                   subject tagging (first-class slice)
    commands.rs           create_tag, add_subject_tag, list_tags, rename_tag, ...
    repo.rs               tags + subject_tags queries (from db.rs)
    models.rs             Tag, TagWithCount, SubjectMatch

  models/                 ML model registry/download/mgmt  (ALREADY GOOD — unchanged)
  pipeline/               processing engine  (ALREADY GOOD — repoint imports only)

  vision/
    engine.rs             VisionEngine (from vision_engine.rs)
    preprocess.rs         tensor pixel prep (from preprocess.rs)

  settings/
    commands.rs           get_setting, update_setting, get_available_*models (from settings.rs)
    repo.rs               settings get/update (from db.rs)

  db/
    mod.rs                pool, init_db, migrations, ensure_sqlite_vec_registered (from db.rs)

  platform/
    logger.rs             (from logger.rs)
    paths.rs              data-dir / cache-dir helpers
```

### `pipeline/` stays a standalone orchestrator

`pipeline/run_pipeline` touches `db` (library + people + search), `vision_engine`, `models`,
`face_store`, `clustering`, `thumbnail`, and `embedder`. It is a facade/orchestrator, not a
domain. It stays as-is structurally; only its imports get repointed to the new slice APIs.
This makes it the **litmus test** for the refactor: if `run_pipeline` reads cleanly as a
consumer of `crate::library::repo`, `crate::people::clustering`, `crate::search::math`,
etc., the boundaries are right.

## `db.rs` dissolution map (the crux — explicit so a less-capable agent cannot guess wrong)

Each current `db.rs` function moves to exactly one destination. Functions become
`pub` members of the destination module. Update all call sites accordingly.

**→ `db/mod.rs` (shared foundation):**
`ensure_sqlite_vec_registered`, `init_db`.

**→ `search/text.rs` (deterministic-match helpers):**
`normalize`, `like_pattern`, `matches_tokens`.

**→ `library/repo.rs` (folders + images):**
`row_to_image`, `insert_folder`, `delete_folder`, `list_folders_with_counts`,
`list_all_folders`, `insert_image`, `update_image_hash_changed`, `update_image_metadata`,
`clear_image_deleted`, `DbImage`, `get_all_images_for_rescan`, `get_image_metadata_by_path`,
`soft_delete_image_by_id`, `soft_delete_image`, `update_thumbnail_path`,
`update_preview_path`, `images_needing_preview`, `list_images`, `get_image_by_id`.

**→ `pipeline/` queue repo (queue lifecycle owned by the processing engine):**
`enqueue_image`, `get_queue_batch`, `mark_semantic_analysis_done`,
`mark_subject_analysis_done`, `mark_failed`, `get_processing_counts`.
*(Place in `pipeline/queue.rs` or `library/repo.rs` — queue rows reference images. Decision:
`pipeline/queue.rs`, since the pipeline owns the queue's read/claim/complete lifecycle.)*

**→ `search/repo.rs` (embeddings + cache):**
`get_image_embedding`, `get_all_embeddings`, `get_cached_embedding`,
`insert_cached_embedding`, `delete_stale_cache_entries`.

**→ `people/repo.rs` (subjects, faces, face-graph edges):**
`insert_subject`, `insert_face`, `list_all_subjects`, `list_faces_for_subject`,
`get_face_by_id`, `update_subject_name`, `update_subject_thumbnail_face`,
`get_subject_detail_with_counts`, `list_images_for_subject`, `get_largest_face_for_subject`,
`list_faces_for_image`, `get_face_with_image`, `get_unassigned_faces_with_embeddings`,
`update_face_subject`, `delete_subjects_with_no_faces`, `auto_assign_missing_thumbnails`,
`upgrade_subject_thumbnails`, `clear_merge_suggestions`, `insert_merge_suggestion`,
`get_merge_suggestions`, `merge_subjects`, `get_dismissed_pair_set`,
`dismiss_merge_suggestion`, `find_subject_by_name`, `assign_face_to_subject`,
`create_subject_for_face`, `unassign_face`, `ordered_pair`, `add_must_link`,
`add_cannot_link`, `upsert_face_edge`, `clear_all_face_edges`, `get_all_similarity_edges`,
`get_all_must_link_pairs`, `get_all_cannot_link_pairs`, `get_assigned_face_subject_map`,
`get_face_ids_for_subject`, `get_all_face_ids_with_vectors`.

**→ `tags/repo.rs` (tags + subject_tags + tag-driven search):**
`create_tag`, `add_subject_tag`, `remove_subject_tag`, `get_subject_tags`,
`list_tags_with_counts`, `rename_tag`, `delete_tag`, `get_tag_image_ids_ordered`,
`search_subjects_matching`, `get_subjects_for_tag`, `get_image_ids_for_subjects`.

**→ `settings/repo.rs`:**
`get_setting` (and `update_setting` if present in settings.rs's path).

**→ Maintenance/reset (place in the slice they reset; cross-cutting resets in `db/mod.rs`):**
`reset_all_embeddings` → `search/repo.rs`; `reset_all_subject_data` → `people/repo.rs`.

**SQLite schema (`CREATE TABLE` block in `init_db`):** stays in `db/mod.rs` as-is. The base
schema is shared. *(Per project convention, schema is the base `CREATE TABLE` set; no
migrations framework — do not split the schema across slices.)*

## Cross-cutting: the IPC registry

Today `lib.rs` holds the `invoke_handler![...]` macro listing `commands::*` and
`settings::*`. After the refactor it lives in `app/mod.rs` and references each slice's
`commands` module: `library::commands::add_folder`, `people::commands::list_subjects`,
`tags::commands::create_tag`, `settings::commands::get_setting`, etc. This is where the
current inconsistency (settings commands living apart) gets fixed: **all** commands are
registered in one list, each pointing at its owning slice.

`AppState` moves to `app/state.rs` unchanged. Re-export at crate root
(`pub use app::state::AppState`) so existing `crate::AppState` references keep working.

## Migration plan (incremental — see implementation plan for step detail)

One reviewable PR per step. Each step ends green on `cargo build` + `cargo test`. No logic
changes are mixed into a move.

1. **Foundation** — scaffold slice folders (empty `mod.rs`s wired into `lib.rs`); split
   `db.rs` → `db/mod.rs` keeping domain queries as a temporary `db/legacy.rs` re-exported as
   `crate::db::*` so nothing breaks yet; extract `app/` (Builder + `AppState`) and
   `platform/` (logger, paths).
2. **library/** — move folder/image queries to `library/repo.rs`; move `indexer`, `watcher`;
   move folder/image commands to `library/commands.rs`; repoint registry.
3. **people/** — subjects/faces/edge queries → `people/repo.rs`; move `clustering`,
   `face_store`, `face_quality`; people commands → `people/commands.rs`.
4. **tags/** — tag queries + commands.
5. **search/** — embedding/cache queries → `search/repo.rs`; move `vector_index`, `search`
   (→ `service.rs`), `embedder` (→ `math.rs`), text helpers (→ `text.rs`).
6. **media/ + vision/** — move `preview`, `thumbnail`; `vision_engine` → `vision/engine.rs`,
   `preprocess` → `vision/preprocess.rs`.
7. **settings/** — settings queries + existing commands.
8. **pipeline/ repoint + cleanup** — update `pipeline/` imports to the new slice APIs;
   delete the temporary `db/legacy.rs`; `db.rs` is gone.
9. **CLAUDE.md** — add the Architecture section (slice map, layering convention, agent
   navigation rules).

## Error handling

No change to runtime error behavior. `anyhow::Result` usage, error strings returned to the
frontend, and logging all move verbatim with their functions. A move that changes an error
message or `?` path is out of scope.

## Testing

This is a behavior-preserving refactor, so the safety net is **the existing build + tests at
every step**, not new assertions:

- `cd src-tauri && cargo build` must pass after each step.
- `cd src-tauri && cargo test` must pass after each step (the `db.rs` `#[cfg(test)] mod
  tests` block moves with its functions; split it across the destination slices' test
  modules).
- The Angular frontend is untouched; no frontend test run needed beyond CI.
- Manual smoke after step 8 and step 9: launch the app, import a folder, run a search,
  confirm People/Tags views populate — to catch any IPC-registration regression.

## CLAUDE.md additions (step 9 deliverable)

Add an **Architecture** section containing:

- **Slice map** — one line per domain folder ("what lives here").
- **The layering convention** — the `commands`/`service`/`repo`/`models` table above, plus
  the two hard rules: *no logic in `commands.rs`*, *no SQL outside `repo.rs`*.
- **Agent navigation rules**, e.g.:
  - *Add a Tauri command:* implement in the slice's `commands.rs`, register in `app/mod.rs`.
    Never add commands to `lib.rs`.
  - *Add a query:* put it in the owning slice's `repo.rs`. Never create a shared
    catch-all DB module; `db/` is foundation only (pool/init/schema).
  - *`pipeline/` is an orchestrator* — it consumes domain APIs; it does not own domain logic.
  - *New domain?* create a new slice folder following the convention; don't bolt onto an
    existing slice.
- **"Which slice does my change belong to?"** quick decision guide mapping common tasks to
  folders.

## Out of scope

- Any behavior change, performance tuning, or bug fix (separate tasks).
- Splitting the SQLite schema or introducing a migrations framework.
- Converting the single crate into a Cargo workspace of multiple crates (possible future
  step; not needed now).
- Touching the Angular frontend.
