# Nebula MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an offline-first desktop photo search app using SigLIP 2 embeddings for semantic image search.

**Architecture:** Angular 20 frontend with Spartan.ng primitives + Tailwind, Rust/Tauri 2 backend with SQLite (rusqlite), Python sidecar for ML inference (SigLIP 2). Frontend ↔ Rust via Tauri IPC, Rust ↔ Python via JSON over stdin/stdout.

**Tech Stack:** Tauri 2, Angular 20, @spartan-ng/brain, Tailwind CSS v4, rusqlite (bundled), Python 3 + PyTorch + Transformers

---

## File Structure

**Rust (create):**
- `src-tauri/src/db.rs` — SQLite initialization, table schemas, CRUD queries
- `src-tauri/src/scanner.rs` — Recursive directory scanning for image files
- `src-tauri/src/sidecar.rs` — Python process spawn, stdin/stdout JSON protocol
- `src-tauri/src/search.rs` — Cosine similarity (dot product on normalized vectors)

**Rust (modify):**
- `src-tauri/Cargo.toml` — Add dependencies
- `src-tauri/src/lib.rs` — State management, command handlers, plugin registration
- `src-tauri/tauri.conf.json` — Window size, sidecar config
- `src-tauri/capabilities/default.json` — Shell and dialog permissions

**Python (create):**
- `sidecar/main.py` — SigLIP 2 embedding server (stdin/stdout JSON protocol)
- `sidecar/requirements.txt` — Python dependencies

**Angular (create):**
- `src/app/services/tauri.service.ts` — Tauri invoke/listen wrapper
- `src/app/components/folder-manager/folder-manager.component.ts` — Folder list UI
- `src/app/components/folder-manager/folder-manager.component.html`
- `src/app/components/folder-manager/folder-manager.component.css`
- `src/app/components/search-bar/search-bar.component.ts` — Debounced search input
- `src/app/components/search-bar/search-bar.component.html`
- `src/app/components/image-grid/image-grid.component.ts` — Results grid
- `src/app/components/image-grid/image-grid.component.html`
- `src/app/components/image-grid/image-grid.component.css`
- `src/app/components/embedding-status/embedding-status.component.ts` — Progress indicator
- `src/app/components/embedding-status/embedding-status.component.html`

**Angular (modify):**
- `package.json` — Add Tauri plugins, Tailwind CSS
- `src/app/app.component.ts` — Main layout (sidebar + content)
- `src/app/app.component.html` — Sidebar/content template
- `src/app/app.component.css` — Layout styles
- `src/styles.css` — Dark mode theme tokens

---

### Task 1: Rust Dependencies & Configuration

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `src-tauri/capabilities/default.json`

- [ ] **Step 1: Update Cargo.toml with all dependencies**

Replace the entire `[dependencies]` section in `src-tauri/Cargo.toml`:

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
tauri-plugin-dialog = "2"
tauri-plugin-shell = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite = { version = "0.32", features = ["bundled"] }
walkdir = "2"
byteorder = "1"
```

- [ ] **Step 2: Update tauri.conf.json**

Set window to 1280x800 and add sidecar config. Replace the `app` and `bundle` sections:

```json
{
  "app": {
    "windows": [
      {
        "title": "Nebula",
        "width": 1280,
        "height": 800,
        "minWidth": 800,
        "minHeight": 600
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "externalBin": ["binaries/Nebula-sidecar"]
  }
}
```

- [ ] **Step 3: Update capabilities/default.json**

Add shell and dialog permissions:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "opener:default",
    "dialog:default",
    "dialog:allow-open",
    "shell:default",
    {
      "identifier": "shell:allow-execute",
      "allow": [{ "name": "binaries/Nebula-sidecar", "sidecar": true }]
    }
  ]
}
```

- [ ] **Step 4: Verify Rust compilation**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles with warnings (unused imports are fine). No errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/capabilities/default.json
git commit -m "chore: add Rust dependencies and Tauri config for MVP"
```

---

### Task 2: Database Module

**Files:**
- Create: `src-tauri/src/db.rs`

- [ ] **Step 1: Create db.rs with initialization and types**

Create `src-tauri/src/db.rs`:

```rust
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Folder {
    pub id: i64,
    pub path: String,
    pub added_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageRecord {
    pub id: i64,
    pub folder_id: i64,
    pub file_path: String,
    pub file_name: String,
    pub file_size: Option<i64>,
    pub created_at: Option<String>,
    pub indexed_at: String,
    pub embedded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexingStatus {
    pub total: i64,
    pub embedded: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: i64,
    pub file_path: String,
    pub file_name: String,
    pub similarity: f64,
}

pub fn init_db(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS folders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            added_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS images (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
            file_path TEXT NOT NULL UNIQUE,
            file_name TEXT NOT NULL,
            file_size INTEGER,
            created_at TEXT,
            indexed_at TEXT NOT NULL DEFAULT (datetime('now')),
            embedded INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS embeddings (
            image_id INTEGER PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE,
            embedding BLOB NOT NULL
        );",
    )?;
    Ok(())
}

pub fn add_folder(conn: &Connection, path: &str) -> SqlResult<Folder> {
    conn.execute("INSERT INTO folders (path) VALUES (?1)", params![path])?;
    let id = conn.last_insert_rowid();
    Ok(Folder {
        id,
        path: path.to_string(),
        added_at: String::new(),
    })
}

pub fn remove_folder(conn: &Connection, id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM folders WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_folders(conn: &Connection) -> SqlResult<Vec<Folder>> {
    let mut stmt = conn.prepare("SELECT id, path, added_at FROM folders ORDER BY added_at DESC")?;
    let rows = stmt.query_map([], |row| {
        Ok(Folder {
            id: row.get(0)?,
            path: row.get(1)?,
            added_at: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn add_images(conn: &Connection, folder_id: i64, images: &[(String, String, Option<i64>)]) -> SqlResult<usize> {
    let mut count = 0;
    for (file_path, file_name, file_size) in images {
        match conn.execute(
            "INSERT OR IGNORE INTO images (folder_id, file_path, file_name, file_size) VALUES (?1, ?2, ?3, ?4)",
            params![folder_id, file_path, file_name, file_size],
        ) {
            Ok(_) => count += 1,
            Err(_) => continue,
        }
    }
    Ok(count)
}

pub fn remove_images_for_folder(conn: &Connection, folder_id: i64) -> SqlResult<usize> {
    Ok(conn.execute("DELETE FROM images WHERE folder_id = ?1", params![folder_id])?)
}

pub fn get_unembedded_images(conn: &Connection) -> SqlResult<Vec<ImageRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder_id, file_path, file_name, file_size, created_at, indexed_at, embedded
         FROM images WHERE embedded = 0"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ImageRecord {
            id: row.get(0)?,
            folder_id: row.get(1)?,
            file_path: row.get(2)?,
            file_name: row.get(3)?,
            file_size: row.get(4)?,
            created_at: row.get(5)?,
            indexed_at: row.get(6)?,
            embedded: row.get::<_, i32>(7)? != 0,
        })
    })?;
    rows.collect()
}

pub fn store_embedding(conn: &Connection, image_id: i64, embedding: &[u8]) -> SqlResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (image_id, embedding) VALUES (?1, ?2)",
        params![image_id, embedding],
    )?;
    Ok(())
}

pub fn mark_embedded(conn: &Connection, image_id: i64) -> SqlResult<()> {
    conn.execute("UPDATE images SET embedded = 1 WHERE id = ?1", params![image_id])?;
    Ok(())
}

pub fn get_indexing_status(conn: &Connection) -> SqlResult<IndexingStatus> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM images", [], |row| row.get(0))?;
    let embedded: i64 = conn.query_row("SELECT COUNT(*) FROM images WHERE embedded = 1", [], |row| row.get(0))?;
    Ok(IndexingStatus { total, embedded })
}

pub fn get_all_embeddings(conn: &Connection) -> SqlResult<Vec<(i64, String, String, Vec<u8>)>> {
    let mut stmt = conn.prepare(
        "SELECT i.id, i.file_path, i.file_name, e.embedding
         FROM images i JOIN embeddings e ON i.id = e.image_id"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    rows.collect()
}

pub fn get_images_paginated(conn: &Connection, offset: i64, limit: i64) -> SqlResult<Vec<ImageRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder_id, file_path, file_name, file_size, created_at, indexed_at, embedded
         FROM images ORDER BY indexed_at DESC LIMIT ?1 OFFSET ?2"
    )?;
    let rows = stmt.query_map(params![limit, offset], |row| {
        Ok(ImageRecord {
            id: row.get(0)?,
            folder_id: row.get(1)?,
            file_path: row.get(2)?,
            file_name: row.get(3)?,
            file_size: row.get(4)?,
            created_at: row.get(5)?,
            indexed_at: row.get(6)?,
            embedded: row.get::<_, i32>(7)? != 0,
        })
    })?;
    rows.collect()
}

pub fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn test_init_db() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(init_db(&conn).is_ok());
        // Tables exist
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM folders", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_add_and_list_folders() {
        let conn = test_db();
        let folder = add_folder(&conn, "/tmp/photos").unwrap();
        assert_eq!(folder.path, "/tmp/photos");

        let folders = list_folders(&conn).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "/tmp/photos");
    }

    #[test]
    fn test_add_duplicate_folder_fails() {
        let conn = test_db();
        add_folder(&conn, "/tmp/photos").unwrap();
        assert!(add_folder(&conn, "/tmp/photos").is_err());
    }

    #[test]
    fn test_remove_folder_cascades() {
        let conn = test_db();
        let folder = add_folder(&conn, "/tmp/photos").unwrap();
        add_images(&conn, folder.id, &[
            ("/tmp/photos/a.jpg".into(), "a.jpg".into(), Some(1024)),
        ]).unwrap();
        remove_folder(&conn, folder.id).unwrap();
        let images = get_unembedded_images(&conn).unwrap();
        assert!(images.is_empty());
    }

    #[test]
    fn test_embedding_roundtrip() {
        let original: Vec<f32> = vec![0.1, -0.2, 0.3, 1.0, -0.5];
        let bytes = embedding_to_bytes(&original);
        assert_eq!(bytes.len(), 20); // 5 floats * 4 bytes
        let restored = bytes_to_embedding(&bytes);
        for (a, b) in original.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_indexing_status() {
        let conn = test_db();
        let folder = add_folder(&conn, "/tmp/photos").unwrap();
        add_images(&conn, folder.id, &[
            ("/tmp/photos/a.jpg".into(), "a.jpg".into(), Some(1024)),
            ("/tmp/photos/b.png".into(), "b.png".into(), Some(2048)),
        ]).unwrap();
        let status = get_indexing_status(&conn).unwrap();
        assert_eq!(status.total, 2);
        assert_eq!(status.embedded, 0);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/pi/nebula/src-tauri && cargo test --lib db`
Expected: All 6 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add database module with SQLite schema and CRUD operations"
```

---

### Task 3: Scanner Module

**Files:**
- Create: `src-tauri/src/scanner.rs`

- [ ] **Step 1: Create scanner.rs**

Create `src-tauri/src/scanner.rs`:

```rust
use std::path::Path;
use walkdir::WalkDir;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

pub struct ScannedImage {
    pub file_path: String,
    pub file_name: String,
    pub file_size: Option<i64>,
}

pub fn scan_directory(dir: &Path) -> Result<Vec<ScannedImage>, String> {
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }

    let mut images = Vec::new();

    for entry in WalkDir::new(dir).follow_links(true).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase());

        let Some(ext) = extension else { continue };

        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let file_size = entry.metadata().ok().map(|m| m.len() as i64);

        images.push(ScannedImage {
            file_path: path.to_string_lossy().to_string(),
            file_name,
            file_size,
        });
    }

    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_nonexistent_directory() {
        let result = scan_directory(Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_filters_by_extension() {
        let dir = std::env::temp_dir().join("nebula_test_scan");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("photo.jpg"), "fake jpg").unwrap();
        fs::write(dir.join("photo.PNG"), "fake png").unwrap();
        fs::write(dir.join("photo.JPEG"), "fake jpeg").unwrap();
        fs::write(dir.join("document.pdf"), "fake pdf").unwrap();
        fs::write(dir.join("notes.txt"), "fake txt").unwrap();

        let images = scan_directory(&dir).unwrap();
        let names: Vec<&str> = images.iter().map(|i| i.file_name.as_str()).collect();
        assert!(names.contains(&"photo.jpg"));
        assert!(names.contains(&"photo.PNG"));
        assert!(names.contains(&"photo.JPEG"));
        assert!(!names.contains(&"document.pdf"));
        assert!(!names.contains(&"notes.txt"));
        assert_eq!(images.len(), 3);

        let _ = fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/pi/nebula/src-tauri && cargo test --lib scanner`
Expected: Both tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/scanner.rs
git commit -m "feat: add image scanner for recursive directory scanning"
```

---

### Task 4: Python Sidecar

**Files:**
- Create: `sidecar/main.py`
- Modify: `sidecar/requirements.txt`

- [ ] **Step 1: Create sidecar/main.py**

```python
import sys
import json
import torch
from transformers import AutoModel, AutoProcessor
from PIL import Image

CHECKPOINT = "google/siglip2-so400m-patch16-naflex"


def main():
    print(json.dumps({"status": "loading", "action": "ready"}), flush=True)

    model = AutoModel.from_pretrained(CHECKPOINT).eval()
    processor = AutoProcessor.from_pretrained(CHECKPOINT)
    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = model.to(device)

    print(json.dumps({"status": "ok", "action": "ready"}), flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
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
                    "status": "ok",
                    "action": "embed_image",
                    "image_path": request["image_path"],
                    "embedding": embedding,
                }), flush=True)

            elif action == "embed_text":
                text = f"This is a photo of {request['text'].lower()}."
                inputs = processor(
                    text=[text],
                    padding="max_length",
                    truncation=True,
                    max_length=64,
                    return_tensors="pt",
                ).to(device)
                with torch.no_grad():
                    features = model.get_text_features(**inputs)
                features = features / features.norm(p=2, dim=-1, keepdim=True)
                embedding = features[0].cpu().tolist()
                print(json.dumps({
                    "status": "ok",
                    "action": "embed_text",
                    "text": request["text"],
                    "embedding": embedding,
                }), flush=True)

            elif action == "health_check":
                print(json.dumps({
                    "status": "ok",
                    "action": "health_check",
                    "model": CHECKPOINT,
                }), flush=True)

            elif action == "shutdown":
                break

        except Exception as e:
            print(json.dumps({
                "status": "error",
                "action": request.get("action", "unknown") if request else "unknown",
                "message": str(e),
            }), flush=True)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Write requirements.txt**

```
torch
transformers>=4.49.0
Pillow
```

- [ ] **Step 3: Commit**

```bash
git add sidecar/main.py sidecar/requirements.txt
git commit -m "feat: add Python sidecar for SigLIP 2 embedding generation"
```

---

### Task 5: Sidecar Management

**Files:**
- Create: `src-tauri/src/sidecar.rs`

- [ ] **Step 1: Create sidecar.rs**

Create `src-tauri/src/sidecar.rs`:

```rust
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

pub struct SidecarProcess {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct SidecarManager {
    process: Option<SidecarProcess>,
    ready: bool,
}

impl SidecarManager {
    pub fn new() -> Self {
        SidecarManager {
            process: None,
            ready: false,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.process.is_some() {
            self.shutdown()?;
        }

        let script_path = std::env::var("CARGO_MANIFEST_DIR")
            .map(|d| {
                std::path::Path::new(&d)
                    .parent()
                    .map(|p| p.join("sidecar").join("main.py"))
            })
            .ok()
            .flatten()
            .unwrap_or_else(|| std::path::PathBuf::from("../sidecar/main.py"));

        let python = if cfg!(windows) { "python" } else { "python3" };

        let mut child = Command::new(python)
            .arg(&script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("Failed to start sidecar: {}. Path: {}", e, script_path.display()))?;

        let stdin = child.stdin.take().ok_or("Failed to get sidecar stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get sidecar stdout")?;
        let mut reader = BufReader::new(stdout);

        // Wait for ready signal
        let mut ready_line = String::new();
        reader.read_line(&mut ready_line).map_err(|e| format!("Failed to read sidecar ready: {}", e))?;
        let ready_msg: Value = serde_json::from_str(ready_line.trim())
            .map_err(|e| format!("Invalid sidecar ready message: {}", e))?;

        if ready_msg["action"] == "ready" && ready_msg["status"] == "loading" {
            // Model is loading, wait for the real ready
            ready_line.clear();
            reader.read_line(&mut ready_line).map_err(|e| format!("Failed to read sidecar ready: {}", e))?;
            let msg: Value = serde_json::from_str(ready_line.trim())
                .map_err(|e| format!("Invalid sidecar ready message: {}", e))?;
            if msg["status"] != "ok" {
                return Err(format!("Sidecar failed to initialize: {}", msg));
            }
        }

        self.process = Some(SidecarProcess {
            stdin,
            stdout: reader,
        });
        self.ready = true;

        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        if let Some(ref mut proc) = self.process {
            let _ = self.send_raw(&json!({"action": "shutdown"}));
            let _ = proc.stdin.flush();
            // Give the process a moment to exit gracefully, then drop (which kills it)
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        self.process = None;
        self.ready = false;
        Ok(())
    }

    pub fn send_request(&mut self, request: &Value) -> Result<Value, String> {
        if !self.ready || self.process.is_none() {
            return Err("Sidecar is not running".to_string());
        }
        self.send_raw(request)?;

        let proc = self.process.as_mut().ok_or("Sidecar process lost")?;
        let mut line = String::new();
        proc.stdout.read_line(&mut line).map_err(|e| format!("Failed to read sidecar response: {}", e))?;

        if line.trim().is_empty() {
            return Err("Sidecar returned empty response".to_string());
        }

        let response: Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("Invalid sidecar response: {} - {}", e, line.trim()))?;

        if response["status"] == "error" {
            return Err(response["message"].as_str().unwrap_or("Unknown error").to_string());
        }

        Ok(response)
    }

    fn send_raw(&mut self, request: &Value) -> Result<(), String> {
        let proc = self.process.as_mut().ok_or("Sidecar process lost")?;
        let msg = format!("{}\n", serde_json::to_string(request).map_err(|e| e.to_string())?);
        proc.stdin.write_all(msg.as_bytes()).map_err(|e| format!("Failed to write to sidecar: {}", e))?;
        proc.stdin.flush().map_err(|e| format!("Failed to flush sidecar stdin: {}", e))?;
        Ok(())
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/sidecar.rs
git commit -m "feat: add sidecar process manager for Python embedding server"
```

---

### Task 6: Search Module

**Files:**
- Create: `src-tauri/src/search.rs`

- [ ] **Step 1: Create search.rs**

Create `src-tauri/src/search.rs`:

```rust
use crate::db::{bytes_to_embedding, SearchResult};

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

pub fn search_embeddings(
    query_embedding: &[f32],
    all_embeddings: &[(i64, String, String, Vec<u8>)],
    limit: usize,
) -> Vec<SearchResult> {
    let mut scored: Vec<SearchResult> = all_embeddings
        .iter()
        .map(|(id, file_path, file_name, embedding_bytes)| {
            let embedding = bytes_to_embedding(embedding_bytes);
            let similarity = cosine_similarity(query_embedding, &embedding) as f64;
            SearchResult {
                id: *id,
                file_path: file_path.clone(),
                file_name: file_name.clone(),
                similarity,
            }
        })
        .collect();

    scored.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::embedding_to_bytes;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_search_returns_top_results() {
        let query = vec![1.0, 0.0, 0.0];
        let embeddings = vec![
            (1i64, "/a.jpg".into(), "a.jpg".into(), embedding_to_bytes(&[0.9, 0.1, 0.0])),
            (2i64, "/b.jpg".into(), "b.jpg".into(), embedding_to_bytes(&[0.0, 1.0, 0.0])),
            (3i64, "/c.jpg".into(), "c.jpg".into(), embedding_to_bytes(&[0.8, 0.2, 0.0])),
        ];
        let results = search_embeddings(&query, &embeddings, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, 1); // 0.9 similarity
        assert_eq!(results[1].id, 3); // 0.8 similarity
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd /home/pi/nebula/src-tauri && cargo test --lib search`
Expected: All 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/search.rs
git commit -m "feat: add cosine similarity search module"
```

---

### Task 7: Tauri Command Handlers (lib.rs)

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Rewrite lib.rs with all commands and state management**

Replace the entire contents of `src-tauri/src/lib.rs`:

```rust
mod db;
mod scanner;
mod search;
mod sidecar;

use db::*;
use scanner::*;
use search::*;
use sidecar::*;
use serde_json::json;
use std::sync::Mutex;
use tauri::Emitter;

pub struct AppState {
    db: Mutex<rusqlite::Connection>,
    sidecar: Mutex<SidecarManager>,
}

#[tauri::command]
fn add_folder(path: String, state: tauri::State<'_, AppState>) -> Result<Folder, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let folder = db::add_folder(&conn, &path)?;

    let scanned = scan_directory(std::path::Path::new(&path))?;
    let image_data: Vec<(String, String, Option<i64>)> = scanned
        .iter()
        .map(|img| (img.file_path.clone(), img.file_name.clone(), img.file_size))
        .collect();
    db::add_images(&conn, folder.id, &image_data)?;

    Ok(folder)
}

#[tauri::command]
fn remove_folder(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::remove_folder(&conn, id)?;
    Ok(())
}

#[tauri::command]
fn list_folders(state: tauri::State<'_, AppState>) -> Result<Vec<Folder>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::list_folders(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_indexing_status(state: tauri::State<'_, AppState>) -> Result<IndexingStatus, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_indexing_status(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_images(offset: i64, limit: i64, state: tauri::State<'_, AppState>) -> Result<Vec<ImageRecord>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::get_images_paginated(&conn, offset, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn start_sidecar(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    sidecar.start()
}

#[tauri::command]
fn sidecar_health(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    if !sidecar.is_ready() {
        return Ok(false);
    }
    let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    match sidecar.send_request(&json!({"action": "health_check"})) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
fn stop_sidecar(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
    sidecar.shutdown()
}

#[tauri::command]
fn start_embedding_job(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let images = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get_unembedded_images(&conn).map_err(|e| e.to_string())?
    };

    if images.is_empty() {
        return Ok(());
    }

    let total = images.len();
    let app_handle = app.clone();

    std::thread::spawn(move || {
        let state = app_handle.state::<AppState>();

        for (i, image) in images.iter().enumerate() {
            let request = json!({
                "action": "embed_image",
                "image_path": image.file_path
            });

            let response = {
                let mut sidecar = match state.sidecar.lock() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                sidecar.send_request(&request)
            };

            match response {
                Ok(resp) => {
                    if let Some(embedding_arr) = resp.get("embedding").and_then(|e| e.as_array()) {
                        let embedding: Vec<f32> = embedding_arr
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();

                        let bytes = embedding_to_bytes(&embedding);

                        let conn = match state.db.lock() {
                            Ok(c) => c,
                            Err(_) => break,
                        };
                        let _ = db::store_embedding(&conn, image.id, &bytes);
                        let _ = db::mark_embedded(&conn, image.id);
                    }
                }
                Err(_) => continue,
            }

            let _ = app_handle.emit(
                "embedding-progress",
                json!({"current": i + 1, "total": total}),
            );
        }

        let _ = app_handle.emit("embedding-complete", json!({}));
    });

    Ok(())
}

#[tauri::command]
fn search_images(query: String, limit: usize, state: tauri::State<'_, AppState>) -> Result<Vec<SearchResult>, String> {
    let text_request = json!({
        "action": "embed_text",
        "text": query
    });

    let query_embedding = {
        let mut sidecar = state.sidecar.lock().map_err(|e| e.to_string())?;
        let response = sidecar.send_request(&text_request)?;

        let embedding_arr = response.get("embedding")
            .and_then(|e| e.as_array())
            .ok_or("No embedding in response")?;

        embedding_arr
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect::<Vec<f32>>()
    };

    let all_embeddings = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get_all_embeddings(&conn).map_err(|e| e.to_string())?
    };

    Ok(search_embeddings(&query_embedding, &all_embeddings, limit))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            add_folder,
            remove_folder,
            list_folders,
            get_indexing_status,
            get_images,
            start_sidecar,
            stop_sidecar,
            sidecar_health,
            start_embedding_job,
            search_images,
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()
                .expect("Failed to resolve app data directory");
            std::fs::create_dir_all(&app_data_dir)
                .expect("Failed to create app data directory");

            let db_path = app_data_dir.join("nebula.db");
            let db_conn = rusqlite::Connection::open(&db_path)
                .expect("Failed to open database");
            db::init_db(&db_conn).expect("Failed to initialize database");

            app.manage(AppState {
                db: Mutex::new(db_conn),
                sidecar: Mutex::new(SidecarManager::new()),
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 2: Verify compilation**

Run: `cd /home/pi/nebula/src-tauri && cargo check`

Note: There may be compilation issues with the `tauri::api::path` module. If `app_data_dir` is not available in Tauri 2, replace the database path initialization in `run()` with:

```rust
pub fn run() {
    let db_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("nebula")
        .join("nebula.db");

    std::fs::create_dir_all(db_path.parent().unwrap()).ok();
    let db_conn = rusqlite::Connection::open(&db_path)
        .expect("Failed to open database");

    db::init_db(&db_conn).expect("Failed to initialize database");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            db: Mutex::new(db_conn),
            sidecar: Mutex::new(SidecarManager::new()),
        })
        .invoke_handler(tauri::generate_handler![
            add_folder,
            remove_folder,
            list_folders,
            get_indexing_status,
            get_images,
            start_sidecar,
            stop_sidecar,
            sidecar_health,
            start_embedding_job,
            search_images,
        ])
        .setup(|app| {
            // Get the proper app data path from Tauri
            let app_data_dir = app.path().app_data_dir().expect("Failed to get app data dir");
            std::fs::create_dir_all(&app_data_dir).expect("Failed to create app data dir");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Fix any compilation errors until `cargo check` passes.

- [ ] **Step 3: Run all tests**

Run: `cd /home/pi/nebula/src-tauri && cargo test --lib`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: wire up all Tauri commands with state management"
```

---

### Task 8: Angular Dependencies & Tailwind Setup

**Files:**
- Modify: `package.json` (via pnpm add)
- Modify: `src/styles.css`

- [ ] **Step 1: Install Angular dependencies**

Run:
```bash
cd /home/pi/nebula
pnpm add @tauri-apps/plugin-dialog @tauri-apps/plugin-shell
pnpm add -D tailwindcss @tailwindcss/postcss
```

- [ ] **Step 2: Create PostCSS config**

Create `/home/pi/nebula/.postcssrc.json`:

```json
{
  "plugins": {
    "@tailwindcss/postcss": {}
  }
}
```

- [ ] **Step 3: Verify Tailwind is working**

Run: `cd /home/pi/nebula && pnpm build`
Expected: Build succeeds with Tailwind processing styles.css.

- [ ] **Step 4: Update styles.css with dark theme tokens**

Read the current `src/styles.css` first, then replace with dark mode theme:

```css
@import 'tailwindcss/theme.css' layer(theme);
@import 'tailwindcss/preflight.css' layer(base);
@import 'tailwindcss/utilities.css';

@layer base {
  :root {
    color-scheme: dark;
    --background: 222.2 84% 4.9%;
    --foreground: 210 40% 98%;
    --card: 222.2 84% 4.9%;
    --card-foreground: 210 40% 98%;
    --popover: 222.2 84% 4.9%;
    --popover-foreground: 210 40% 98%;
    --primary: 210 40% 98%;
    --primary-foreground: 222.2 47.4% 11.2%;
    --secondary: 217.2 32.6% 17.5%;
    --secondary-foreground: 210 40% 98%;
    --muted: 217.2 32.6% 17.5%;
    --muted-foreground: 215 20.2% 65.1%;
    --accent: 217.2 32.6% 17.5%;
    --accent-foreground: 210 40% 98%;
    --destructive: 0 62.8% 30.6%;
    --destructive-foreground: 210 40% 98%;
    --border: 217.2 32.6% 17.5%;
    --input: 217.2 32.6% 17.5%;
    --ring: 212.7 26.8% 83.9%;
    --radius: 0.5rem;

    --color-background: hsl(var(--background));
    --color-foreground: hsl(var(--foreground));
    --color-card: hsl(var(--card));
    --color-card-foreground: hsl(var(--card-foreground));
    --color-primary: hsl(var(--primary));
    --color-primary-foreground: hsl(var(--primary-foreground));
    --color-secondary: hsl(var(--secondary));
    --color-secondary-foreground: hsl(var(--secondary-foreground));
    --color-muted: hsl(var(--muted));
    --color-muted-foreground: hsl(var(--muted-foreground));
    --color-accent: hsl(var(--accent));
    --color-accent-foreground: hsl(var(--accent-foreground));
    --color-destructive: hsl(var(--destructive));
    --color-destructive-foreground: hsl(var(--destructive-foreground));
    --color-border: hsl(var(--border));
    --color-input: hsl(var(--input));
    --color-ring: hsl(var(--ring));
  }

  * {
    border-color: hsl(var(--border));
  }

  body {
    background-color: hsl(var(--background));
    color: hsl(var(--foreground));
    font-family: Inter, system-ui, -apple-system, sans-serif;
    margin: 0;
    overflow: hidden;
    height: 100vh;
  }
}
```

- [ ] **Step 5: Commit**

```bash
git add package.json pnpm-lock.yaml .postcssrc.json src/styles.css
git commit -m "chore: add Angular dependencies, Tailwind CSS, and dark theme"
```

---

### Task 9: Tauri Service

**Files:**
- Create: `src/app/services/tauri.service.ts`

- [ ] **Step 1: Create tauri.service.ts**

```typescript
import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface Folder {
  id: number;
  path: string;
  added_at: string;
}

export interface ImageRecord {
  id: number;
  folder_id: number;
  file_path: string;
  file_name: string;
  file_size: number | null;
  created_at: string | null;
  indexed_at: string;
  embedded: boolean;
}

export interface IndexingStatus {
  total: number;
  embedded: number;
}

export interface SearchResult {
  id: number;
  file_path: string;
  file_name: string;
  similarity: number;
}

export interface EmbeddingProgress {
  current: number;
  total: number;
}

@Injectable({ providedIn: 'root' })
export class TauriService {
  // Folders
  addFolder(path: string): Promise<Folder> {
    return invoke<Folder>('add_folder', { path });
  }

  removeFolder(id: number): Promise<void> {
    return invoke<void>('remove_folder', { id });
  }

  listFolders(): Promise<Folder[]> {
    return invoke<Folder[]>('list_folders');
  }

  // Images
  getIndexingStatus(): Promise<IndexingStatus> {
    return invoke<IndexingStatus>('get_indexing_status');
  }

  getImages(offset: number, limit: number): Promise<ImageRecord[]> {
    return invoke<ImageRecord[]>('get_images', { offset, limit });
  }

  // Sidecar
  startSidecar(): Promise<void> {
    return invoke<void>('start_sidecar');
  }

  stopSidecar(): Promise<void> {
    return invoke<void>('stop_sidecar');
  }

  sidecarHealth(): Promise<boolean> {
    return invoke<boolean>('sidecar_health');
  }

  // Embedding
  startEmbeddingJob(): Promise<void> {
    return invoke<void>('start_embedding_job');
  }

  // Search
  searchImages(query: string, limit: number = 20): Promise<SearchResult[]> {
    return invoke<SearchResult[]>('search_images', { query, limit });
  }

  // Events
  onEmbeddingProgress(callback: (progress: EmbeddingProgress) => void): Promise<UnlistenFn> {
    return listen<EmbeddingProgress>('embedding-progress', (event) => callback(event.payload));
  }

  onEmbeddingComplete(callback: () => void): Promise<UnlistenFn> {
    return listen('embedding-complete', () => callback());
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/app/services/tauri.service.ts
git commit -m "feat: add Tauri service wrapper for IPC commands and events"
```

---

### Task 10: Folder Manager Component

**Files:**
- Create: `src/app/components/folder-manager/folder-manager.component.ts`
- Create: `src/app/components/folder-manager/folder-manager.component.html`
- Create: `src/app/components/folder-manager/folder-manager.component.css`

- [ ] **Step 1: Create folder-manager component**

Create `src/app/components/folder-manager/folder-manager.component.ts`:

```typescript
import { Component, OnInit, OnDestroy } from '@angular/core';
import { Subject, takeUntil } from 'rxjs';
import { open } from '@tauri-apps/plugin-dialog';
import { TauriService, Folder, IndexingStatus } from '../../services/tauri.service';

@Component({
  selector: 'app-folder-manager',
  standalone: true,
  imports: [],
  templateUrl: './folder-manager.component.html',
  styleUrl: './folder-manager.component.css',
})
export class FolderManagerComponent implements OnInit, OnDestroy {
  folders: Folder[] = [];
  status: IndexingStatus | null = null;
  loading = false;
  private destroy$ = new Subject<void>();

  constructor(private tauri: TauriService) {}

  ngOnInit(): void {
    this.loadFolders();
    this.loadStatus();
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  async addFolder(): Promise<void> {
    const selected = await open({
      directory: true,
      multiple: false,
      title: 'Select a folder to index',
    });
    if (typeof selected === 'string' && selected) {
      this.loading = true;
      try {
        await this.tauri.addFolder(selected);
        await this.loadFolders();
        await this.loadStatus();
      } catch (e) {
        console.error('Failed to add folder:', e);
      } finally {
        this.loading = false;
      }
    }
  }

  async removeFolder(id: number): Promise<void> {
    try {
      await this.tauri.removeFolder(id);
      await this.loadFolders();
      await this.loadStatus();
    } catch (e) {
      console.error('Failed to remove folder:', e);
    }
  }

  async loadFolders(): Promise<void> {
    try {
      this.folders = await this.tauri.listFolders();
    } catch (e) {
      console.error('Failed to load folders:', e);
    }
  }

  async loadStatus(): Promise<void> {
    try {
      this.status = await this.tauri.getIndexingStatus();
    } catch (e) {
      console.error('Failed to load status:', e);
    }
  }
}
```

- [ ] **Step 2: Create template**

Create `src/app/components/folder-manager/folder-manager.component.html`:

```html
<div class="p-4">
  <div class="flex items-center justify-between mb-4">
    <h2 class="text-sm font-semibold uppercase tracking-wider text-muted-foreground">Folders</h2>
    <button
      class="px-3 py-1.5 text-xs font-medium rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
      (click)="addFolder()"
      [disabled]="loading"
    >
      {{ loading ? 'Scanning...' : '+ Add Folder' }}
    </button>
  </div>

  <div class="space-y-1">
    @for (folder of folders; track folder.id) {
      <div class="group flex items-center justify-between p-2 rounded-md hover:bg-accent transition-colors">
        <span class="text-sm truncate pr-2" [title]="folder.path">{{ folder.path }}</span>
        <button
          class="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive transition-all text-xs px-1"
          (click)="removeFolder(folder.id)"
          title="Remove folder"
        >
          ✕
        </button>
      }
    } @empty {
      <p class="text-xs text-muted-foreground text-center py-4">
        No folders added yet
      </p>
    }
  </div>

  @if (status) {
    <div class="mt-4 pt-4 border-t border-border">
      <p class="text-xs text-muted-foreground">
        {{ status.total }} images found
        · {{ status.embedded }} embedded
      </p>
    </div>
  }
</div>
```

- [ ] **Step 3: Create styles**

Create `src/app/components/folder-manager/folder-manager.component.css`:

```css
:host {
  display: block;
}
```

- [ ] **Step 4: Commit**

```bash
git add src/app/components/folder-manager/
git commit -m "feat: add folder manager component with add/remove UI"
```

---

### Task 11: Embedding Status Component

**Files:**
- Create: `src/app/components/embedding-status/embedding-status.component.ts`
- Create: `src/app/components/embedding-status/embedding-status.component.html`

- [ ] **Step 1: Create embedding-status component**

Create `src/app/components/embedding-status/embedding-status.component.ts`:

```typescript
import { Component, OnInit, OnDestroy } from '@angular/core';
import { UnlistenFn } from '@tauri-apps/api/event';
import { TauriService, IndexingStatus, EmbeddingProgress } from '../../services/tauri.service';

@Component({
  selector: 'app-embedding-status',
  standalone: true,
  imports: [],
  templateUrl: './embedding-status.component.html',
  styleUrl: './embedding-status.component.css',
})
export class EmbeddingStatusComponent implements OnInit, OnDestroy {
  status: IndexingStatus | null = null;
  progress: EmbeddingProgress | null = null;
  isRunning = false;
  sidecarReady = false;
  private unlistenProgress: UnlistenFn | null = null;
  private unlistenComplete: UnlistenFn | null = null;

  constructor(private tauri: TauriService) {}

  async ngOnInit(): Promise<void> {
    this.unlistenProgress = await this.tauri.onEmbeddingProgress((p) => {
      this.progress = p;
      this.isRunning = true;
    });
    this.unlistenComplete = await this.tauri.onEmbeddingComplete(() => {
      this.isRunning = false;
      this.progress = null;
      this.loadStatus();
    });
    await this.checkSidecar();
    await this.loadStatus();
  }

  ngOnDestroy(): void {
    this.unlistenProgress?.();
    this.unlistenComplete?.();
  }

  async checkSidecar(): Promise<void> {
    try {
      this.sidecarReady = await this.tauri.sidecarHealth();
    } catch {
      this.sidecarReady = false;
    }
  }

  async startSidecar(): Promise<void> {
    try {
      await this.tauri.startSidecar();
      this.sidecarReady = true;
    } catch (e) {
      console.error('Failed to start sidecar:', e);
    }
  }

  async startEmbedding(): Promise<void> {
    try {
      this.isRunning = true;
      await this.tauri.startEmbeddingJob();
    } catch (e) {
      console.error('Failed to start embedding:', e);
      this.isRunning = false;
    }
  }

  async loadStatus(): Promise<void> {
    try {
      this.status = await this.tauri.getIndexingStatus();
    } catch (e) {
      console.error('Failed to load status:', e);
    }
  }

  get progressPercent(): number {
    if (!this.progress) return 0;
    return Math.round((this.progress.current / this.progress.total) * 100);
  }
}
```

- [ ] **Step 2: Create template**

Create `src/app/components/embedding-status/embedding-status.component.html`:

```html
<div class="p-4 border-t border-border">
  <h2 class="text-sm font-semibold uppercase tracking-wider text-muted-foreground mb-3">Embeddings</h2>

  @if (!sidecarReady) {
    <button
      class="w-full px-3 py-2 text-xs font-medium rounded-md bg-accent text-accent-foreground hover:bg-accent/80 transition-colors"
      (click)="startSidecar()"
    >
      Load Model (Sidecar)
    </button>
  }

  @if (sidecarReady && status && status.embedded < status.total && !isRunning) {
    <button
      class="w-full px-3 py-2 text-xs font-medium rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
      (click)="startEmbedding()"
    >
      Generate Embeddings ({{ status.total - status.embedded }} remaining)
    </button>
  }

  @if (isRunning && progress) {
    <div>
      <div class="flex justify-between text-xs text-muted-foreground mb-1">
        <span>Embedding...</span>
        <span>{{ progress.current }} / {{ progress.total }}</span>
      </div>
      <div class="w-full h-2 rounded-full bg-secondary overflow-hidden">
        <div
          class="h-full bg-primary transition-all duration-300 rounded-full"
          [style.width.%]="progressPercent"
        ></div>
      </div>
    </div>
  }

  @if (status && status.total > 0) {
    <p class="text-xs text-muted-foreground mt-2">
      {{ status.embedded }} / {{ status.total }} images embedded
    </p>
  }
</div>
```

- [ ] **Step 3: Create styles**

Create `src/app/components/embedding-status/embedding-status.component.css`:

```css
:host {
  display: block;
}
```

- [ ] **Step 4: Commit**

```bash
git add src/app/components/embedding-status/
git commit -m "feat: add embedding status component with progress tracking"
```

---

### Task 12: Search Bar & Image Grid Components

**Files:**
- Create: `src/app/components/search-bar/search-bar.component.ts`
- Create: `src/app/components/search-bar/search-bar.component.html`
- Create: `src/app/components/image-grid/image-grid.component.ts`
- Create: `src/app/components/image-grid/image-grid.component.html`
- Create: `src/app/components/image-grid/image-grid.component.css`

- [ ] **Step 1: Create search-bar component**

Create `src/app/components/search-bar/search-bar.component.ts`:

```typescript
import { Component, output } from '@angular/core';
import { Subject } from 'rxjs';
import { debounceTime, distinctUntilChanged, takeUntil } from 'rxjs/operators';

@Component({
  selector: 'app-search-bar',
  standalone: true,
  imports: [],
  templateUrl: './search-bar.component.html',
  styleUrl: './search-bar.component.css',
})
export class SearchBarComponent implements OnDestroy {
  searchQuery = output<string>();
  private searchSubject = new Subject<string>();
  private destroy$ = new Subject<void>();

  constructor() {
    this.searchSubject
      .pipe(debounceTime(500), distinctUntilChanged(), takeUntil(this.destroy$))
      .subscribe((query) => this.searchQuery.emit(query));
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
    this.searchSubject.complete();
  }

  onInput(event: Event): void {
    const value = (event.target as HTMLInputElement).value.trim();
    if (value.length >= 2) {
      this.searchSubject.next(value);
    } else if (value.length === 0) {
      this.searchQuery.emit('');
    }
  }
}
```

Create `src/app/components/search-bar/search-bar.component.html`:

```html
<div class="relative">
  <input
    type="text"
    placeholder="Search images with natural language..."
    class="w-full px-4 py-3 rounded-lg bg-secondary text-foreground placeholder:text-muted-foreground border border-input focus:outline-none focus:ring-2 focus:ring-ring text-sm"
    (input)="onInput($event)"
  />
  <svg class="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
  </svg>
</div>
```

Create `src/app/components/search-bar/search-bar.component.css`:

```css
:host {
  display: block;
}
```

- [ ] **Step 2: Create image-grid component**

Create `src/app/components/image-grid/image-grid.component.ts`:

```typescript
import { Component, Input } from '@angular/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { SearchResult } from '../../services/tauri.service';

@Component({
  selector: 'app-image-grid',
  standalone: true,
  imports: [],
  templateUrl: './image-grid.component.html',
  styleUrl: './image-grid.component.css',
})
export class ImageGridComponent {
  @Input() results: SearchResult[] = [];
  @Input() loading = false;
  @Input() query = '';

  getAssetUrl(filePath: string): string {
    return convertFileSrc(filePath);
  }

  getSimilarityLabel(score: number): string {
    return `${Math.round(score * 100)}%`;
  }

  async openImage(filePath: string): Promise<void> {
    try {
      const { open } = await import('@tauri-apps/plugin-opener');
      await open(filePath);
    } catch (e) {
      console.error('Failed to open image:', e);
    }
  }
}
```

Create `src/app/components/image-grid/image-grid.component.html`:

```html
@if (loading) {
  <div class="flex items-center justify-center py-20">
    <div class="text-muted-foreground text-sm">Searching...</div>
  </div>
}

@else if (query && results.length === 0) {
  <div class="flex items-center justify-center py-20">
    <div class="text-center">
      <p class="text-muted-foreground text-sm">No images found for "{{ query }}"</p>
    </div>
  </div>
}

@else if (results.length > 0) {
  <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-3 p-4">
    @for (result of results; track result.id) {
      <div
        class="group relative rounded-lg overflow-hidden bg-secondary cursor-pointer hover:ring-2 hover:ring-ring transition-all"
        (click)="openImage(result.file_path)"
      >
        <img
          [src]="getAssetUrl(result.file_path)"
          [alt]="result.file_name"
          class="w-full aspect-square object-cover"
          loading="lazy"
        />
        <div class="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/70 to-transparent p-2 opacity-0 group-hover:opacity-100 transition-opacity">
          <p class="text-xs text-white truncate">{{ result.file_name }}</p>
          <span class="text-[10px] text-white/70">{{ getSimilarityLabel(result.similarity) }}</span>
        </div>
      </div>
    }
  </div>
}

@else {
  <div class="flex items-center justify-center py-20">
    <div class="text-center">
      <p class="text-muted-foreground text-sm">Search for images using natural language</p>
      <p class="text-muted-foreground/70 text-xs mt-1">e.g. "sunset at the beach", "kids playing soccer"</p>
    </div>
  </div>
}
```

Create `src/app/components/image-grid/image-grid.component.css`:

```css
:host {
  display: block;
  overflow-y: auto;
  height: 100%;
}
```

- [ ] **Step 3: Commit**

```bash
git add src/app/components/search-bar/ src/app/components/image-grid/
git commit -m "feat: add search bar and image grid components"
```

---

### Task 13: Main App Layout

**Files:**
- Modify: `src/app/app.component.ts`
- Modify: `src/app/app.component.html`
- Modify: `src/app/app.component.css`

- [ ] **Step 1: Rewrite app.component.ts**

Replace the entire contents of `src/app/app.component.ts`:

```typescript
import { Component, OnInit } from '@angular/core';
import { FolderManagerComponent } from './components/folder-manager/folder-manager.component';
import { EmbeddingStatusComponent } from './components/embedding-status/embedding-status.component';
import { SearchBarComponent } from './components/search-bar/search-bar.component';
import { ImageGridComponent } from './components/image-grid/image-grid.component';
import { TauriService, SearchResult } from './services/tauri.service';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [
    FolderManagerComponent,
    EmbeddingStatusComponent,
    SearchBarComponent,
    ImageGridComponent,
  ],
  templateUrl: './app.component.html',
  styleUrl: './app.component.css',
})
export class AppComponent implements OnInit {
  searchResults: SearchResult[] = [];
  searchLoading = false;
  currentQuery = '';

  constructor(private tauri: TauriService) {}

  ngOnInit(): void {
    // Auto-start sidecar on app load
    this.tauri.startSidecar().catch(() => {});
  }

  async onSearch(query: string): Promise<void> {
    if (!query) {
      this.searchResults = [];
      this.currentQuery = '';
      return;
    }
    this.currentQuery = query;
    this.searchLoading = true;
    try {
      this.searchResults = await this.tauri.searchImages(query, 50);
    } catch (e) {
      console.error('Search failed:', e);
      this.searchResults = [];
    } finally {
      this.searchLoading = false;
    }
  }
}
```

- [ ] **Step 2: Rewrite app.component.html**

Replace the entire contents of `src/app/app.component.html`:

```html
<div class="flex h-screen">
  <!-- Sidebar -->
  <aside class="w-64 flex-shrink-0 border-r border-border bg-card flex flex-col">
    <div class="p-4 border-b border-border">
      <h1 class="text-lg font-bold text-foreground">Nebula</h1>
      <p class="text-xs text-muted-foreground">Semantic Image Search</p>
    </div>
    <div class="flex-1 overflow-y-auto">
      <app-folder-manager />
    </div>
    <app-embedding-status />
  </aside>

  <!-- Main Content -->
  <main class="flex-1 flex flex-col min-w-0">
    <div class="p-4 border-b border-border">
      <app-search-bar (searchQuery)="onSearch($event)" />
    </div>
    <div class="flex-1 overflow-y-auto">
      <app-image-grid
        [results]="searchResults"
        [loading]="searchLoading"
        [query]="currentQuery"
      />
    </div>
  </main>
</div>
```

- [ ] **Step 3: Rewrite app.component.css**

Replace the entire contents of `src/app/app.component.css`:

```css
:host {
  display: block;
  height: 100vh;
  overflow: hidden;
}
```

- [ ] **Step 4: Commit**

```bash
git add src/app/app.component.ts src/app/app.component.html src/app/app.component.css
git commit -m "feat: main layout with sidebar and content area"
```

---

### Task 14: Integration Build & Fix

**Files:**
- May modify any file from previous tasks

- [ ] **Step 1: Build the Angular frontend**

Run: `cd /home/pi/nebula && pnpm build`
Fix any compilation errors (missing imports, type errors, template issues).

- [ ] **Step 2: Build the Tauri app**

Run: `cd /home/pi/nebula && pnpm tauri build --debug`
Fix any Rust compilation errors.

- [ ] **Step 3: Fix any remaining issues**

Run `cargo check` and `pnpm build` until both pass cleanly.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "fix: resolve integration build issues and database path"
```

---

### Task 15: Manual Integration Test

- [ ] **Step 1: Set up Python virtual environment**

Run:
```bash
cd /home/pi/nebula/sidecar
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

- [ ] **Step 2: Run the Tauri dev server**

Run:
```bash
cd /home/pi/nebula
pnpm tauri dev
```

- [ ] **Step 3: Verify folder management**

1. Click "Add Folder" in the sidebar
2. Select a directory with images
3. Verify the folder appears in the list
4. Verify the image count updates

- [ ] **Step 4: Verify embedding generation**

1. Click "Load Model (Sidecar)"
2. Wait for the model to load
3. Click "Generate Embeddings"
4. Verify the progress bar updates

- [ ] **Step 5: Verify search**

1. Type a natural language query in the search bar
2. Verify results appear in the grid
3. Click an image to open it

- [ ] **Step 6: Final commit**

```bash
git add -A
git commit -m "chore: integration testing complete, MVP ready"
```
