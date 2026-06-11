# Backend Domain Vertical-Slice Modularization — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize `src-tauri/src/` from a flat namespace (with a 2,644-line `db.rs` god-module) into vertical slices by domain, with zero behavior change.

**Architecture:** Each domain becomes a folder (`library/`, `media/`, `search/`, `people/`, `tags/`, `vision/`, `settings/`) with a consistent internal layering — `commands.rs` (IPC) / `service.rs` (logic) / `repo.rs` (SQL) / `models.rs` (types). `db.rs` dissolves; each query moves to its owning slice's `repo.rs`, leaving only pool/init/schema in `db/mod.rs`. `models/` and `pipeline/` are already well-structured and stay put.

**Tech Stack:** Rust 2021, Tauri 2, sqlx (SQLite), tokio. Build/test from `src-tauri/`.

**Companion spec:** `docs/superpowers/specs/2026-06-11-tt63-backend-modularization-design.md` — contains the full `db.rs` function→slice mapping. This plan tells you *how to move safely*; the spec tells you *what goes where*.

---

## READ THIS FIRST — the technique that makes this safe

You are moving code, not changing it. Four rules keep you out of trouble:

### Rule 1 — The compiler is your safety net
After every move, run `cd src-tauri && cargo build`. The Rust compiler reports **every** broken path with its exact location (`error[E0433]: failed to resolve... use of undeclared crate or module`). Your job is mechanical: fix each reported path, rebuild, repeat until green. You do not need to find references by hand — the compiler finds them all.

### Rule 2 — The re-export bridge keeps each step compiling on its own
When you move a function out of `db.rs`, you do **not** immediately hunt down its callers. Instead you leave a one-line re-export so the old path `crate::db::foo` still resolves:

```rust
// in the OLD location (db.rs / db/legacy.rs), after moving `insert_folder` to library::repo:
pub use crate::library::repo::insert_folder;
```

Now `crate::db::insert_folder(...)` at every call site still works, and the build stays green. Callers get repointed to `crate::library::repo::insert_folder` in that slice's own task (or, for laggards, in the final cleanup task). This is why the steps are independently shippable.

### Rule 3 — NEVER edit a function body during a move
A move changes only: the file a function lives in, its `mod`/`use` declarations, and `crate::`-qualified paths. If making the build pass seems to require changing logic, an `if`, an error string, or a `?` — **STOP**, you have misunderstood the move. Revert and re-read. The only edits allowed are import paths and module wiring.

### Rule 4 — Prove behavior is unchanged before committing
Before each commit run BOTH:
```bash
cd src-tauri && cargo build && cargo test
```
Both must pass. Then eyeball `git diff --stat` — a move task should show lines leaving one file and arriving in another in roughly equal measure, plus small import edits. A large net change in line count means you edited a body. Investigate before committing.

### The repeating rhythm for every task below
1. Create the destination module file(s) and wire `mod` declarations.
2. Move the items (cut from source, paste into destination, make them `pub`).
3. Add re-export bridges at the old location for anything still referenced as the old path.
4. `cargo build` → fix every path the compiler complains about → repeat until green.
5. `cargo test` → green.
6. `git diff --stat` sanity check.
7. Commit.

---

## File Structure (target)

See the spec's "Target module tree" for the full layout. The slices created by this plan:
`app/`, `library/`, `media/`, `search/`, `people/`, `tags/`, `vision/`, `settings/`, `db/`, `platform/`. Unchanged: `models/`, `pipeline/`, `main.rs`.

---

## Task 1: Foundation — scaffold slices, carve `db.rs`, extract `app/` and `platform/`

**Files:**
- Create: `src-tauri/src/db/mod.rs`, `src-tauri/src/db/legacy.rs`
- Create: `src-tauri/src/app/mod.rs`, `src-tauri/src/app/state.rs`
- Create: `src-tauri/src/platform/mod.rs`, `src-tauri/src/platform/paths.rs`
- Create empty wiring stubs: `src-tauri/src/library/mod.rs`, `media/mod.rs`, `search/mod.rs`, `people/mod.rs`, `tags/mod.rs`, `vision/mod.rs`, `settings/mod.rs`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/logger.rs` (moves)

- [ ] **Step 1: Confirm a clean green baseline**

Run:
```bash
cd src-tauri && cargo build && cargo test
```
Expected: PASS. If it does not pass on `main` before you start, STOP and report — you cannot distinguish your breakage from a pre-existing one otherwise.

- [ ] **Step 2: Split `db.rs` into `db/mod.rs` + `db/legacy.rs` (mechanical, no logic change)**

`git mv` the file into the new folder as `legacy.rs`, then create a `mod.rs` that re-exports everything so `crate::db::*` is unchanged for now:

```bash
cd src-tauri/src
mkdir db
git mv db.rs db/legacy.rs
```

Create `src-tauri/src/db/mod.rs`:
```rust
//! Persistence foundation: pool, init, schema, sqlite-vec registration.
//! Domain queries are being migrated OUT of `legacy` into per-slice `repo.rs`
//! modules (TT-63). `legacy` shrinks to empty and is deleted in the final task.
mod legacy;
pub use legacy::*;
```

`cargo build && cargo test` → expect PASS (nothing moved yet, only the file location changed). Commit:
```bash
git add -A && git commit -m "refactor(TT-63): move db.rs into db/ module (no behavior change)"
```

- [ ] **Step 3: Move the foundation items into `db/mod.rs`**

Cut these items from `db/legacy.rs` and paste them into `db/mod.rs` (keep them `pub`): `ensure_sqlite_vec_registered`, `init_db`, and the `CREATE TABLE` schema string they use. Remove their `pub use legacy::*` shadow only if duplicate-definition errors appear; the glob re-export will not conflict because the names now live in `mod.rs` directly. Keep all `use` imports these functions need (copy the relevant `use` lines to the top of `mod.rs`).

`cargo build` → fix any import paths the compiler flags → `cargo test` → PASS. Commit:
```bash
git add -A && git commit -m "refactor(TT-63): move pool/init/schema into db/mod.rs foundation"
```

- [ ] **Step 4: Scaffold the empty slice modules**

Create each of these as a one-line doc stub so later tasks have a home to land in. Example `src-tauri/src/library/mod.rs`:
```rust
//! Library slice: the photo catalog (folders, images, indexing, watching).
```
Repeat for `media/mod.rs`, `search/mod.rs`, `people/mod.rs`, `tags/mod.rs`, `vision/mod.rs`, `settings/mod.rs` with an appropriate one-line doc comment each.

- [ ] **Step 5: Create `platform/` and move `logger`**

```bash
cd src-tauri/src
mkdir platform
git mv logger.rs platform/logger.rs
```
Create `src-tauri/src/platform/mod.rs`:
```rust
//! Cross-cutting infrastructure: logging, filesystem paths.
pub mod logger;
pub mod paths;
```
Create `src-tauri/src/platform/paths.rs` and move the cache-dir/path helpers out of `thumbnail.rs` into it: `thumbnail_cache_dir`, `face_crop_cache_dir` (the directory helpers). Leave `thumbnail_path_for`, `preview_path_for`, `face_crop_path_for` in `thumbnail.rs` for now (they move with `media/` in Task 6). Add a re-export in `thumbnail.rs` so existing call sites keep working:
```rust
pub use crate::platform::paths::{thumbnail_cache_dir, face_crop_cache_dir};
```

- [ ] **Step 6: Register the new modules and repoint `logger`**

In `src-tauri/src/lib.rs`, replace `mod logger;` with `mod platform;` and add the slice modules. The module declaration block becomes:
```rust
mod app;
mod db;
mod platform;
mod library;
mod media;
mod search;
mod people;
mod tags;
mod vision;
mod settings;
pub mod models;
pub mod pipeline;
// legacy flat modules still present, removed as their slices absorb them:
mod clustering;
mod face_quality;
mod face_store;
mod commands;
mod embedder;
mod preprocess;
mod preview;
mod indexer;
mod vector_index;
mod watcher;
mod thumbnail;
pub mod vision_engine;
```
`cargo build` will now flag `logger::init` (called in `lib.rs` setup). Repoint it to `platform::logger::init`. Fix every path the compiler reports.

- [ ] **Step 7: Extract `app/` (Builder + AppState)**

Create `src-tauri/src/app/state.rs` and move the `AppState` struct there (cut from `lib.rs`). Add the needed `use` lines. Create `src-tauri/src/app/mod.rs` and move the entire `run()` function body there (the `tauri::Builder`...`.run()` chain plus the `setup` closure). `app/mod.rs` starts:
```rust
//! Tauri application wiring: Builder, setup, command registry.
pub mod state;
pub use state::AppState;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

pub fn run() {
    // ... entire body moved verbatim from lib.rs run() ...
}
```
In `lib.rs`, keep the crate-root re-export so external references (`nebula_lib::run`, `crate::AppState`) keep resolving:
```rust
pub use app::{run, AppState};
```
`lib.rs` now contains only the `mod` block and that one `pub use`. Inside `app/mod.rs`, prefix the moved code's references to sibling modules with `crate::` (e.g. `db::init_db` → `crate::db::init_db`, `pipeline::run_pipeline` → `crate::pipeline::run_pipeline`). `cargo build` → fix every reported path → `cargo test` → PASS.

- [ ] **Step 8: Verify and commit**

```bash
cd src-tauri && cargo build && cargo test && cd .. && git diff --stat
```
Expected: PASS; diff shows moves only. Commit:
```bash
git add -A && git commit -m "refactor(TT-63): extract app/ and platform/, scaffold slice modules"
```

---

## Task 2: `library/` slice — folders, images, indexer, watcher

**Files:**
- Create: `src-tauri/src/library/repo.rs`, `library/commands.rs`, `library/models.rs`
- Move: `indexer.rs` → `library/indexer.rs`, `watcher.rs` → `library/watcher.rs`
- Modify: `library/mod.rs`, `db/legacy.rs`, `app/mod.rs` (registry), call sites

**Slice mapping (from spec):** `library/repo.rs` receives `row_to_image`, `insert_folder`, `delete_folder`, `list_folders_with_counts`, `list_all_folders`, `insert_image`, `update_image_hash_changed`, `update_image_metadata`, `clear_image_deleted`, `DbImage`, `get_all_images_for_rescan`, `get_image_metadata_by_path`, `soft_delete_image_by_id`, `soft_delete_image`, `update_thumbnail_path`, `update_preview_path`, `images_needing_preview`, `list_images`, `get_image_by_id`.

- [ ] **Step 1: Wire the slice module**

`src-tauri/src/library/mod.rs`:
```rust
//! Library slice: the photo catalog (folders, images, indexing, watching).
pub mod commands;
pub mod indexer;
pub mod models;
pub mod repo;
pub mod watcher;
```

- [ ] **Step 2: Move the domain types into `library/models.rs`**

Find the `Folder`, `FolderWithCount`, `Image` struct definitions (they live in `models/entities.rs` or `db.rs` — grep: `cd src-tauri && grep -rn "struct Folder\b\|struct Image\b\|struct FolderWithCount" src`). For types currently in `models/entities.rs`, leave them and re-export from `library/models.rs` (`pub use crate::models::{Folder, Image, FolderWithCount};`) to avoid disturbing the shared `models/` crate this pass. Move `DbImage` (defined in `db.rs`) into `library/models.rs`. Build.

- [ ] **Step 3: Move folder/image queries into `library/repo.rs`**

Cut each function listed in the slice mapping from `db/legacy.rs` into `library/repo.rs`. Start the file:
```rust
//! Library persistence: folders + images.
use anyhow::Result;
use sqlx::SqlitePool;
use crate::library::models::{Folder, FolderWithCount, Image, DbImage};
```
Copy whatever additional `use` lines those functions need. Keep every function `pub` and its body byte-for-byte identical.

- [ ] **Step 4: Add re-export bridges in `db/legacy.rs`**

For each moved function, replace its old definition site with a re-export so `crate::db::*` callers stay green:
```rust
pub use crate::library::repo::{
    insert_folder, delete_folder, list_folders_with_counts, list_all_folders,
    insert_image, update_image_hash_changed, update_image_metadata, clear_image_deleted,
    get_all_images_for_rescan, get_image_metadata_by_path, soft_delete_image_by_id,
    soft_delete_image, update_thumbnail_path, update_preview_path, images_needing_preview,
    list_images, get_image_by_id,
};
```
(`row_to_image` is a private helper — move it into `library/repo.rs` as a non-`pub` fn; it has no external callers.) `cargo build` → green.

- [ ] **Step 5: Move `indexer` and `watcher`**

```bash
cd src-tauri/src
git mv indexer.rs library/indexer.rs
git mv watcher.rs library/watcher.rs
```
Remove `mod indexer;` and `mod watcher;` from `lib.rs`. `cargo build` → the compiler flags every `crate::indexer::` / `crate::watcher::` reference (notably in `app/mod.rs` and `pipeline/`). Repoint them to `crate::library::indexer::` / `crate::library::watcher::`. Inside the moved files, repoint their own `crate::db::` calls to `crate::library::repo::` where the target now lives. Build green.

- [ ] **Step 6: Move folder/image commands into `library/commands.rs`**

From `commands.rs`, cut the handlers `add_folder`, `remove_folder`, `list_folders`, `list_images` into `library/commands.rs` (keep `#[tauri::command]` attributes and bodies verbatim). Repoint their internal calls to `crate::library::repo::*`.

- [ ] **Step 7: Repoint the IPC registry**

In `app/mod.rs`, in the `tauri::generate_handler![...]` list, change `commands::add_folder` → `crate::library::commands::add_folder` (and the other three). Build green.

- [ ] **Step 8: Verify and commit**

```bash
cd src-tauri && cargo build && cargo test && cd .. && git diff --stat
git add -A && git commit -m "refactor(TT-63): extract library slice (folders, images, indexer, watcher)"
```

---

## Task 3: `people/` slice — subjects, faces, edges, clustering, face_store, face_quality

**Files:**
- Create: `src-tauri/src/people/repo.rs`, `people/commands.rs`, `people/models.rs`, `people/service.rs`
- Move: `clustering.rs` → `people/clustering.rs`, `face_store.rs` → `people/face_store.rs`, `face_quality.rs` → `people/face_quality.rs`
- Modify: `people/mod.rs`, `db/legacy.rs`, `commands.rs`, `app/mod.rs`, `pipeline/` call sites

**Slice mapping (from spec):** all subject/face/edge/merge functions — see the spec's `people/repo.rs` bullet for the exact 38-function list. Types: `Subject`, `Face`, `MergeSuggestion`, `SubjectDetail`.

- [ ] **Step 1: Wire `people/mod.rs`**
```rust
//! People slice: faces, subjects, clustering, merge suggestions.
pub mod clustering;
pub mod commands;
pub mod face_quality;
pub mod face_store;
pub mod models;
pub mod repo;
pub mod service;
```

- [ ] **Step 2: Move the three self-contained files first (lowest risk)**
```bash
cd src-tauri/src
git mv clustering.rs people/clustering.rs
git mv face_store.rs people/face_store.rs
git mv face_quality.rs people/face_quality.rs
```
Remove their `mod` lines from `lib.rs`. `cargo build` → repoint every `crate::clustering::`, `crate::face_store::`, `crate::face_quality::` reference the compiler flags (heavily used in `pipeline/mod.rs`) to `crate::people::...`. Build green, commit:
```bash
git add -A && git commit -m "refactor(TT-63): move clustering/face_store/face_quality into people slice"
```

- [ ] **Step 3: Move people types into `people/models.rs`**

As in Task 2 Step 2: re-export shared types that live in `models/entities.rs` (`pub use crate::models::{Subject, Face, MergeSuggestion, SubjectDetail};`); move any people-only structs defined in `db.rs`. Build.

- [ ] **Step 4: Move the subject/face/edge queries into `people/repo.rs`**

Cut every function in the spec's `people/repo.rs` list (plus `ordered_pair` as a private helper and `reset_all_subject_data`) from `db/legacy.rs` into `people/repo.rs`. Header:
```rust
//! People persistence: subjects, faces, face-graph edges, merge suggestions.
use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use crate::people::models::{Subject, Face};
```
Add other `use` lines as the functions require (e.g. `crate::models::{MergeSuggestion, SubjectDetail}`). Bodies verbatim.

- [ ] **Step 5: Re-export bridges in `db/legacy.rs`**

Add `pub use crate::people::repo::{ ... all moved names ... };`. `cargo build` green.

- [ ] **Step 6: Move people commands**

Cut from `commands.rs` into `people/commands.rs` the handlers: `list_subjects`, `name_subject`, `list_faces`, `list_faces_for_image`, `get_face_crop`, `set_subject_thumbnail`, `get_subject_photos`, `get_subject_detail`, `get_merge_suggestions`, `merge_subjects`, `dismiss_merge_suggestion`, `assign_face_to_subject`, `create_subject_for_face`, `unassign_face`, `search_subjects`. Repoint internals to `crate::people::repo::*` and `crate::people::clustering::*`. (If any command holds non-trivial orchestration, move that logic into `people/service.rs` and have the command call it — otherwise leave `service.rs` empty/minimal per YAGNI.)

- [ ] **Step 7: Repoint registry + verify + commit**

Update the `people::commands::*` entries in `app/mod.rs`'s handler list.
```bash
cd src-tauri && cargo build && cargo test && cd .. && git diff --stat
git add -A && git commit -m "refactor(TT-63): extract people slice (subjects, faces, clustering, merges)"
```

---

## Task 4: `tags/` slice

**Files:**
- Create: `src-tauri/src/tags/repo.rs`, `tags/commands.rs`, `tags/models.rs`
- Modify: `tags/mod.rs`, `db/legacy.rs`, `commands.rs`, `app/mod.rs`

**Slice mapping (from spec):** `repo.rs` ← `create_tag`, `add_subject_tag`, `remove_subject_tag`, `get_subject_tags`, `list_tags_with_counts`, `rename_tag`, `delete_tag`, `get_tag_image_ids_ordered`, `search_subjects_matching`, `get_subjects_for_tag`, `get_image_ids_for_subjects`. Types: `Tag`, `TagWithCount`, `SubjectMatch`.

- [ ] **Step 1: Wire `tags/mod.rs`**
```rust
//! Tags slice: subject tagging.
pub mod commands;
pub mod models;
pub mod repo;
```

- [ ] **Step 2: Types → `tags/models.rs`**

Move/`pub use` `Tag`, `TagWithCount`, `SubjectMatch` (re-export from `crate::models` if defined there, else move the definitions). Build.

- [ ] **Step 3: Queries → `tags/repo.rs`, add bridges**

Cut the listed functions from `db/legacy.rs` into `tags/repo.rs` (header mirrors Task 2 Step 3). Add `pub use crate::tags::repo::{ ... };` bridges in `db/legacy.rs`. `cargo build` green.

- [ ] **Step 4: Commands → `tags/commands.rs`**

Cut `create_tag`, `add_subject_tag`, `remove_subject_tag`, `get_subject_tags`, `list_tags`, `rename_tag`, `delete_tag`, `get_tag_subjects` from `commands.rs` into `tags/commands.rs`; repoint internals to `crate::tags::repo::*`.

- [ ] **Step 5: Registry + verify + commit**

Update the eight `tags::commands::*` entries in `app/mod.rs`.
```bash
cd src-tauri && cargo build && cargo test && cd .. && git diff --stat
git add -A && git commit -m "refactor(TT-63): extract tags slice"
```

---

## Task 5: `search/` slice — vector_index, search, embedder math, text helpers, embedding cache

**Files:**
- Create: `src-tauri/src/search/repo.rs`, `search/service.rs`, `search/text.rs`, `search/math.rs`, `search/models.rs`
- Move: `vector_index.rs` → `search/vector_index.rs`; fold `search.rs` → `search/service.rs`; `embedder.rs` → `search/math.rs`
- Modify: `search/mod.rs`, `db/legacy.rs`, `commands.rs`, `app/mod.rs`, `pipeline/` and `lib.rs`/`app` call sites

**Slice mapping (from spec):** `repo.rs` ← `get_image_embedding`, `get_all_embeddings`, `get_cached_embedding`, `insert_cached_embedding`, `delete_stale_cache_entries`, `reset_all_embeddings`. `text.rs` ← `normalize`, `like_pattern`, `matches_tokens`. `math.rs` ← contents of `embedder.rs` (`f32_slice_to_bytes`, `bytes_to_f32_vec`, `cosine_similarity`, `emit_progress`).

- [ ] **Step 1: Wire `search/mod.rs`**
```rust
//! Search slice: semantic + deterministic image search.
pub mod commands;
pub mod math;
pub mod models;
pub mod repo;
pub mod service;
pub mod text;
pub mod vector_index;
```

- [ ] **Step 2: Move `vector_index` and `embedder`**
```bash
cd src-tauri/src
git mv vector_index.rs search/vector_index.rs
git mv embedder.rs search/math.rs
```
Remove their `mod` lines from `lib.rs`. `cargo build` → repoint `crate::vector_index::` → `crate::search::vector_index::` and `crate::embedder::` → `crate::search::math::` everywhere the compiler flags (pipeline, app, etc.). Note `IndexStore`/`FlatIndex` types referenced in `AppState` (`app/state.rs`) and `app/mod.rs` setup. Build green, commit.

- [ ] **Step 3: Move `search.rs` body into `search/service.rs`**
```bash
git mv search.rs search/service.rs
```
Remove `mod search;` from `lib.rs`. Repoint `crate::search::search_images` callers — since the slice module is now `search`, the function path becomes `crate::search::service::search_images`. Build green.

- [ ] **Step 4: Move text helpers → `search/text.rs`, math already in place**

Cut `normalize`, `like_pattern`, `matches_tokens` from `db/legacy.rs` into `search/text.rs`. These are used by `tags/repo.rs` and `people` name matching — add bridge `pub use crate::search::text::{normalize, like_pattern};` in `db/legacy.rs` and repoint the obvious internal callers in `tags/repo.rs` to `crate::search::text::normalize`. Build green.

- [ ] **Step 5: Embedding queries → `search/repo.rs`, add bridges**

Cut the `repo.rs` mapping functions from `db/legacy.rs` into `search/repo.rs`; add `pub use crate::search::repo::{ ... };` bridges. Build green.

- [ ] **Step 6: Command + registry + verify + commit**

Cut the `search` handler from `commands.rs` into `search/commands.rs`; repoint internals to `crate::search::service::*` / `crate::search::repo::*`. Update `search::commands::search` in the registry.
```bash
cd src-tauri && cargo build && cargo test && cd .. && git diff --stat
git add -A && git commit -m "refactor(TT-63): extract search slice (index, service, math, text, cache)"
```

---

## Task 6: `media/` + `vision/` slices

**Files:**
- Move: `preview.rs` → `media/preview.rs`, `thumbnail.rs` → `media/thumbnail.rs`; `vision_engine.rs` → `vision/engine.rs`, `preprocess.rs` → `vision/preprocess.rs`
- Create: `media/commands.rs`, `media/mod.rs`, `vision/mod.rs`
- Modify: `db/legacy.rs` (none expected), `commands.rs`, `app/mod.rs`, `pipeline/`, `platform/paths.rs` bridge

- [ ] **Step 1: Wire `media/mod.rs`**
```rust
//! Media slice: decode, resize, cached artifacts (thumbnails, previews, face crops).
pub mod commands;
pub mod preview;
pub mod thumbnail;
```
Wire `vision/mod.rs`:
```rust
//! Vision slice: ONNX runtime engine + tensor preprocessing.
pub mod engine;
pub mod preprocess;
```

- [ ] **Step 2: Move the four files**
```bash
cd src-tauri/src
git mv preview.rs media/preview.rs
git mv thumbnail.rs media/thumbnail.rs
git mv vision_engine.rs vision/engine.rs
git mv preprocess.rs vision/preprocess.rs
```
Remove their `mod` lines from `lib.rs` (`mod preview; mod thumbnail; mod preprocess; pub mod vision_engine;`). `cargo build` → repoint: `crate::preview::` → `crate::media::preview::`, `crate::thumbnail::` → `crate::media::thumbnail::`, `crate::vision_engine::` → `crate::vision::engine::`, `crate::preprocess::` → `crate::vision::preprocess::`. These are referenced in `app/mod.rs` (PreviewService start, cache-dir creation), `app/state.rs` (`PreviewHandle`, `VisionEngine` types), and `pipeline/`. Fix all flagged paths.

- [ ] **Step 3: Resolve the `platform/paths.rs` bridge**

The path helpers re-exported from `thumbnail.rs` in Task 1 Step 5 now live under `media`. Remove the temporary `pub use crate::platform::paths::...` bridge from `media/thumbnail.rs` and instead have `media/thumbnail.rs` call `crate::platform::paths::*` directly where it used the local names. Build green.

- [ ] **Step 4: Media commands**

Cut `prioritize_previews` (and `get_face_crop`, `set_subject_thumbnail` if you judge them media-owned; otherwise they may stay in `people` — pick one and update the registry to match). Recommended: `prioritize_previews` → `media/commands.rs`; leave face-crop/thumbnail commands in `people` since they are subject-centric. Repoint registry entries accordingly.

- [ ] **Step 5: Verify + commit**
```bash
cd src-tauri && cargo build && cargo test && cd .. && git diff --stat
git add -A && git commit -m "refactor(TT-63): extract media and vision slices"
```

---

## Task 7: `settings/` slice

**Files:**
- Move: `settings.rs` → `settings/commands.rs`
- Create: `settings/repo.rs`, `settings/mod.rs`
- Modify: `db/legacy.rs`, `app/mod.rs`

- [ ] **Step 1: Wire `settings/mod.rs`**
```rust
//! Settings slice: app + model configuration.
pub mod commands;
pub mod repo;
```

- [ ] **Step 2: Move `settings.rs` → `settings/commands.rs`**
```bash
cd src-tauri/src
git mv settings.rs settings/commands.rs
```
Remove `mod settings;` from `lib.rs`. `cargo build` → repoint `crate::settings::` references (registry in `app/mod.rs`) to `crate::settings::commands::`.

- [ ] **Step 3: Settings queries → `settings/repo.rs`**

Cut `get_setting` (and `update_setting` if it lives in `db/legacy.rs`) into `settings/repo.rs`. Add `pub use crate::settings::repo::get_setting;` bridge in `db/legacy.rs` (the pipeline reads `db::get_setting` during startup). Repoint the `settings/commands.rs` internal calls to `crate::settings::repo::*`. Build green.

- [ ] **Step 4: Verify + commit**
```bash
cd src-tauri && cargo build && cargo test && cd .. && git diff --stat
git add -A && git commit -m "refactor(TT-63): extract settings slice"
```

---

## Task 8: Repoint `pipeline/` and delete the legacy bridge

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs` (and `embed_actor.rs`, `face_actor.rs` if they reference moved modules)
- Delete: `src-tauri/src/db/legacy.rs`
- Modify: `src-tauri/src/db/mod.rs`

- [ ] **Step 1: Repoint every `crate::db::*` call in `pipeline/` to its real home**

In `pipeline/mod.rs`, change each call to the slice that now owns it (use the spec's dissolution map):
- `crate::db::get_queue_batch`, `mark_semantic_analysis_done`, `mark_subject_analysis_done`, `mark_failed`, `get_processing_counts`, `enqueue_image` → `crate::pipeline::queue::*` (created in Step 2).
- `crate::db::get_image_by_id` → `crate::library::repo::get_image_by_id`.
- `crate::db::insert_face`, `upgrade_subject_thumbnails`, `get_face_with_image` → `crate::people::repo::*`.
- `crate::db::get_setting` → `crate::settings::repo::get_setting`.
- Already-moved sibling modules: `crate::face_quality::*` → `crate::people::face_quality::*`, `crate::face_store::*` → `crate::people::face_store::*`, `crate::clustering::*` → `crate::people::clustering::*`, `crate::embedder::*` → `crate::search::math::*`, `crate::thumbnail::*` → `crate::media::thumbnail::*`, `crate::vision_engine::*` → `crate::vision::engine::*`.

- [ ] **Step 2: Move the queue functions out of legacy into `pipeline/queue.rs`**

Create `src-tauri/src/pipeline/queue.rs`; cut `enqueue_image`, `get_queue_batch`, `mark_semantic_analysis_done`, `mark_subject_analysis_done`, `mark_failed`, `get_processing_counts` from `db/legacy.rs` into it. Add `pub mod queue;` to `pipeline/mod.rs`'s sibling declarations (top of file or in the existing `pub mod` block). `cargo build` → fix paths. Note: `get_processing_counts` is also used by `commands::get_processing_status` — repoint that caller to `crate::pipeline::queue::get_processing_counts`.

- [ ] **Step 3: Confirm `db/legacy.rs` is empty, then delete it**

At this point `db/legacy.rs` should contain only re-export `pub use` bridges (every real definition has moved). Verify:
```bash
cd src-tauri && grep -nE "fn |struct |impl " src/db/legacy.rs
```
Expected: no matches (only `pub use` lines remain). Delete the file and remove its `mod legacy; pub use legacy::*;` lines from `db/mod.rs`:
```bash
git rm src/db/legacy.rs
```

- [ ] **Step 4: Fix the fallout — repoint every remaining `crate::db::*` caller**

`cargo build` now fails on any caller still using a bridged path (the bridges are gone). The compiler lists each one. Repoint each to its real slice (`crate::library::repo::`, `crate::people::repo::`, `crate::tags::repo::`, `crate::search::repo::`, `crate::settings::repo::`, `crate::pipeline::queue::`). This is the moment the temporary `crate::db::*` indirection is fully removed. `db/mod.rs` now contains only `ensure_sqlite_vec_registered`, `init_db`, and the schema. Build green.

- [ ] **Step 5: Move the `#[cfg(test)] mod tests` block**

The old `db.rs` test module (was at `db/legacy.rs` bottom, or already moved) must land with the code it tests. Split it: each `#[test]`/`#[tokio::test]` goes into a `#[cfg(test)] mod tests` at the bottom of the slice `repo.rs` whose functions it exercises (e.g. a `normalize` test → `search/text.rs`; a tag CRUD test → `tags/repo.rs`). Repoint the test bodies' `crate::db::` references. `cargo test` → all prior tests still present and PASS.

- [ ] **Step 6: Full verification + commit**
```bash
cd src-tauri && cargo build && cargo test && cd ..
grep -rn "crate::db::" src-tauri/src | grep -vE "crate::db::(init_db|ensure_sqlite_vec_registered)" || echo "CLEAN: no stray db references"
git diff --stat
git add -A && git commit -m "refactor(TT-63): repoint pipeline to slice APIs, delete legacy db bridge"
```
Expected: build+test PASS; the grep prints `CLEAN`.

- [ ] **Step 7: Manual smoke test**

Launch the app and exercise the IPC surface to catch any registration regression the compiler cannot:
```bash
cd src-tauri && cargo tauri dev   # or the project's run command
```
Verify: add a folder → images appear; run a text search → results render; open People → subjects/faces load; open Tags → tags load; open Settings → models list loads. Any failure here points at a mis-registered command in `app/mod.rs` (check the `generate_handler!` list spelling).

---

## Task 9: Document the architecture in `CLAUDE.md`

**Files:**
- Modify: `CLAUDE.md` (project root)

- [ ] **Step 1: Append the Architecture section**

Add the following section to `CLAUDE.md` (after the existing Tooling/Error-handling sections):

```markdown
# Backend architecture (src-tauri/src) — vertical slices by domain

The backend is organized into **domain slices** (the same "group by feature, not by
type" philosophy as the Angular frontend). Each slice is a folder with a consistent
internal layering. Learn one slice and you know them all.

## Slice map — what lives where
- `app/`        — Tauri Builder, `AppState`, `setup()`, the IPC command registry.
- `library/`    — the photo catalog: folders, images, indexer, fs watcher.
- `media/`      — decode/resize/cached artifacts: previews, thumbnails, face crops.
- `search/`     — semantic + deterministic search: vector index, service, math, text.
- `people/`     — faces, subjects, clustering, merge suggestions.
- `tags/`       — subject tagging.
- `vision/`     — ONNX runtime engine + tensor preprocessing.
- `settings/`   — app + model configuration.
- `models/`     — ML model registry/download/management.
- `pipeline/`   — the background processing engine (decode → embed → faces). An
                  ORCHESTRATOR: it consumes other slices' APIs; it owns no domain logic.
- `db/`         — persistence FOUNDATION only: pool, init, schema, sqlite-vec. No
                  domain queries live here.
- `platform/`   — cross-cutting infra: logging, filesystem paths.

## Layering convention inside a slice
- `commands.rs` — Tauri IPC boundary. Deserialize args, call the service/repo, map
                  errors. NO business logic.
- `service.rs`  — business logic / orchestration. (Optional for tiny slices.)
- `repo.rs`     — persistence. THE ONLY PLACE SQL LIVES.
- `models.rs`   — serde types/DTOs returned to the frontend.

## Rules for agents (respect the architecture)
- Add a Tauri command → implement in the slice's `commands.rs`, then register it in
  `app/mod.rs`'s `generate_handler!` list. NEVER add commands to `lib.rs`.
- Add a query → put it in the owning slice's `repo.rs`. NEVER add a query to `db/`;
  `db/` is foundation only.
- Put no SQL outside a `repo.rs`; put no business logic in a `commands.rs`.
- `pipeline/` consumes domain APIs — do not move domain logic into it.
- New domain? Create a new slice folder following the convention; do not bolt it onto
  an existing slice.

## "Which slice does my change belong to?"
- Folders/images/scanning → `library/`
- Thumbnails/previews/crops → `media/`
- Text/semantic search, embeddings, vector index → `search/`
- Faces/subjects/clustering/merges → `people/`
- Tags → `tags/`
- ONNX/model inference engine → `vision/` (model files/registry → `models/`)
- App settings → `settings/`
- Background processing throughput/queue → `pipeline/`
```

- [ ] **Step 2: Commit**
```bash
git add CLAUDE.md
git commit -m "docs(TT-63): document backend vertical-slice architecture for agents"
```

---

## Final verification (whole-refactor acceptance)

- [ ] `cd src-tauri && cargo build` — PASS
- [ ] `cd src-tauri && cargo test` — PASS (same test count as the pre-refactor baseline; no test deleted)
- [ ] `grep -rn "crate::db::" src-tauri/src | grep -vE "init_db|ensure_sqlite_vec_registered"` — no output
- [ ] `ls src-tauri/src/db.rs` — does not exist (it is now `db/mod.rs`)
- [ ] Manual smoke (Task 8 Step 7) — folder import, search, People, Tags, Settings all work
- [ ] Every Tauri command in `app/mod.rs`'s `generate_handler!` resolves to a `<slice>::commands::*` path
- [ ] `CLAUDE.md` has the Architecture section

## Self-review notes (gaps a reviewer should watch)

- **Shared types in `models/entities.rs`:** the plan re-exports them from each slice's
  `models.rs` rather than relocating them, to avoid churning the shared `models/` crate
  in this pass. If a reviewer prefers types to live in their slice, that is a follow-up.
- **`get_face_crop` / `set_subject_thumbnail` ownership** (media vs people) is a judgment
  call flagged in Task 6 Step 4 — either placement is acceptable as long as the registry
  matches.
- **Commit granularity:** if any single task's `cargo build` cannot be made green by
  import-path fixes alone, STOP — that signals the move boundary is wrong; re-read the
  spec's dissolution map rather than editing logic to force a compile.
