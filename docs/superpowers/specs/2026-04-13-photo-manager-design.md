# Nebula — Photo Manager Design Spec
**Date:** 2026-04-13  
**Status:** Approved

---

## Overview

Nebula is a Tauri 2.0 desktop app (Angular 20 frontend, Rust backend) that helps a camp photographer organize and semantically search tens of thousands of photos across a summer season. The core value is natural-language image search powered by Google's `gemini-embedding-2-preview` multimodal embedding model, which shares a vector space between images and text — enabling queries like "kids on a yellow boat down a stream yelling."

**Target platform:** Native Windows executable. Developed in WSL2, compiled on Windows.  
**Expected scale:** 25k–100k images over ~48 shooting days.

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  Angular Frontend (Tauri WebView)                   │
│  • Gallery view  • Search bar  • Folder manager     │
│  • Embedding status badge                           │
└────────────────────┬────────────────────────────────┘
                     │ Tauri commands + events
┌────────────────────▼────────────────────────────────┐
│  Rust Backend                                       │
│  ┌─────────────┐  ┌────────────┐  ┌─────────────┐  │
│  │ FileWatcher │  │ EmbedQueue │  │  SearchSvc  │  │
│  │  (notify)   │  │ (reqwest)  │  │ (in-memory  │  │
│  └──────┬──────┘  └─────┬──────┘  │ cosine sim) │  │
│         │               │         └──────┬───────┘  │
│  ┌──────▼───────────────▼────────────────▼───────┐  │
│  │              SQLite (sqlx)                    │  │
│  │   folders | images | embedding_queue          │  │
│  └───────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘
                     │ HTTPS (when online)
              Google AI Studio
              gemini-embedding-2-preview
```

All cross-process communication goes through Tauri's typed command/event system. The frontend never reads image files directly — it accesses thumbnails and originals through the Tauri asset protocol.

---

## Data Model

### `folders`
Registered watch roots.

| Column | Type | Notes |
|--------|------|-------|
| `id` | INTEGER PK | |
| `path` | TEXT UNIQUE NOT NULL | Absolute Windows path |
| `added_at` | INTEGER NOT NULL | Unix timestamp |

### `images`
One row per tracked image file.

| Column | Type | Notes |
|--------|------|-------|
| `id` | INTEGER PK | |
| `folder_id` | INTEGER | FK → `folders.id` |
| `path` | TEXT UNIQUE NOT NULL | Absolute path |
| `file_hash` | TEXT NOT NULL | SHA-256, used for change detection |
| `date_taken` | INTEGER | EXIF timestamp; NULL if absent |
| `date_file` | INTEGER NOT NULL | File mtime fallback |
| `thumbnail_path` | TEXT | Path to cached 400px thumbnail |
| `embed_status` | TEXT NOT NULL | `'pending'` \| `'done'` \| `'failed'` |
| `embedding` | BLOB | Raw float32 array; NULL until embedded |
| `added_at` | INTEGER NOT NULL | |
| `updated_at` | INTEGER NOT NULL | |
| `deleted_at` | INTEGER | NULL if present; set on removal event |

Gallery sort key: `COALESCE(date_taken, date_file) DESC`. EXIF is preferred; mtime is the fallback.

### `embedding_queue`
Pending and retrying embedding jobs.

| Column | Type | Notes |
|--------|------|-------|
| `id` | INTEGER PK | |
| `image_id` | INTEGER | FK → `images.id` |
| `attempts` | INTEGER | Default 0 |
| `last_error` | TEXT | Most recent error message |
| `scheduled_at` | INTEGER NOT NULL | Retry-after timestamp (backoff target) |

---

## File Watching & Ingestion

**On folder add:**
1. Recursively scan for `.jpg`, `.jpeg`, `.png` files.
2. Insert each into `images` (if not already present by path) and `embedding_queue`.
3. Register a recursive `notify` watcher on the folder path.

**On filesystem events:**
- **Created** — insert into `images`, enqueue for embedding, generate thumbnail asynchronously.
- **Modified** — recompute SHA-256; if hash differs from stored value, update the `images` row and re-enqueue (replacing any existing queue entry for that image).
- **Removed** — soft-delete the `images` row (retain embedding). If the file reappears at the same path, restore it rather than creating a duplicate.

**Thumbnails** are generated in Rust (400px longest-side JPEG) and written to a cache directory alongside the SQLite DB. This happens immediately on ingestion, independently of the embedding pipeline.

---

## Embedding Pipeline

A persistent Tokio background task processes the queue continuously:

1. Poll `embedding_queue` for rows where `scheduled_at <= now()`, oldest first.
2. For each job: read the image file → base64-encode → POST to `gemini-embedding-2-preview`.
3. **On success:** store embedding BLOB in `images.embedding`, set `embed_status = 'done'`, delete queue row, emit a Tauri event so the frontend updates the status badge.
4. **On failure:** increment `attempts`, set `scheduled_at = now() + min(2^attempts × 30s, 8h)`, store `last_error`.

**Concurrency:** 3 parallel embedding requests by default (configurable).  
**API key:** Stored in a `config.json` file in Tauri's app data directory (`%APPDATA%\nebula\` on Windows); never hardcoded.  
**Offline behaviour:** Network errors trigger backoff. The queue resumes automatically when connectivity returns — no user action required.

---

## Gallery UI

**Layout:**
- **Top bar:** App name, search input (full-width, centered), embedding status badge (amber, shows pending count; hidden when queue is empty).
- **Left sidebar:** List of registered folders with photo counts; "+ Add folder" button at the bottom.
- **Main area:** Photo grid, grouped by day, descending date order. Day headers show relative labels ("Today", "Yesterday") for the two most recent days, then absolute dates.

**Photo grid:** `auto-fill` CSS grid with ~160px minimum column width. Each thumbnail is square, aspect-ratio cropped.

**Embedding status indicator:** A small dot visible on thumbnail hover — green for `done`, amber for `pending`. This gives a quick visual of search coverage without cluttering the UI.

**Theme:** Adapts to the system light/dark mode preference via Angular CDK `MediaMatcher` (`prefers-color-scheme`). Spartan NG components inherit the active theme. No manual toggle needed.

**Virtual scrolling:** The gallery uses Angular CDK virtual scroll to keep rendering performance stable at 25k–100k images.

---

## Search

**Flow:**
1. User types a query and presses Enter.
2. Frontend sends the query string to Rust via Tauri command.
3. Rust calls `gemini-embedding-2-preview` with the text query to get a vector.
4. Rust loads all `embedding` BLOBs where `embed_status = 'done'` into memory, computes cosine similarity against the query vector.
5. Returns top 50 results sorted by similarity score (image IDs + scores).
6. Frontend enters "search results" mode: same day-grouped layout, filtered to those IDs, with a result count shown at the top.

**Offline:** If the query embedding API call fails, show an inline message: *"Search requires a connection — try again when online."*

**Unembedded images:** Simply absent from search results. No error, no special UI — they'll appear once their embedding job completes.

**Clear search:** Clicking × in the search bar returns to the full gallery.

---

## Out of Scope

The following are acknowledged future features, not part of this spec:

- Face labeling, grouping, and person tracking
- Migration to a proper vector DB (e.g. sqlite-vec) for memory-efficient search at scale
- Multi-machine sync or cloud backup

---

## Key Dependencies (Rust)

| Crate | Purpose |
|-------|---------|
| `tauri` v2 | App framework |
| `sqlx` | Async SQLite |
| `notify` | Cross-platform file watching |
| `reqwest` | HTTP client for Google API |
| `image` | Thumbnail generation |
| `serde` / `serde_json` | Serialization |
| `tokio` | Async runtime |
| `sha2` | SHA-256 file hashing |

## Key Dependencies (Angular)

| Package | Purpose |
|---------|---------|
| `@angular/cdk` | Virtual scroll, MediaMatcher |
| `@spartan-ng/brain` | UI components |
| `tailwindcss` | Styling |
| `@tauri-apps/api` | Tauri command/event bindings |
