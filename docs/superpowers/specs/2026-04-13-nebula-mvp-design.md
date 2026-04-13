# Nebula MVP Design

**Date:** 2026-04-13
**Project:** Nebula - Tauri 2.0 Desktop Photo Search App

---

## Overview

Nebula is an offline-first desktop application for photographers that enables natural language search of local images using semantic embeddings. The app uses SigLIP 2 (Google/siglip2-so400m-patch16-naflex) to generate 1152-dimensional embeddings for images and text queries, with cosine similarity for matching.

---

## Architecture

Three-tier Tauri desktop application:

| Layer | Technology | Responsibility |
|-------|------------|----------------|
| Frontend | Angular 20 + Spartan.ng | UI, user interaction |
| Backend | Rust (Tauri 2) | Business logic, SQLite, sidecar orchestration |
| Sidecar | Python + PyTorch + Transformers | SigLIP 2 embedding generation |

**Communication flows:**
- Frontend ↔ Rust: Tauri IPC commands + event emission
- Rust ↔ Sidecar: JSON over stdin/stdout (line-delimited)
- Data persistence: SQLite database (no browser storage)

---

## Database Schema

```sql
-- Folders to index
CREATE TABLE folders (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  path TEXT NOT NULL UNIQUE,
  added_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Images found in folders
CREATE TABLE images (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  file_path TEXT NOT NULL UNIQUE,
  file_name TEXT NOT NULL,
  file_size INTEGER,
  created_at TEXT,
  indexed_at TEXT NOT NULL DEFAULT (datetime('now')),
  embedded INTEGER NOT NULL DEFAULT 0
);

-- 1152-dim embeddings (4608 bytes each)
CREATE TABLE embeddings (
  image_id INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE,
  embedding BLOB NOT NULL
);
```

---

## Rust Backend Modules

| File | Module | Responsibility |
|------|--------|----------------|
| `src-tauri/src/db.rs` | - | SQLite setup, table schemas, CRUD queries |
| `src-tauri/src/scanner.rs` | - | Recursive directory scanning for .jpg/.jpeg/.png files |
| `src-tauri/src/sidecar.rs` | - | Sidecar process spawn, stdin/stdout JSON communication |
| `src-tauri/src/search.rs` | - | Cosine similarity computation, text embedding |
| `src-tauri/src/lib.rs` | - | Tauri command handlers, event emission |

---

## Tauri Commands

| Command | Input | Output | Description |
|---------|-------|--------|-------------|
| `add_folder` | `path: String` | `Folder` | Add directory to track |
| `remove_folder` | `id: i64` | `()` | Remove folder + cascade images |
| `list_folders` | - | `Vec<Folder>` | List all tracked folders |
| `get_indexing_status` | - | `IndexingStatus` | Total images, embedded count |
| `get_images` | `offset, limit, search_filter` | `Vec<ImageResult>` | Paginated images |
| `start_embedding_job` | - | `()` | Generate embeddings for unembedded images |
| `search_images` | `query, limit` | `Vec<SearchResult>` | Semantic image search |
| `sidecar_health` | - | `HealthStatus` | Check if sidecar ready |

---

## Angular Components

| Component | Purpose |
|-----------|---------|
| `AppComponent` | Main layout (sidebar + content area) |
| `FolderManagerComponent` | Folder list, add/remove buttons |
| `EmbeddingStatusComponent` | Progress bar, status text |
| `SearchBarComponent` | Debounced input, search trigger |
| `ImageGridComponent` | Responsive grid, thumbnail display |
| `TauriService` | Wrapper for `invoke()` and `listen()` |

---

## Sidecar Protocol

JSON over stdin/stdout, one JSON object per line.

### Requests (Rust → Python)

```json
{"action": "embed_image", "image_path": "/path/to/photo.jpg"}
{"action": "embed_text", "text": "kids climbing using yellow harnesses"}
{"action": "health_check"}
{"action": "shutdown"}
```

### Responses (Python → Rust)

```json
{"status": "ok", "action": "embed_image", "image_path": "...", "embedding": [0.012, ...]}
{"status": "ok", "action": "embed_text", "text": "...", "embedding": [0.008, ...]}
{"status": "ok", "action": "health_check", "model": "google/siglip2-so400m-patch16-naflex"}
{"status": "error", "message": "..."}
{"status": "ok", "action": "ready"}  // Startup signal
```

### Python Sidecar Implementation (`sidecar/main.py`)

```python
import sys, json
import torch
from transformers import AutoModel, AutoProcessor
from PIL import Image

CHECKPOINT = "google/siglip2-so400m-patch16-naflex"

def main():
    model = AutoModel.from_pretrained(CHECKPOINT).eval()
    processor = AutoProcessor.from_pretrained(CHECKPOINT)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = model.to(device)

    print(json.dumps({"status": "ok", "action": "ready"}), flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line: continue
        try:
            request = json.loads(line)
            action = request.get("action")

            if action == "embed_image":
                image = Image.open(request["image_path"]).convert("RGB")
                inputs = processor(images=[image], return_tensors="pt").to(device)
                with torch.no_grad():
                    features = model.get_image_features(**inputs)
                features = features / features.norm(p=2, dim=-1, keepdim=True)
                embedding = features[0].cpu().tolist()
                print(json.dumps({
                    "status": "ok", "action": "embed_image",
                    "image_path": request["image_path"], "embedding": embedding
                }), flush=True)

            elif action == "embed_text":
                text = f"This is a photo of {request['text'].lower()}."
                inputs = processor(text=[text], padding="max_length",
                    truncation=True, max_length=64, return_tensors="pt").to(device)
                with torch.no_grad():
                    features = model.get_text_features(**inputs)
                features = features / features.norm(p=2, dim=-1, keepdim=True)
                embedding = features[0].cpu().tolist()
                print(json.dumps({
                    "status": "ok", "action": "embed_text",
                    "text": request["text"], "embedding": embedding
                }), flush=True)

            elif action == "health_check":
                print(json.dumps({
                    "status": "ok", "action": "health_check", "model": CHECKPOINT
                }), flush=True)

            elif action == "shutdown":
                break
        except Exception as e:
            print(json.dumps({"status": "error", "message": str(e)}), flush=True)

if __name__ == "__main__":
    main()
```

---

## Data Flow: Search

1. User types query in `SearchBarComponent`
2. Angular debounces 500ms, then invokes `search_images(query, 20)`
3. Rust sends `embed_text` request to sidecar
4. Sidecar returns 1152-dim text embedding (normalized)
5. Rust loads all image embeddings from `embeddings` table
6. Compute cosine similarity (dot product since embeddings are normalized)
7. Return top 20 results with similarity scores
8. Angular displays in `ImageGridComponent` with `convertFileSrc()` for image URLs

---

## Data Flow: Embedding Generation

1. User adds folder → images scanned and stored in `images` table (`embedded=0`)
2. User clicks "Start Embedding" or it triggers automatically
3. `start_embedding_job()` invoked
4. Query all images where `embedded=0`
5. For each image:
   - Send `embed_image` request to sidecar
   - Receive 1152-dim embedding
   - Store as BLOB in `embeddings` table
   - Update `images.embedded=1`
   - Emit `embedding-progress` event with current/total
6. Angular updates progress bar via event listener

---

## Error Handling

| Scenario | Handling |
|----------|----------|
| Sidecar not running | Show "Sidecar unavailable" toast, disable search/embedding |
| Sidecar crashes | Attempt restart once, then show error to user |
| Image file not found | Skip with warning, continue embedding job |
| Invalid image format | Skip, log warning |
| Database locked | Retry with backoff (max 3 attempts) |
| No search results | Show "No images found" message in grid |
| Folder path invalid | Show error toast via hlmSonner |
| Sidecar model loading (15-30s) | Show loading spinner, disable features until `ready` |

---

## Dependencies

### Rust (Cargo.toml additions)

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
tauri-plugin-dialog = "2"
tauri-plugin-shell = "2"
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
walkdir = "2"
byteorder = "1"
```

### Python (sidecar/requirements.txt)

```
torch
transformers>=4.49.0
Pillow
```

### Angular (package.json additions)

```json
{
  "dependencies": {
    "@tauri-apps/plugin-dialog": "^2",
    "@tauri-apps/plugin-shell": "^2",
    "@spartan-ng/ui-button-helm": "0.0.1-alpha.668",
    "@spartan-ng/ui-card-helm": "0.0.1-alpha.668",
    "@spartan-ng/ui-input-helm": "0.0.1-alpha.668",
    "@spartan-ng/ui-sonner-helm": "0.0.1-alpha.668",
    "@spartan-ng/ui-progress-helm": "0.0.1-alpha.668",
    "@spartan-ng/ui-badge-helm": "0.0.1-alpha.668",
    "@spartan-ng/ui-scroll-area-helm": "0.0.1-alpha.668"
  }
}
```

---

## Development vs Production (Sidecar)

### Development
Run Python script directly:
```bash
cd sidecar/
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
python main.py
```
Tauri spawns: `python /path/to/sidecar/main.py`

### Production
Bundle with PyInstaller:
```bash
cd sidecar/
pip install pyinstaller
pyinstaller -F main.py --name Nebula-sidecar
cp dist/Nebula-sidecar ../src-tauri/binaries/
```
Configure in `tauri.conf.json`:
```json
{
  "bundle": {
    "externalBin": ["binaries/Nebula-sidecar"]
  }
}
```

---

## Implementation Order

1. **Database setup** — `db.rs` with all tables, initialize on app startup
2. **Folder management** — Rust commands + Angular UI (add/remove/list)
3. **Image scanning** — Scan folders on add, store in database
4. **Sidecar integration** — Spawn process, JSON protocol, stdin/stdout
5. **Embedding generation** — Job to embed unembedded images, emit progress
6. **Search** — Cosine similarity in Rust, search bar in Angular, display results
7. **Polish** — Loading states, error handling, UI refinements

---

## UI Design Notes

- **Dark mode by default** — Professional photographer tool aesthetic
- **Layout:** Sidebar (folders, status) + Content area (search, grid)
- **Theming:** Spartan.ng CSS variables, neutral palette with single accent
- **Image thumbnails:** Maintain aspect ratio, `object-fit: cover` for uniform grid
- **Loading states:** Spinner during sidecar initialization (15-30s on first run)

---

## Important Implementation Notes

- Embeddings stored as raw f32 bytes (1152 × 4 = 4608 bytes each)
- Normalize all embeddings to unit vectors before storing
- Cosine similarity = dot product for normalized vectors
- Text queries are automatically wrapped with prompt template: `"This is a photo of {query}."` in the sidecar
- Use `flush=True` on all Python `print()` calls
- Handle sidecar startup gracefully — model loading takes time
- No `localStorage` or `sessionStorage` in Angular
