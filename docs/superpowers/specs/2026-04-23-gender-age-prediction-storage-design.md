# Gender/Age Prediction Storage & Display

**Date:** 2026-04-23
**Status:** Approved

## Problem

The `face_id` crate's `FaceAnalyzer::analyze()` always runs gender/age estimation as part of its pipeline (detection -> embedding -> gender/age). The gender/age model (`genderage.onnx`) is downloaded, loaded into an ONNX session, and run on every face crop — but Nebula discards the results in `vision_engine.rs:75`:

```rust
faces.into_iter().map(|f| (f.detection.bbox, f.embedding)).collect()
```

This wastes download bandwidth, disk space, and inference time. Since the `FaceAnalyzer` API requires the gender/age model as a mandatory parameter, we cannot remove it. Instead, we should capture the data and make it useful.

## Decision

Store predicted gender and age on the `faces` table and expose aggregated values on the subject detail page. No changes to the people grid cards.

## Design

### 1. Database Schema

Add two nullable columns to the `faces` table via `ALTER TABLE` in the migration block in `db.rs`:

```sql
ALTER TABLE faces ADD COLUMN gender TEXT;
ALTER TABLE faces ADD COLUMN age INTEGER;
```

Existing faces remain `NULL`. Only new faces processed after this change will have gender/age populated.

### 2. Capture in Pipeline

**`vision_engine.rs`** — Change `analyze_faces` / `analyze_faces_full` to return gender and age alongside bbox and embedding:

- Current return: `Vec<(BoundingBox, Vec<f32>)>`
- New return: `Vec<(BoundingBox, Vec<f32>, face_id::gender_age::Gender, u8)>`

**`embedder.rs`** — In `process_subject_one` (line 165), extract gender/age from the result tuple and pass them to `db::insert_face`.

**`db.rs`** — Update `insert_face` to accept and store `gender: Option<&str>` and `age: Option<u8>`.

### 3. Expose on Subject Detail

Add two fields to the `SubjectDetail` struct:

```rust
pub predicted_gender: Option<String>,
pub predicted_age: Option<u8>,
```

In `get_subject_detail_with_counts`, add a subquery that aggregates from the subject's faces:

```sql
SELECT
  (SELECT gender FROM faces WHERE subject_id = s.id AND gender IS NOT NULL
   GROUP BY gender ORDER BY COUNT(*) DESC LIMIT 1) AS predicted_gender,
  (SELECT ROUND(AVG(age)) FROM faces WHERE subject_id = s.id AND age IS NOT NULL) AS predicted_age
```

- **Gender:** majority vote (most common gender among the subject's faces).
- **Age:** average age rounded to nearest integer.

Both are `NULL` when no faces with gender/age data are assigned to the subject.

### 4. Frontend Display

**TypeScript model** (`models.ts`): Add `predicted_gender` and `predicted_age` to `SubjectDetail`.

**Subject detail template** (`subject-detail.component.html`): Below the existing "X photos / Y faces" line, add a subtle line only when data exists:

```html
<div class="text-sm text-muted-foreground flex items-center gap-3">
  <span>{{ detail()?.photo_count || 0 }} photos</span>
  <span class="w-1 h-1 rounded-full bg-border"></span>
  <span>{{ detail()?.face_count || 0 }} faces</span>
  @if (detail()?.predicted_gender) {
    <span class="w-1 h-1 rounded-full bg-border"></span>
    <span>Predicted gender: {{ detail()?.predicted_gender }}</span>
  }
  @if (detail()?.predicted_age != null) {
    <span class="w-1 h-1 rounded-full bg-border"></span>
    <span>Predicted age: ~{{ detail()?.predicted_age }}</span>
  }
</div>
```

Labels say "Predicted gender" and "Predicted age" so users understand these are estimates.

**No changes to the people grid** (`people-view.component.html`).

## Scope

- Backend: `db.rs`, `embedder.rs`, `vision_engine.rs`, `models/entities.rs`
- Frontend: `models/models.ts`, `subject-detail.component.html`
- No model changes, no new downloads, no people grid changes
