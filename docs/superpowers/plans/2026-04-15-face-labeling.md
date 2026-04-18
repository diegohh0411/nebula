# Face Labeling & Subject Grouping Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement automated face detection, individual face embeddings via Gemini API, and automatic subject clustering/labeling.

**Architecture:** Update the existing `embedding_worker` to perform a multi-stage pipeline (Global Embed -> Face Detect -> Face Embed -> Cluster). Faces are stored with bounding boxes and embeddings, linked to a `subjects` table.

**Tech Stack:** Rust (Tauri), `rust-faces` (ONNX), Gemini API (Embeddings), SQLite (sqlx), Angular (Signals).

---

### Task 1: Database Schema & Rust Models

**Files:**
- Modify: `src-tauri/src/db.rs` (MIGRATIONS and new functions)
- Modify: `src-tauri/src/models.rs` (Face and Subject structs)

- [ ] **Step 1: Update Database Migrations**
  Add the `subjects` and `faces` tables to the `MIGRATIONS` constant in `src-tauri/src/db.rs`.

- [ ] **Step 2: Define Rust Models**
  Add `Subject` and `Face` structs to `src-tauri/src/models.rs`.

- [ ] **Step 3: Implement Database Accessors**
  Add functions to `src-tauri/src/db.rs` for:
  - `insert_subject(pool, name, type)`
  - `insert_face(pool, image_id, subject_id, bbox, embedding)`
  - `list_all_subjects(pool)`
  - `list_faces_for_subject(pool, subject_id)`
  - `get_subject_embeddings(pool)` (to compare new faces against)

- [ ] **Step 4: Commit**
  `git add src-tauri/src/db.rs src-tauri/src/models.rs && git commit -m "feat: add faces and subjects database schema"`

---

### Task 2: Face Detection Integration

**Files:**
- Modify: `src-tauri/Cargo.toml` (Add `rust-faces`)
- Create: `src-tauri/src/face_detector.rs` (Wrapper for rust-faces)
- Modify: `src-tauri/src/lib.rs` (Register module)

- [ ] **Step 1: Add Dependencies**
  Add `rust-faces = "0.1"` (or latest compatible) to `src-tauri/Cargo.toml`.

- [ ] **Step 2: Implement Local Detector**
  Create `src-tauri/src/face_detector.rs` that loads the ONNX model and provides a function to detect faces from an image path or buffer, returning bounding boxes.

- [ ] **Step 3: Write Test for Detection**
  Create a test in `face_detector.rs` using a sample image to verify bounding boxes are detected.

- [ ] **Step 4: Commit**
  `git add src-tauri/Cargo.toml src-tauri/src/face_detector.rs && git commit -m "feat: integrate rust-faces for local face detection"`

---

### Task 3: Processing Pipeline & Clustering

**Files:**
- Modify: `src-tauri/src/embedder.rs` (Update `process_one` and `run_embedding_worker`)

- [ ] **Step 1: Update Image Cropping Utility**
  Add a helper function to `embedder.rs` (or a new `utils.rs`) to crop a face from an image given a bounding box.

- [ ] **Step 2: Refactor Embedding Logic**
  Update `process_one` to:
  1. Detect faces locally.
  2. For each face, crop and call `embed_image`.
  3. Compare face embedding with existing subject embeddings (Cosine Similarity).
  4. Assign to existing subject or create new "Unnamed" subject.

- [ ] **Step 3: Implement Cosine Similarity Helper**
  Add a `cosine_similarity` function to `embedder.rs` for clustering.

- [ ] **Step 4: Commit**
  `git add src-tauri/src/embedder.rs && git commit -m "feat: update embedding worker with face detection and clustering"`

---

### Task 4: Frontend People View

**Files:**
- Create: `src/app/components/people-view/` (New component)
- Modify: `src/app/models/models.ts` (Add Subject and Face types)
- Modify: `src/app/services/photo.service.ts` (Add face-related methods)
- Modify: `src/app/app.routes.ts` (Add route for People)

- [ ] **Step 1: Update Angular Models**
  Add `Subject` and `Face` interfaces to `src/app/models/models.ts`.

- [ ] **Step 2: Implement PhotoService Methods**
  Add `loadSubjects()`, `nameSubject(id, name)`, and `removeFaceFromSubject(faceId)` to `PhotoService`.

- [ ] **Step 3: Create People View Component**
  Build a grid view showing subject thumbnails and names.

- [ ] **Step 4: Integrate into Sidebar/Routes**
  Add a "People" link to the sidebar and register the route.

- [ ] **Step 5: Commit**
  `git add src/app/ && git commit -m "feat: add people view and management UI"`
