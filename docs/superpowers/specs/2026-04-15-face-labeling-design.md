# Face Labeling & Subject Grouping Design

Implement an automated face recognition and labeling system for Nebula, allowing users to discover, name, and search for people and other subjects (pets, mascots) across their photo collection.

## Architecture

### Processing Pipeline
The system uses an **Integrated Worker** approach, enhancing the existing `embedding_worker` in Tauri to handle face-specific tasks alongside full-image embeddings.

1.  **Image Discovery:** When an image is added, it's queued for processing.
2.  **Global Embedding:** The full image is sent to the Gemini API for general semantic search (existing behavior).
3.  **Face Detection:** `rust-faces` (using Ort/ONNX) runs locally to detect faces and their bounding boxes.
4.  **Face Embeddings:** Each detected face is cropped in memory and sent to the Gemini API for a specialized face embedding.
5.  **Clustering:** New face embeddings are compared against existing `subjects` using cosine similarity.
    *   **Threshold:** A conservative threshold (~0.85) is used to avoid false positives.
    *   **Assignment:** Faces matching an existing subject are assigned automatically. Unmatched faces create a new "Unnamed Subject".
6.  **Persistence:** Bounding boxes, embeddings, and subject associations are stored in the SQLite database.

### Data Model
New tables in `nebula.db`:

```sql
CREATE TABLE IF NOT EXISTS subjects (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    name              TEXT, -- NULL means "Unnamed"
    thumbnail_face_id INTEGER, -- FK to faces(id)
    type              TEXT NOT NULL DEFAULT 'person', -- 'person', 'pet', etc.
    added_at          INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS faces (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    image_id    INTEGER NOT NULL, -- No CASCADE to preserve "training data"
    subject_id  INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    bbox_x      REAL NOT NULL, -- Relative (0.0 - 1.0)
    bbox_y      REAL NOT NULL,
    bbox_w      REAL NOT NULL,
    bbox_h      REAL NOT NULL,
    embedding   BLOB, -- Gemini API face embedding (f32 vector)
    added_at    INTEGER NOT NULL
);
```

**Note on Deletion:** `image_id` in the `faces` table does **not** use `ON DELETE CASCADE`. This preserves face embeddings and metadata even if an image is removed from the library, allowing for future re-clustering or heuristic improvements using historical data.

## User Experience

### Discovery & Labeling
*   **People View:** A new sidebar section showing circles for each subject.
*   **Immediate Visibility:** All "Unnamed" subjects are shown immediately in a "Discover" or "Unnamed" section, allowing users to label them as they appear.
*   **Naming & Merging:** When a user names an unnamed subject with an existing name, Nebula offers to merge the clusters.

### Refinement
*   **Manual Correction:** Users can navigate to a subject's view and manually remove photos that were misidentified.
*   **Subject Types:** Future support for non-human subjects (pets, mascots) via the `type` field.

## Technical Details

### Dependencies
*   `rust-faces`: Local face detection (ONNX runtime via `ort`).
*   Gemini API: Used for generating high-quality embeddings for face crops.
*   `sqlx`: Database migrations and queries.

### Performance
*   Face detection runs locally to avoid unnecessary API costs for images without faces.
*   Parallel processing in the `embedding_worker` ensures face discovery doesn't block the main UI.
*   In-memory cropping avoids writing temporary files to disk.

## Success Criteria
*   Faces are correctly detected in newly added images.
*   Similar faces are grouped together with a high degree of accuracy.
*   Users can name groups and see those names reflected across the app.
*   System handles 10k+ faces with performant clustering lookups.
