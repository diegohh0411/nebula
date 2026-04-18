# Image Search: Unified Command + Embedding Cache

## Problem

The lightbox magnifying glass "Find Similar" button redirects to the gallery but shows no results and no loading indicator. Additionally, the search bar only accepts text — there is no way to search by pasting or dropping an external image.

## Solution

Replace `search_images` and `search_similar_images` with a single unified `search` command that handles text, library image IDs, and raw image bytes. Add an `embedding_cache` table to avoid redundant Gemini API calls for repeated text queries and external images within a 30-minute window. Extend the search bar UI to accept images via drag-and-drop and clipboard paste, and to display a thumbnail chip when searching by image.

## Backend

### Embedding cache table

```sql
CREATE TABLE IF NOT EXISTS embedding_cache (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cache_key TEXT NOT NULL UNIQUE,
    query_type TEXT NOT NULL CHECK(query_type IN ('text', 'image')),
    embedding BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
```

- `cache_key`: SHA-256 hex digest of the query text (for text) or the raw image bytes (for images).
- `query_type`: Discriminant for debugging and future cleanup.
- `embedding`: `Vec<f32>` serialized as little-endian bytes (same format as `images.embedding`).
- `created_at`: Unix timestamp. Entries older than 30 minutes are treated as stale on read and ignored.

### Unified search command

New Rust enum, tagged by `type` with camelCase renaming:

```rust
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SearchQuery {
    Text { query: String },
    ImageId { image_id: i64 },
    ImageBytes { data: String, mime_type: String },
}
```

New Tauri command:

```rust
#[tauri::command]
pub async fn search(query: SearchQuery, state: State<'_, AppState>) -> Result<Vec<SearchResult>, String>
```

Each variant flows as follows:

**`text`**:
1. SHA-256 hash the query string.
2. Look up in `embedding_cache` where `query_type = 'text'` and `created_at > now - 30min`.
3. Cache miss: call `embedder::embed_text()` via Gemini API. Insert into cache.
4. Also run fuzzy subject name search (existing `search_subjects_by_name` logic preserved).
5. Merge subject matches (score 1.0) with cosine similarity results, deduplicate.

**`imageId`**:
1. Load embedding from `images.embedding` column (already stored during indexing — no API call, no cache needed).
2. Run cosine similarity search against all image embeddings.
3. Exclude the source image from results.

**`imageBytes`**:
1. Decode base64 `data` to raw bytes. SHA-256 hash the raw bytes.
2. Look up in `embedding_cache` where `query_type = 'image'` and `created_at > now - 30min`.
3. Cache miss: call `embedder::embed_image()` via Gemini API with the decoded bytes and `mime_type`. Insert into cache.
4. Run cosine similarity search against all image embeddings.

All three variants use the existing `search::search_images()` cosine similarity engine with the gap heuristic cutoff.

### Deprecation

Remove `search_images` and `search_similar_images` commands. Update `lib.rs` to register only the new `search` command.

### Cache cleanup

Lazy: stale entries are ignored on read. No background cleanup job required for MVP. A `DELETE FROM embedding_cache WHERE created_at < ?` can be run opportunistically during search.

## Frontend

### PhotoService changes

New signals:

```typescript
readonly searchImage = signal<{ thumbnailUrl: string; type: 'library' | 'external' } | null>(null);
readonly searchText = signal<string>('');
```

Modified methods:

- `searchByText(query: string)`: sets `searchText`, clears `searchImage`, invokes unified `search` with `{ type: 'text', query }`.
- `searchByImage(image: Image | SearchResult)`: sets `searchImage` with the image's thumbnail, clears `searchText`, invokes unified `search` with `{ type: 'imageId', imageId }`.
- `searchByExternalImage(base64Data: string, mimeType: string, objectUrl: string)`: sets `searchImage` with the blob URL and type `'external'`, clears `searchText`, invokes unified `search` with `{ type: 'imageBytes', data: base64Data, mimeType }`.
- `clearSearch()`: resets `searchResults`, `searchError`, `searchImage`, and `searchText`.

### Search bar component

State driven by PhotoService signals:

- **No search active**: text input displayed (existing behavior).
- **Image search active**: thumbnail chip replaces the text input. Chip shows a small thumbnail with an X button to clear. A spinner shows while `isSearching` is true.
- **Text search active**: text input shows the query (existing behavior).

Image input handlers:

- **Drag-and-drop**: `dragover` (prevent default, show drop zone highlight) + `drop` (read `DataTransfer.files[0]`, convert to base64, create object URL, call `searchByExternalImage`).
- **Clipboard paste**: `paste` event on the search bar. Inspect `ClipboardEvent.clipboardData.items` for image types. Read as base64, call `searchByExternalImage`.

### Lightbox flow

`findSimilar()` in `lightbox.component.ts`:

1. Set `photos.searchImage` with `{ thumbnailUrl: image.thumbnail_path, type: 'library' }`.
2. Call `photos.searchByImage(image)` which triggers the unified search.
3. Close the lightbox.

Result: lightbox closes, gallery shows "Search Results" group, search bar displays the source image thumbnail chip.

## Scope

- In scope: unified search command, embedding cache, search bar image input, lightbox magnifying glass fix, drag-and-drop and paste support.
- Out of scope: file picker button, vector index optimization, EXIF metadata search, search history persistence.
