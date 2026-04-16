# Subject Merge Suggestions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add cross-pair linking merge suggestions, name-based merge detection, and named-first subject sorting.

**Architecture:** After each HDBSCAN recluster, a new pass computes cross-pair cosine similarities between all subject face embeddings to detect likely duplicates. Suggestions are stored in a new `merge_suggestions` table and exposed via Tauri commands. The frontend shows suggestions in the People view and Subject Detail page, and detects name conflicts when renaming.

**Tech Stack:** Rust (sqlx, cosine similarity from existing embedder.rs), TypeScript/Angular (signals, Tauri invoke)

---

### Task 1: Named-first subject sorting (bug fix)

**Files:**
- Modify: `src-tauri/src/db.rs:447`

- [ ] **Step 1: Update the `list_all_subjects` query to sort named subjects first**

In `src-tauri/src/db.rs`, change the `list_all_subjects` function's query from:

```rust
let rows = sqlx::query("SELECT id, name, thumbnail_face_id, type, added_at FROM subjects ORDER BY added_at DESC")
```

to:

```rust
let rows = sqlx::query("SELECT id, name, thumbnail_face_id, type, added_at FROM subjects ORDER BY CASE WHEN name IS NOT NULL THEN 0 ELSE 1 END, added_at DESC")
```

- [ ] **Step 2: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "fix: sort named subjects before unnamed in People view"
```

---

### Task 2: Add `merge_suggestions` table migration

**Files:**
- Modify: `src-tauri/src/db.rs` (MIGRATIONS constant)

- [ ] **Step 1: Add the merge_suggestions table to the MIGRATIONS string**

In `src-tauri/src/db.rs`, append the following to the `MIGRATIONS` constant (after the existing `embedding_cache` table and index):

```sql
CREATE TABLE IF NOT EXISTS merge_suggestions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    cross_match_count INTEGER NOT NULL,
    total_pairs INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_merge_pair ON merge_suggestions(
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END
);
```

Note: SQLite doesn't support LEAST/GREATEST in index expressions, so we use CASE WHEN for direction-independent uniqueness.

- [ ] **Step 2: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add merge_suggestions table migration"
```

---

### Task 3: Add new model types for merge suggestions

**Files:**
- Modify: `src-tauri/src/models.rs`
- Modify: `src/app/models/models.ts`

- [ ] **Step 1: Add Rust model types**

In `src-tauri/src/models.rs`, add after the `SubjectDetail` struct:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MergeSuggestion {
    pub id: i64,
    pub subject_a: Subject,
    pub subject_b: Subject,
    pub cross_match_count: i64,
    pub total_pairs: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NameSubjectResult {
    pub duplicate_subject_id: Option<i64>,
}
```

- [ ] **Step 2: Add TypeScript model types**

In `src/app/models/models.ts`, add after the `SubjectDetail` interface:

```typescript
export interface MergeSuggestion {
  id: number;
  subject_a: Subject;
  subject_b: Subject;
  cross_match_count: number;
  total_pairs: number;
}

export interface NameSubjectResult {
  duplicate_subject_id: number | null;
}
```

- [ ] **Step 3: Verify builds compile**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles (may have unused warnings, that's fine)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/models.rs src/app/models/models.ts
git commit -m "feat: add MergeSuggestion and NameSubjectResult models"
```

---

### Task 4: Add DB helper functions for merge suggestions

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add `clear_merge_suggestions` function**

Add at the end of `db.rs` (before the closing, after `delete_stale_cache_entries`):

```rust
pub async fn clear_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    sqlx::query("DELETE FROM merge_suggestions")
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 2: Add `insert_merge_suggestion` function**

```rust
pub async fn insert_merge_suggestion(
    pool: &SqlitePool,
    subject_id_a: i64,
    subject_id_b: i64,
    cross_match_count: i64,
    total_pairs: i64,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let (lo, hi) = if subject_id_a < subject_id_b {
        (subject_id_a, subject_id_b)
    } else {
        (subject_id_b, subject_id_a)
    };
    sqlx::query(
        "INSERT OR IGNORE INTO merge_suggestions (subject_id_a, subject_id_b, cross_match_count, total_pairs, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(lo)
    .bind(hi)
    .bind(cross_match_count)
    .bind(total_pairs)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 3: Add `get_merge_suggestions` function**

```rust
pub async fn get_merge_suggestions(pool: &SqlitePool) -> Result<Vec<crate::models::MergeSuggestion>> {
    let rows = sqlx::query(
        r#"SELECT ms.id, ms.cross_match_count, ms.total_pairs,
                  sa.id as sa_id, sa.name as sa_name, sa.thumbnail_face_id as sa_thumbnail_face_id, sa.type as sa_type, sa.added_at as sa_added_at,
                  sb.id as sb_id, sb.name as sb_name, sb.thumbnail_face_id as sb_thumbnail_face_id, sb.type as sb_type, sb.added_at as sb_added_at
           FROM merge_suggestions ms
           JOIN subjects sa ON ms.subject_id_a = sa.id
           JOIN subjects sb ON ms.subject_id_b = sb.id"#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| crate::models::MergeSuggestion {
            id: r.get("id"),
            subject_a: crate::models::Subject {
                id: r.get("sa_id"),
                name: r.get("sa_name"),
                thumbnail_face_id: r.get("sa_thumbnail_face_id"),
                subject_type: r.get("sa_type"),
                added_at: r.get("sa_added_at"),
            },
            subject_b: crate::models::Subject {
                id: r.get("sb_id"),
                name: r.get("sb_name"),
                thumbnail_face_id: r.get("sb_thumbnail_face_id"),
                subject_type: r.get("sb_type"),
                added_at: r.get("sb_added_at"),
            },
            cross_match_count: r.get("cross_match_count"),
            total_pairs: r.get("total_pairs"),
        })
        .collect())
}
```

- [ ] **Step 4: Add `merge_subjects` function**

```rust
pub async fn merge_subjects(pool: &SqlitePool, target_id: i64, source_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = ? WHERE subject_id = ?")
        .bind(target_id)
        .bind(source_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM merge_suggestions WHERE subject_id_a = ? OR subject_id_b = ? OR subject_id_a = ? OR subject_id_b = ?")
        .bind(target_id)
        .bind(target_id)
        .bind(source_id)
        .bind(source_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM subjects WHERE id = ?")
        .bind(source_id)
        .execute(pool)
        .await?;

    let _ = auto_assign_missing_thumbnails(pool).await;
    Ok(())
}
```

- [ ] **Step 5: Add `dismiss_merge_suggestion` function**

```rust
pub async fn dismiss_merge_suggestion(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM merge_suggestions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 6: Add `find_subject_by_name` function**

```rust
pub async fn find_subject_by_name(pool: &SqlitePool, name: &str, exclude_id: i64) -> Result<Option<Subject>> {
    let row = sqlx::query(
        "SELECT id, name, thumbnail_face_id, type, added_at FROM subjects WHERE name = ? COLLATE NOCASE AND id != ? LIMIT 1",
    )
    .bind(name)
    .bind(exclude_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Subject {
        id: r.get("id"),
        name: r.get("name"),
        thumbnail_face_id: r.get("thumbnail_face_id"),
        subject_type: r.get("type"),
        added_at: r.get("added_at"),
    }))
}
```

- [ ] **Step 7: Add `get_faces_by_subject` function** (needed by the cross-pair algorithm to get embeddings per subject)

```rust
pub async fn get_faces_by_subject(pool: &SqlitePool, subject_id: i64) -> Result<Vec<(i64, Vec<u8>)>> {
    let rows = sqlx::query(
        "SELECT id, embedding FROM faces WHERE subject_id = ? AND embedding IS NOT NULL",
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let id: i64 = r.get("id");
            let emb: Option<Vec<u8>> = r.get("embedding");
            emb.map(|e| (id, e))
        })
        .collect())
}
```

- [ ] **Step 8: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles with no errors

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add DB helpers for merge suggestions, merge, dismiss, name lookup"
```

---

### Task 5: Implement cross-pair linking algorithm

**Files:**
- Modify: `src-tauri/src/clustering.rs`

- [ ] **Step 1: Add the `find_merge_suggestions` function**

In `src-tauri/src/clustering.rs`, add a new function after `recluster_all`:

```rust
const MERGE_SIMILARITY_THRESHOLD: f32 = 0.35;
const MERGE_MIN_CROSS_MATCHES: i64 = 2;
const MERGE_MIN_CROSS_RATIO: f32 = 0.20;

pub async fn find_merge_suggestions(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    // TODO(perf): Throttle this to run at most once every 12-24 hours rather than
    // after every recluster batch. For now it runs every time since the dataset is
    // small, but as face count grows this O(n*m) per subject pair will get expensive.
    // Consider a `last_merge_scan_at` timestamp in the DB or a dedicated periodic task.

    let subjects = crate::db::list_all_subjects(pool).await?;

    let subject_embeddings: Vec<(i64, Vec<Vec<f32>>)> = {
        let mut result = Vec::new();
        for subject in &subjects {
            let faces = crate::db::get_faces_by_subject(pool, subject.id).await?;
            let embeddings: Vec<Vec<f32>> = faces
                .into_iter()
                .filter_map(|(_, blob)| crate::embedder::bytes_to_f32_vec(&blob).ok())
                .collect();
            if !embeddings.is_empty() {
                result.push((subject.id, embeddings));
            }
        }
        result
    };

    crate::db::clear_merge_suggestions(pool).await?;

    for i in 0..subject_embeddings.len() {
        for j in (i + 1)..subject_embeddings.len() {
            let (_, emb_a) = &subject_embeddings[i];
            let (id_b, emb_b) = &subject_embeddings[j];

            let total_pairs = (emb_a.len() * emb_b.len()) as i64;
            let mut cross_match_count: i64 = 0;

            for a_face in emb_a.iter() {
                for b_face in emb_b.iter() {
                    let sim = crate::embedder::cosine_similarity(a_face, b_face);
                    if sim > MERGE_SIMILARITY_THRESHOLD {
                        cross_match_count += 1;
                    }
                }
            }

            let ratio = if total_pairs > 0 {
                cross_match_count as f32 / total_pairs as f32
            } else {
                0.0
            };

            if cross_match_count >= MERGE_MIN_CROSS_MATCHES && ratio >= MERGE_MIN_CROSS_RATIO {
                crate::db::insert_merge_suggestion(
                    pool,
                    subject_embeddings[i].0,
                    *id_b,
                    cross_match_count,
                    total_pairs,
                )
                .await?;
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Call `find_merge_suggestions` from `recluster_all`**

In `recluster_all`, add the call after `auto_assign_missing_thumbnails` (around line 89), before the `Ok(ReclusterResult {` return:

```rust
let _ = auto_assign_missing_thumbnails(pool).await;

let _ = find_merge_suggestions(pool).await;

Ok(ReclusterResult {
```

- [ ] **Step 3: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat: implement cross-pair linking merge suggestion algorithm"
```

---

### Task 6: Add Tauri commands for merge suggestions + modify `name_subject`

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `get_merge_suggestions` command**

In `src-tauri/src/commands.rs`, add after the `recluster_faces` function:

```rust
#[tauri::command]
pub async fn get_merge_suggestions(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::models::MergeSuggestion>, String> {
    db::get_merge_suggestions(&state.pool)
        .await
        .map_err(map_err)
}
```

- [ ] **Step 2: Add `merge_subjects_cmd` command**

```rust
#[tauri::command]
pub async fn merge_subjects(
    target_id: i64,
    source_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::merge_subjects(&state.pool, target_id, source_id)
        .await
        .map_err(map_err)
}
```

- [ ] **Step 3: Add `dismiss_merge_suggestion_cmd` command**

```rust
#[tauri::command]
pub async fn dismiss_merge_suggestion(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::dismiss_merge_suggestion(&state.pool, id)
        .await
        .map_err(map_err)
}
```

- [ ] **Step 4: Modify `name_subject` to return duplicate detection**

Replace the existing `name_subject` function in `commands.rs`:

```rust
#[tauri::command]
pub async fn name_subject(
    id: i64,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::NameSubjectResult, String> {
    let pool = &state.pool;

    let duplicate_subject_id = if let Some(ref n) = name {
        let trimmed = n.trim();
        if !trimmed.is_empty() {
            db::find_subject_by_name(pool, trimmed, id)
                .await
                .map_err(map_err)?
                .map(|s| s.id)
        } else {
            None
        }
    } else {
        None
    };

    db::update_subject_name(pool, id, name.as_deref())
        .await
        .map_err(map_err)?;

    Ok(crate::models::NameSubjectResult {
        duplicate_subject_id,
    })
}
```

- [ ] **Step 5: Register new commands in `lib.rs`**

In `src-tauri/src/lib.rs`, add the three new commands to the `invoke_handler` array. Find the existing `recluster_faces,` line and add after it:

```rust
            commands::get_merge_suggestions,
            commands::merge_subjects,
            commands::dismiss_merge_suggestion,
```

- [ ] **Step 6: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add Tauri commands for merge suggestions and duplicate name detection"
```

---

### Task 7: Add PhotoService methods and update frontend models

**Files:**
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Update the import to include `MergeSuggestion` and `NameSubjectResult`**

In `photo.service.ts`, change the import line:

```typescript
import {
  DayGroup,
  EmbedStatus,
  Folder,
  Image,
  SearchResult,
  VirtualRow,
  Subject,
  Face,
} from '../models/models';
```

to:

```typescript
import {
  DayGroup,
  EmbedStatus,
  Folder,
  Image,
  SearchResult,
  VirtualRow,
  Subject,
  Face,
  MergeSuggestion,
  NameSubjectResult,
} from '../models/models';
```

- [ ] **Step 2: Update `nameSubject` to return `NameSubjectResult`**

Change the `nameSubject` method from:

```typescript
async nameSubject(id: number, name: string | null): Promise<void> {
    await invoke('name_subject', { id, name });
    await this.loadSubjects();
}
```

to:

```typescript
async nameSubject(id: number, name: string | null): Promise<NameSubjectResult> {
    const result = await invoke<NameSubjectResult>('name_subject', { id, name });
    await this.loadSubjects();
    return result;
}
```

- [ ] **Step 3: Add merge suggestion methods**

Add these methods after `reclusterFaces`:

```typescript
async getMergeSuggestions(): Promise<MergeSuggestion[]> {
    return await invoke<MergeSuggestion[]>('get_merge_suggestions');
}

async mergeSubjects(targetId: number, sourceId: number): Promise<void> {
    await invoke('merge_subjects', { targetId, sourceId });
    await this.loadSubjects();
}

async dismissMergeSuggestion(id: number): Promise<void> {
    await invoke('dismiss_merge_suggestion', { id });
}
```

- [ ] **Step 4: Verify the Angular build**

Run: `cd /home/pi/nebula && npx ng build 2>&1 | tail -5`
Expected: Build succeeds (may have warnings about unused imports, that's fine)

- [ ] **Step 5: Commit**

```bash
git add src/app/services/photo.service.ts
git commit -m "feat: add PhotoService methods for merge suggestions"
```

---

### Task 8: Add "Possible Duplicates" section to People View

**Files:**
- Modify: `src/app/components/people-view/people-view.component.ts`
- Modify: `src/app/components/people-view/people-view.component.html`
- Modify: `src/app/components/people-view/people-view.component.css`

- [ ] **Step 1: Update the component TypeScript**

Replace the contents of `people-view.component.ts` with:

```typescript
import { Component, inject, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';
import { MergeSuggestion, Subject } from '../../models/models';
import { RouterLink } from '@angular/router';

@Component({
  selector: 'app-people-view',
  standalone: true,
  imports: [CommonModule, RouterLink],
  templateUrl: './people-view.component.html',
  styleUrl: './people-view.component.css'
})
export class PeopleViewComponent implements OnInit {
  protected photoService = inject(PhotoService);
  protected faceCropUrls = signal<Record<number, string>>({});
  protected reclustering = signal(false);
  protected mergeSuggestions = signal<MergeSuggestion[]>([]);
  protected suggestionCropUrls = signal<Record<number, string>>({});

  async ngOnInit() {
    await this.photoService.loadSubjects();
    void this.loadMergeSuggestions();
    void this.loadThumbnails();
  }

  private async loadMergeSuggestions() {
    try {
      const suggestions = await this.photoService.getMergeSuggestions();
      this.mergeSuggestions.set(suggestions);
      void this.loadSuggestionCrops(suggestions);
    } catch (e) {
      console.error('Failed to load merge suggestions', e);
    }
  }

  private async loadSuggestionCrops(suggestions: MergeSuggestion[]) {
    const ids = new Set<number>();
    for (const s of suggestions) {
      if (s.subject_a.thumbnail_face_id) ids.add(s.subject_a.thumbnail_face_id);
      if (s.subject_b.thumbnail_face_id) ids.add(s.subject_b.thumbnail_face_id);
    }
    const urls: Record<number, string> = {};
    await Promise.all(
      [...ids].map(async (faceId) => {
        try {
          const path = await this.photoService.getFaceCrop(faceId);
          const url = this.photoService.thumbnailUrl(path);
          if (url) urls[faceId] = url;
        } catch {}
      })
    );
    this.suggestionCropUrls.set(urls);
  }

  private async loadThumbnails() {
    const subjects = this.photoService.subjects();
    const urls: Record<number, string> = {};

    await Promise.all(subjects.map(async (s) => {
      if (s.thumbnail_face_id) {
        try {
          const path = await this.photoService.getFaceCrop(s.thumbnail_face_id);
          const url = this.photoService.thumbnailUrl(path);
          if (url) urls[s.id] = url;
        } catch (e) {
          console.error(`Failed to load thumbnail for subject ${s.id}`, e);
        }
      }
    }));

    this.faceCropUrls.set(urls);
  }

  async recluster() {
    this.reclustering.set(true);
    try {
      const result = await this.photoService.reclusterFaces();
      console.log('Recluster result:', result);
      await this.photoService.loadSubjects();
      await Promise.all([this.loadThumbnails(), this.loadMergeSuggestions()]);
    } catch (e) {
      console.error('Recluster failed', e);
    } finally {
      this.reclustering.set(false);
    }
  }

  async merge(suggestion: MergeSuggestion) {
    try {
      await this.photoService.mergeSubjects(suggestion.subject_a.id, suggestion.subject_b.id);
      await Promise.all([this.loadThumbnails(), this.loadMergeSuggestions()]);
    } catch (e) {
      console.error('Merge failed', e);
    }
  }

  async dismiss(suggestion: MergeSuggestion) {
    try {
      await this.photoService.dismissMergeSuggestion(suggestion.id);
      this.mergeSuggestions.update((list) => list.filter((s) => s.id !== suggestion.id));
    } catch (e) {
      console.error('Dismiss failed', e);
    }
  }

  protected getThumbUrl(subject: Subject): string | null {
    if (!subject.thumbnail_face_id) return null;
    return this.suggestionCropUrls()[subject.thumbnail_face_id] ?? this.faceCropUrls()[subject.id] ?? null;
  }
}
```

- [ ] **Step 2: Update the HTML template**

Replace the contents of `people-view.component.html` with:

```html
<div class="people-container p-8">
  <div class="flex items-center justify-between mb-8">
    <h1 class="text-3xl font-bold">People &amp; Subjects</h1>
    <button
      class="px-4 py-2 rounded-lg bg-secondary text-secondary-foreground hover:bg-secondary/80 transition-colors disabled:opacity-50"
      (click)="recluster()"
      [disabled]="reclustering()"
    >
      {{ reclustering() ? 'Clustering...' : 'Re-cluster' }}
    </button>
  </div>

  @if (mergeSuggestions().length > 0) {
    <div class="mb-8 p-4 rounded-lg border border-accent/30 bg-accent/5">
      <h2 class="text-lg font-semibold mb-3">Possible Duplicates</h2>
      <div class="flex flex-col gap-3">
        @for (suggestion of mergeSuggestions(); track suggestion.id) {
          <div class="flex items-center gap-4 p-3 rounded-md bg-background border border-border">
            <div class="flex items-center gap-3 flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <div class="w-12 h-12 rounded-full overflow-hidden border border-border bg-muted flex items-center justify-center shrink-0">
                  @if (getThumbUrl(suggestion.subject_a)) {
                    <img [src]="getThumbUrl(suggestion.subject_a)" alt="" class="w-full h-full object-cover" />
                  } @else {
                    <span class="text-lg text-muted-foreground">👤</span>
                  }
                </div>
                <div class="w-12 h-12 rounded-full overflow-hidden border border-border bg-muted flex items-center justify-center shrink-0">
                  @if (getThumbUrl(suggestion.subject_b)) {
                    <img [src]="getThumbUrl(suggestion.subject_b)" alt="" class="w-full h-full object-cover" />
                  } @else {
                    <span class="text-lg text-muted-foreground">👤</span>
                  }
                </div>
              </div>
              <div class="min-w-0">
                <div class="font-medium truncate">
                  {{ suggestion.subject_a.name || 'Unnamed' }}
                  &amp;
                  {{ suggestion.subject_b.name || 'Unnamed' }}
                </div>
                <div class="text-xs text-muted-foreground">
                  {{ suggestion.cross_match_count }} similar face pairs
                </div>
              </div>
            </div>
            <div class="flex items-center gap-2 shrink-0">
              <button
                class="px-3 py-1.5 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
                (click)="merge(suggestion)"
              >
                Merge
              </button>
              <button
                class="px-3 py-1.5 text-sm rounded-md border border-border hover:bg-muted transition-colors text-muted-foreground"
                (click)="dismiss(suggestion)"
              >
                Dismiss
              </button>
            </div>
          </div>
        }
      </div>
    </div>
  }

  <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-6">
    @for (subject of photoService.subjects(); track subject.id) {
      <a class="group cursor-pointer flex flex-col items-center gap-3 transition-transform hover:scale-105" [routerLink]="['/subject', subject.id]">
        <div class="w-32 h-32 rounded-full overflow-hidden border-2 border-border group-hover:border-accent bg-muted flex items-center justify-center transition-colors">
          @if (faceCropUrls()[subject.id]) {
            <img [src]="faceCropUrls()[subject.id]" alt="Face Crop" class="w-full h-full object-cover" />
          } @else {
            <span class="text-4xl text-muted-foreground">👤</span>
          }
        </div>

        <div class="text-center">
          <span class="font-medium block" [class.text-muted-foreground]="!subject.name">
            {{ subject.name || 'Unnamed' }}
          </span>
        </div>
      </a>
    } @empty {
      <div class="col-span-full py-20 text-center text-muted-foreground">
        <p class="text-lg mb-2">No subjects discovered yet.</p>
        <p class="text-sm">Start adding photos with faces to see people here!</p>
      </div>
    }
  </div>
</div>
```

- [ ] **Step 3: Verify the Angular build**

Run: `cd /home/pi/nebula && npx ng build 2>&1 | tail -5`
Expected: Build succeeds

- [ ] **Step 4: Commit**

```bash
git add src/app/components/people-view/
git commit -m "feat: add Possible Duplicates section to People view"
```

---

### Task 9: Add similar subjects + name conflict dialog to Subject Detail

**Files:**
- Modify: `src/app/components/subject-detail/subject-detail.component.ts`
- Modify: `src/app/components/subject-detail/subject-detail.component.html`

- [ ] **Step 1: Update the component TypeScript**

In `subject-detail.component.ts`, add the new imports, signals, and methods. The full file should be:

```typescript
import {
  Component,
  OnInit,
  inject,
  signal,
  computed,
  ChangeDetectionStrategy,
} from '@angular/core';
import { CommonModule, Location } from '@angular/common';
import { ActivatedRoute, RouterLink, Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SearchResult, VirtualRow, SubjectDetail, MergeSuggestion } from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { FormsModule } from '@angular/forms';
import { buildJustifiedRows } from '../../utils/justified-layout';
import { LightboxComponent } from '../lightbox/lightbox.component';

@Component({
  selector: 'app-subject-detail',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CommonModule,
    RouterLink,
    LucideAngularModule,
    PhotoGridComponent,
    FormsModule,
    LightboxComponent,
  ],
  templateUrl: './subject-detail.component.html',
  styleUrl: './subject-detail.component.css',
})
export class SubjectDetailComponent implements OnInit {
  private route = inject(ActivatedRoute);
  private location = inject(Location);
  private router = inject(Router);
  protected photos = inject(PhotoService);

  protected subjectId = signal<number | null>(null);
  protected detail = signal<SubjectDetail | null>(null);
  protected subjectPhotos = signal<SearchResult[]>([]);
  protected faceCropUrl = signal<string | null>(null);

  protected isEditingName = signal(false);
  protected editedName = signal('');
  protected isMenuOpen = signal(false);

  protected similarSubjects = signal<MergeSuggestion[]>([]);
  protected similarCropUrls = signal<Record<number, string>>({});
  protected showNameConflict = signal(false);
  protected conflictingSubjectId = signal<number | null>(null);

  protected readonly virtualRows = computed<VirtualRow[]>(() => {
    const images = this.subjectPhotos();
    const width = this.photos.viewportWidth();
    const targetRowHeight = this.photos.targetRowHeight();

    const rows: VirtualRow[] = [];
    const justifiedRows = buildJustifiedRows(images, width, targetRowHeight, 4);
    for (const row of justifiedRows) {
      rows.push({ type: 'row', images: row.images, rowHeight: row.rowHeight });
    }
    return rows;
  });

  ngOnInit() {
    this.route.params.subscribe((params) => {
      const id = Number(params['id']);
      if (!isNaN(id)) {
        this.subjectId.set(id);
        void this.loadData(id);
      }
    });
  }

  private async loadData(id: number) {
    try {
      const detail = await this.photos.getSubjectDetail(id);
      this.detail.set(detail);
      this.editedName.set(detail.subject.name || '');

      if (detail.subject.thumbnail_face_id) {
        const path = await this.photos.getFaceCrop(detail.subject.thumbnail_face_id);
        this.faceCropUrl.set(this.photos.thumbnailUrl(path));
      }

      const photos = await this.photos.getSubjectPhotos(id);
      this.subjectPhotos.set(photos);

      void this.loadSimilarSubjects(id);
    } catch (e) {
      console.error('Failed to load subject detail', e);
      this.location.back();
    }
  }

  private async loadSimilarSubjects(id: number) {
    try {
      const all = await this.photos.getMergeSuggestions();
      const related = all.filter(
        (s) => s.subject_a.id === id || s.subject_b.id === id
      );
      this.similarSubjects.set(related);
      void this.loadSimilarCrops(related);
    } catch (e) {
      console.error('Failed to load similar subjects', e);
    }
  }

  private async loadSimilarCrops(suggestions: MergeSuggestion[]) {
    const ids = new Set<number>();
    for (const s of suggestions) {
      if (s.subject_a.thumbnail_face_id) ids.add(s.subject_a.thumbnail_face_id);
      if (s.subject_b.thumbnail_face_id) ids.add(s.subject_b.thumbnail_face_id);
    }
    const urls: Record<number, string> = {};
    await Promise.all(
      [...ids].map(async (faceId) => {
        try {
          const path = await this.photos.getFaceCrop(faceId);
          const url = this.photos.thumbnailUrl(path);
          if (url) urls[faceId] = url;
        } catch {}
      })
    );
    this.similarCropUrls.set(urls);
  }

  protected goBack() {
    this.location.back();
  }

  protected startEdit() {
    this.isEditingName.set(true);
  }

  protected cancelEdit() {
    this.isEditingName.set(false);
    this.editedName.set(this.detail()?.subject.name || '');
  }

  protected async saveName() {
    const id = this.subjectId();
    const name = this.editedName().trim();
    if (id !== null) {
      const result = await this.photos.nameSubject(id, name || null);
      this.detail.update((d) => {
        if (d) d.subject.name = name || null;
        return d;
      });
      this.isEditingName.set(false);

      if (result.duplicate_subject_id) {
        this.conflictingSubjectId.set(result.duplicate_subject_id);
        this.showNameConflict.set(true);
      }
    }
  }

  protected async confirmMerge() {
    const id = this.subjectId();
    const conflictId = this.conflictingSubjectId();
    if (id !== null && conflictId !== null) {
      await this.photos.mergeSubjects(id, conflictId);
      this.showNameConflict.set(false);
      this.conflictingSubjectId.set(null);
      this.router.navigate(['/subject', id]);
    }
  }

  protected cancelMerge() {
    this.showNameConflict.set(false);
    this.conflictingSubjectId.set(null);
  }

  protected async mergeSimilar(suggestion: MergeSuggestion) {
    const id = this.subjectId();
    if (id === null) return;
    const sourceId =
      suggestion.subject_a.id === id
        ? suggestion.subject_b.id
        : suggestion.subject_a.id;
    await this.photos.mergeSubjects(id, sourceId);
    void this.loadData(id);
  }

  protected async dismissSimilar(suggestion: MergeSuggestion) {
    await this.photos.dismissMergeSuggestion(suggestion.id);
    this.similarSubjects.update((list) =>
      list.filter((s) => s.id !== suggestion.id)
    );
  }

  protected getOtherSubject(suggestion: MergeSuggestion) {
    const id = this.subjectId();
    return suggestion.subject_a.id === id
      ? suggestion.subject_b
      : suggestion.subject_a;
  }

  protected getSimilarThumbUrl(subject: { thumbnail_face_id: number | null }): string | null {
    if (!subject.thumbnail_face_id) return null;
    return this.similarCropUrls()[subject.thumbnail_face_id] ?? null;
  }

  protected toggleMenu() {
    this.isMenuOpen.update((v) => !v);
  }

  protected closeMenu() {
    this.isMenuOpen.set(false);
  }
}
```

- [ ] **Step 2: Update the HTML template**

Add the following sections to `subject-detail.component.html`:

After the `<main>` closing tag (the photo grid section) and before the `<app-lightbox>`, add the similar subjects card and name conflict dialog:

```html
  <!-- Similar Subjects -->
  @if (similarSubjects().length > 0) {
    <div class="border-t border-border px-6 py-4">
      <h2 class="text-sm font-semibold text-muted-foreground uppercase tracking-wide mb-3">Similar Subjects</h2>
      <div class="flex flex-col gap-2">
        @for (suggestion of similarSubjects(); track suggestion.id) {
          @let other = getOtherSubject(suggestion);
          <div class="flex items-center gap-3 p-2 rounded-md border border-border bg-card">
            <a
              [routerLink]="['/subject', other.id]"
              class="flex items-center gap-3 flex-1 min-w-0 hover:bg-muted/50 rounded-md p-1 transition-colors"
            >
              <div class="w-10 h-10 rounded-full overflow-hidden border border-border bg-muted flex items-center justify-center shrink-0">
                @if (getSimilarThumbUrl(other)) {
                  <img [src]="getSimilarThumbUrl(other)" alt="" class="w-full h-full object-cover" />
                } @else {
                  <span class="text-sm text-muted-foreground">👤</span>
                }
              </div>
              <span class="font-medium text-sm truncate">{{ other.name || 'Unnamed' }}</span>
            </a>
            <button
              class="px-2 py-1 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
              (click)="mergeSimilar(suggestion)"
            >
              Merge
            </button>
            <button
              class="px-2 py-1 text-xs rounded-md border border-border hover:bg-muted transition-colors text-muted-foreground"
              (click)="dismissSimilar(suggestion)"
            >
              Dismiss
            </button>
          </div>
        }
      </div>
    </div>
  }

  <!-- Name Conflict Dialog -->
  @if (showNameConflict()) {
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div class="bg-card border border-border rounded-lg p-6 max-w-sm mx-4 shadow-xl">
        <h3 class="text-lg font-semibold mb-2">Duplicate Name</h3>
        <p class="text-sm text-muted-foreground mb-4">
          A subject with this name already exists. Would you like to merge them?
        </p>
        <div class="flex justify-end gap-2">
          <button
            class="px-4 py-2 text-sm rounded-md border border-border hover:bg-muted transition-colors"
            (click)="cancelMerge()"
          >
            Keep Separate
          </button>
          <button
            class="px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
            (click)="confirmMerge()"
          >
            Merge
          </button>
        </div>
      </div>
    </div>
  }

  <app-lightbox [image]="photos.selectedImage()" />
```

- [ ] **Step 3: Verify the Angular build**

Run: `cd /home/pi/nebula && npx ng build 2>&1 | tail -5`
Expected: Build succeeds

- [ ] **Step 4: Commit**

```bash
git add src/app/components/subject-detail/
git commit -m "feat: add similar subjects and name conflict dialog to Subject Detail"
```

---

### Task 10: Full build verification

- [ ] **Step 1: Run full Rust check**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: No errors

- [ ] **Step 2: Run full Angular build**

Run: `cd /home/pi/nebula && npx ng build 2>&1 | tail -10`
Expected: Build succeeds

- [ ] **Step 3: Verify no lint/type issues**

Check the build output for any TypeScript errors or warnings related to the new code. Fix any issues found.
