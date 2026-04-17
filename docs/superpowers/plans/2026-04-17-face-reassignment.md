# Face Subject Reassignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow users to reassign or remove incorrectly-assigned faces in the lightbox, with manual assignments protected during recluster.

**Architecture:** Extend the existing `FaceAssignPopoverComponent` with a "reassign mode" for already-assigned faces. Add an `is_manual` flag to the `faces` table to protect user corrections during reclustering. Store correction history in a new `face_corrections` table.

**Tech Stack:** Rust (Tauri backend, SQLx, SQLite), Angular (frontend, spartan-ng components)

---

## File Structure

| File | Responsibility | Action |
|------|---------------|--------|
| `src-tauri/src/db.rs` | Schema migration, DB functions | Modify |
| `src-tauri/src/models.rs` | `Face` struct with `is_manual` | Modify |
| `src-tauri/src/commands.rs` | Tauri commands: `unassign_face`, modified `assign_face_to_subject` | Modify |
| `src-tauri/src/clustering.rs` | Protect `is_manual` faces during recluster | Modify |
| `src-tauri/src/lib.rs` | Register new `unassign_face` command | Modify |
| `src/app/models/models.ts` | `Face` interface with `is_manual` | Modify |
| `src/app/services/photo.service.ts` | `unassignFace()` method | Modify |
| `src/app/components/face-assign-popover/face-assign-popover.component.ts` | Reassign mode logic | Modify |
| `src/app/components/face-assign-popover/face-assign-popover.component.html` | Reassign mode template | Modify |
| `src/app/components/lightbox/lightbox.component.ts` | Face overlay popover, sidebar pencil, state updates | Modify |
| `src/app/components/lightbox/lightbox.component.html` | Popover on overlay, sidebar edit buttons | Modify |

---

### Task 1: Schema Migration — `is_manual` Column and `face_corrections` Table

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add `is_manual` column to faces table in MIGRATIONS**

In `src-tauri/src/db.rs`, find the `MIGRATIONS` constant. After the existing `CREATE INDEX IF NOT EXISTS idx_faces_subject` line (line 63), add the ALTER TABLE and new table:

```rust
"#;

const POST_MIGRATIONS: &str = r#"
ALTER TABLE faces ADD COLUMN is_manual INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS face_corrections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    face_id INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    old_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    new_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_corrections_face ON face_corrections(face_id);
"#;
```

Then in the `init_db` function, after the loop that runs `MIGRATIONS`, add a block to run `POST_MIGRATIONS` (using `ALTER TABLE` which can fail if column already exists, so we catch the error):

```rust
    for stmt in MIGRATIONS.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            sqlx::query(s).execute(&pool).await?;
        }
    }

    for stmt in POST_MIGRATIONS.split(';') {
        let s = stmt.trim();
        if !s.is_empty() {
            let _ = sqlx::query(s).execute(&pool).await;
        }
    }

    Ok(pool)
```

Using `let _ =` ignores errors from `ALTER TABLE` when the column already exists on subsequent startups.

- [ ] **Step 2: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles successfully

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add is_manual column and face_corrections table schema"
```

---

### Task 2: Rust Model — Add `is_manual` to Face Struct

**Files:**
- Modify: `src-tauri/src/models.rs`

- [ ] **Step 1: Add `is_manual` field to `Face` struct**

In `src-tauri/src/models.rs`, add `is_manual` to the `Face` struct after `added_at`:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Face {
    pub id: i64,
    pub image_id: i64,
    pub subject_id: Option<i64>,
    pub bbox_x: f64,
    pub bbox_y: f64,
    pub bbox_w: f64,
    pub bbox_h: f64,
    pub embedding: Option<Vec<u8>>,
    pub added_at: i64,
    pub is_manual: bool,
}
```

- [ ] **Step 2: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compile errors in `db.rs` where `Face` is constructed — that's expected, we fix them in Task 3.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/models.rs
git commit -m "feat: add is_manual field to Face model"
```

---

### Task 3: DB Functions — New and Modified

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Fix existing Face construction to include `is_manual`**

Find the `list_faces_for_image` function (around line 619). The SQL query and `Face` construction need `is_manual`:

```rust
pub async fn list_faces_for_image(pool: &SqlitePool, image_id: i64) -> Result<Vec<Face>> {
    let rows = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at, is_manual
         FROM faces WHERE image_id = ? ORDER BY added_at DESC",
    )
    .bind(image_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Face {
            id: r.get("id"),
            image_id: r.get("image_id"),
            subject_id: r.get("subject_id"),
            bbox_x: r.get("bbox_x"),
            bbox_y: r.get("bbox_y"),
            bbox_w: r.get("bbox_w"),
            bbox_h: r.get("bbox_h"),
            embedding: r.get("embedding"),
            added_at: r.get("added_at"),
            is_manual: r.get::<i32, _>("is_manual") != 0,
        })
        .collect())
}
```

Find any other places where `Face` is constructed (search for `Face {` in db.rs). The `list_faces` function should have a similar query — update it too:

```rust
pub async fn list_faces(pool: &SqlitePool, subject_id: i64) -> Result<Vec<Face>> {
```

Add `is_manual` to its SELECT and `Face` construction as well. Use `is_manual: r.get::<i32, _>("is_manual") != 0` for the boolean conversion.

- [ ] **Step 2: Modify `assign_face_to_subject` to set `is_manual = 1`**

Replace the existing function:

```rust
pub async fn assign_face_to_subject(pool: &SqlitePool, face_id: i64, subject_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = ?, is_manual = 1 WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 3: Modify `create_subject_for_face` to set `is_manual = 1`**

In the UPDATE query inside `create_subject_for_face` (around line 921), change:

```rust
    sqlx::query("UPDATE faces SET subject_id = ?, is_manual = 1 WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
```

- [ ] **Step 4: Add `get_face_subject_id` helper**

After `assign_face_to_subject`:

```rust
pub async fn get_face_subject_id(pool: &SqlitePool, face_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT subject_id FROM faces WHERE id = ?")
        .bind(face_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.get::<Option<i64>, _>("subject_id")))
}
```

- [ ] **Step 5: Add `record_face_correction` function**

```rust
pub async fn record_face_correction(pool: &SqlitePool, face_id: i64, old_subject_id: Option<i64>, new_subject_id: Option<i64>) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO face_corrections (face_id, old_subject_id, new_subject_id, created_at) VALUES (?, ?, ?, ?)"
    )
    .bind(face_id)
    .bind(old_subject_id)
    .bind(new_subject_id)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 6: Add `unassign_face` function**

```rust
pub async fn unassign_face(pool: &SqlitePool, face_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = NULL, is_manual = 1 WHERE id = ?")
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 7: Modify `get_all_faces_with_embeddings` to include `is_manual`**

```rust
pub async fn get_all_faces_with_embeddings(pool: &SqlitePool) -> Result<Vec<(i64, Option<i64>, Vec<u8>, bool)>> {
    let rows = sqlx::query(
        "SELECT id, subject_id, embedding, is_manual FROM faces WHERE embedding IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let id: i64 = r.get("id");
            let subject_id: Option<i64> = r.get("subject_id");
            let emb: Option<Vec<u8>> = r.get("embedding");
            let is_manual: bool = r.get::<i32, _>("is_manual") != 0;
            emb.map(|e| (id, subject_id, e, is_manual))
        })
        .collect())
}
```

- [ ] **Step 8: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Errors in `clustering.rs` where `get_all_faces_with_embeddings` return type changed — we fix that in Task 5. But `db.rs` itself should compile. Check for any remaining `Face {` constructions missing `is_manual`.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add unassign_face, record_face_correction, is_manual to DB functions"
```

---

### Task 4: Tauri Commands — New `unassign_face`, Modified `assign_face_to_subject`

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `unassign_face` command**

In `src-tauri/src/commands.rs`, add after `create_subject_for_face`:

```rust
#[tauri::command]
pub async fn unassign_face(
    face_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let old_subject_id = db::get_face_subject_id(&state.pool, face_id)
        .await
        .map_err(map_err)?;
    db::unassign_face(&state.pool, face_id)
        .await
        .map_err(map_err)?;
    db::record_face_correction(&state.pool, face_id, old_subject_id, None)
        .await
        .map_err(map_err)?;
    let _ = db::auto_assign_missing_thumbnails(&state.pool).await;
    let _ = db::delete_subjects_with_no_faces(&state.pool).await;
    Ok(())
}
```

- [ ] **Step 2: Modify `assign_face_to_subject` to record correction**

Replace the existing `assign_face_to_subject` command:

```rust
#[tauri::command]
pub async fn assign_face_to_subject(
    face_id: i64,
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let old_subject_id = db::get_face_subject_id(&state.pool, face_id)
        .await
        .map_err(map_err)?;
    db::assign_face_to_subject(&state.pool, face_id, subject_id)
        .await
        .map_err(map_err)?;
    if old_subject_id != Some(subject_id) {
        let _ = db::record_face_correction(&state.pool, face_id, old_subject_id, Some(subject_id))
            .await;
    }
    let _ = db::auto_assign_missing_thumbnails(&state.pool).await;
    let _ = db::delete_subjects_with_no_faces(&state.pool).await;
    Ok(())
}
```

- [ ] **Step 3: Register `unassign_face` in `lib.rs`**

In `src-tauri/src/lib.rs`, add to the `invoke_handler` list after `create_subject_for_face`:

```rust
            commands::create_subject_for_face,
            commands::unassign_face,
```

- [ ] **Step 4: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: May have errors from `clustering.rs` — that's fine, fix in Task 5.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add unassign_face command, record corrections on assign"
```

---

### Task 5: Clustering — Protect Manual Assignments During Recluster

**Files:**
- Modify: `src-tauri/src/clustering.rs`

- [ ] **Step 1: Update `recluster_all` to handle `is_manual`**

Replace the function body in `src-tauri/src/clustering.rs`. The key changes:
1. Updated destructuring for the new 4-tuple from `get_all_faces_with_embeddings`
2. After HDBSCAN clustering, skip faces with `is_manual = true`

```rust
pub async fn recluster_all(pool: &SqlitePool) -> Result<ReclusterResult> {
    let faces = db::get_all_faces_with_embeddings(pool).await?;

    if faces.is_empty() {
        return Ok(ReclusterResult {
            clusters: 0,
            noise: 0,
            merged: 0,
            deleted: 0,
        });
    }

    let face_ids: Vec<i64> = faces.iter().map(|(id, _, _, _)| *id).collect();
    let old_subject_ids: Vec<Option<i64>> = faces.iter().map(|(_, sid, _, _)| *sid).collect();
    let is_manual_flags: Vec<bool> = faces.iter().map(|(_, _, _, m)| *m).collect();

    let embeddings: Vec<Vec<f32>> = faces
        .iter()
        .filter_map(|(_, _, emb_blob, _)| crate::embedder::bytes_to_f32_vec(emb_blob).ok())
        .collect();

    if embeddings.len() != face_ids.len() {
        anyhow::bail!(
            "Embedding decode mismatch: {} faces but {} decoded embeddings",
            face_ids.len(),
            embeddings.len()
        );
    }

    let hyper_params = HdbscanHyperParams::builder()
        .min_cluster_size(2)
        .min_samples(2)
        .build();

    let clusterer = Hdbscan::new(&embeddings, hyper_params);
    let labels = clusterer.cluster().map_err(|e| anyhow::anyhow!("HDBSCAN failed: {}", e))?;

    let mut cluster_to_face_indices: HashMap<i32, Vec<usize>> = HashMap::new();
    for (idx, &label) in labels.iter().enumerate() {
        if is_manual_flags[idx] {
            continue;
        }
        cluster_to_face_indices.entry(label).or_default().push(idx);
    }

    let mut subjects_merged = 0i64;

    for (&label, face_indices) in &cluster_to_face_indices {
        if label < 0 {
            continue;
        }

        let existing_subject_ids: Vec<Option<i64>> = face_indices
            .iter()
            .map(|&idx| old_subject_ids[idx])
            .collect();

        let non_none: Vec<i64> = existing_subject_ids.iter().filter_map(|&s| s).collect();

        let chosen_subject_id = if !non_none.is_empty() {
            let mut counts: HashMap<i64, usize> = HashMap::new();
            for &sid in &non_none {
                *counts.entry(sid).or_default() += 1;
            }
            let best = counts.into_iter().max_by_key(|(_, c)| *c).map(|(s, _)| s).unwrap();
            subjects_merged += non_none.iter().filter(|&&s| s != best).count() as i64;
            best
        } else {
            db::insert_subject(pool, None, "person").await?
        };

        for &idx in face_indices {
            db::update_face_subject(pool, face_ids[idx], Some(chosen_subject_id)).await?;
        }
    }

    let noise_count = cluster_to_face_indices.get(&-1).map(|v| v.len()).unwrap_or(0);
    if let Some(noise_indices) = cluster_to_face_indices.get(&-1) {
        for &idx in noise_indices {
            db::update_face_subject(pool, face_ids[idx], None).await?;
        }
    }

    let deleted = db::delete_subjects_with_no_faces(pool).await?;

    let _ = db::auto_assign_missing_thumbnails(pool).await;

    let _ = find_merge_suggestions(pool).await;

    Ok(ReclusterResult {
        clusters: cluster_to_face_indices.keys().filter(|&&l| l >= 0).count(),
        noise: noise_count,
        merged: subjects_merged,
        deleted,
    })
}
```

The key change: `is_manual` faces are excluded from `cluster_to_face_indices` entirely, so they keep their existing subject_id (or stay unassigned if subject_id is NULL).

- [ ] **Step 2: Verify the build compiles**

Run: `cd /home/pi/nebula/src-tauri && cargo check`
Expected: Compiles successfully — no remaining errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat: protect manually-assigned faces during recluster"
```

---

### Task 6: Frontend Model and Service

**Files:**
- Modify: `src/app/models/models.ts`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Add `is_manual` to Face interface in `models.ts`**

```typescript
export interface Face {
  id: number;
  image_id: number;
  subject_id: number | null;
  bbox_x: number;
  bbox_y: number;
  bbox_w: number;
  bbox_h: number;
  added_at: number;
  is_manual: boolean;
}
```

- [ ] **Step 2: Add `unassignFace` method to `photo.service.ts`**

Add after the `createSubjectForFace` method (around line 322):

```typescript
  async unassignFace(faceId: number): Promise<void> {
    await invoke('unassign_face', { faceId });
  }
```

- [ ] **Step 3: Verify the frontend builds**

Run: `cd /home/pi/nebula && npx ng build --configuration development 2>&1 | tail -20`
Expected: Build may fail on `lightbox.component.ts` or `face-assign-popover` — that's expected, we fix them in Tasks 7-8.

- [ ] **Step 4: Commit**

```bash
git add src/app/models/models.ts src/app/services/photo.service.ts
git commit -m "feat: add is_manual to Face model, unassignFace to PhotoService"
```

---

### Task 7: FaceAssignPopover — Add Reassign Mode

**Files:**
- Modify: `src/app/components/face-assign-popover/face-assign-popover.component.ts`
- Modify: `src/app/components/face-assign-popover/face-assign-popover.component.html`

- [ ] **Step 1: Rewrite the TypeScript component**

Replace the entire content of `face-assign-popover.component.ts`:

```typescript
import {
  Component,
  ChangeDetectionStrategy,
  input,
  output,
  inject,
  signal,
  computed,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { LucideAngularModule } from 'lucide-angular';
import { BrnPopoverImports } from '@spartan-ng/brain/popover';
import { BrnCommandImports } from '@spartan-ng/brain/command';
import { HlmPopoverImports } from '@spartan-ng/helm/popover';
import { HlmCommandImports } from '@spartan-ng/helm/command';
import { PhotoService } from '../../services/photo.service';
import { Face, Subject } from '../../models/models';

@Component({
  selector: 'app-face-assign-popover',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CommonModule,
    LucideAngularModule,
    BrnPopoverImports,
    BrnCommandImports,
    HlmPopoverImports,
    HlmCommandImports,
  ],
  templateUrl: './face-assign-popover.component.html',
  styleUrl: './face-assign-popover.component.css',
})
export class FaceAssignPopoverComponent {
  readonly face = input.required<Face>();
  readonly assigned = output<{ face: Face; subject: Subject }>();
  readonly removed = output<{ face: Face }>();

  protected photos = inject(PhotoService);
  protected query = signal('');
  protected isOpen = signal(false);

  protected isReassignMode = computed(() => this.face().subject_id !== null);

  protected currentSubjectName = computed(() => {
    const sid = this.face().subject_id;
    if (!sid) return null;
    const sub = this.photos.subjects().find(s => s.id === sid);
    return sub?.name || 'Unnamed Subject';
  });

  protected filteredSubjects = computed(() => {
    const currentId = this.face().subject_id;
    const q = this.query().toLowerCase().trim();
    let subjects = this.photos.subjects().filter(s => s.id !== currentId);
    if (q) {
      subjects = subjects.filter(s =>
        s.name?.toLowerCase().includes(q) ?? false
      );
    }
    return subjects;
  });

  async open() {
    this.isOpen.set(true);
    this.query.set('');
  }

  close() {
    this.isOpen.set(false);
    this.query.set('');
  }

  async selectSubject(subject: Subject) {
    await this.photos.assignFaceToSubject(this.face().id, subject.id);
    this.assigned.emit({ face: this.face(), subject });
    this.close();
  }

  async createSubject() {
    const name = this.query().trim() || undefined;
    const subject = await this.photos.createSubjectForFace(this.face().id, name);
    this.assigned.emit({ face: this.face(), subject });
    this.close();
  }

  async removeFace() {
    await this.photos.unassignFace(this.face().id);
    this.removed.emit({ face: this.face() });
    this.close();
  }
}
```

- [ ] **Step 2: Rewrite the HTML template**

Replace the entire content of `face-assign-popover.component.html`:

```html
<div brnPopover #pop="brnPopover">
  @if (isReassignMode()) {
    <button brnPopoverTrigger [brnPopoverTriggerFor]="pop"
            class="text-xs text-muted-foreground hover:text-foreground transition-colors bg-transparent border-none p-0 flex items-center gap-1 cursor-pointer"
            (click)="open()">
      <span class="underline underline-offset-2">{{ currentSubjectName() }}</span>
      <lucide-icon name="pencil" [size]="12" class="shrink-0" />
    </button>
  } @else {
    <button brnPopoverTrigger [brnPopoverTriggerFor]="pop"
            class="text-xs text-muted-foreground hover:text-foreground underline underline-offset-2 transition-colors cursor-pointer bg-transparent border-none p-0"
            (click)="open()">
      Tap to add
    </button>
  }

  <ng-template brnPopoverContent>
    <div hlmPopoverContent class="w-60 p-0 overflow-hidden">
      @if (isReassignMode()) {
        <div class="px-3 py-2 border-b border-border">
          <div class="text-xs text-muted-foreground mb-1">Currently:</div>
          <div class="text-sm font-medium">{{ currentSubjectName() }}</div>
        </div>
        <button
          class="w-full flex items-center gap-2 px-3 py-2 text-sm text-destructive hover:bg-accent transition-colors bg-transparent border-none cursor-pointer text-left"
          (click)="removeFace()">
          <lucide-icon name="x" [size]="14" class="shrink-0" />
          Remove from subject
        </button>
        <div class="border-t border-border"></div>
      }
      <hlm-command (searchChange)="query.set($event)">
        <hlm-command-input placeholder="Search subjects..." />

        <hlm-command-list>
          @if (!query()) {
            <button hlmCommandItem value="__create__" (selected)="createSubject()"
                    class="gap-2">
              <lucide-icon name="plus" [size]="16" class="shrink-0" />
              Create new subject
            </button>
            @if (filteredSubjects().length > 0) {
              <div class="border-t my-1"></div>
            }
          }

          @for (subject of filteredSubjects(); track subject.id) {
            <button hlmCommandItem [value]="subject.name ?? 'unnamed-' + subject.id"
                    (selected)="selectSubject(subject)">
              @if (subject.name) {
                {{ subject.name }}
              } @else {
                <em class="text-muted-foreground">Unnamed subject</em>
              }
            </button>
          }

          @if (query()) {
            <button hlmCommandItem [value]="'__create__' + query()" (selected)="createSubject()"
                    class="gap-2 mt-1 border-t pt-1">
              <lucide-icon name="plus" [size]="16" class="shrink-0" />
              Create "{{ query() }}"
            </button>
          }
        </hlm-command-list>
      </hlm-command>
    </div>
  </ng-template>
</div>
```

- [ ] **Step 3: Verify the frontend builds**

Run: `cd /home/pi/nebula && npx ng build --configuration development 2>&1 | tail -20`
Expected: Build errors in `lightbox.component.ts` where it uses the old `assigned` output — we fix that in Task 8.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/face-assign-popover/
git commit -m "feat: add reassign mode to FaceAssignPopover"
```

---

### Task 8: Lightbox — Face Overlay Popover and Sidebar Pencil Icons

**Files:**
- Modify: `src/app/components/lightbox/lightbox.component.ts`
- Modify: `src/app/components/lightbox/lightbox.component.html`

- [ ] **Step 1: Update `lightbox.component.ts`**

Add the `onFaceRemoved` handler and update `onFaceAssigned` to handle the `removed` output:

```typescript
  onFaceAssigned(event: { face: Face; subject: Subject }) {
    this.faces.update(faces =>
      faces.map(f => f.id === event.face.id ? { ...f, subject_id: event.subject.id } : f)
    );
    this.localSubjects.update(subjects => {
      if (subjects.find(s => s.id === event.subject.id)) return subjects;
      return [...subjects, event.subject];
    });
    if (this.unassignedFaces().length === 0) {
      this.showFacesToAdd.set(false);
    }
  }

  onFaceRemoved(event: { face: Face }) {
    this.faces.update(faces =>
      faces.map(f => f.id === event.face.id ? { ...f, subject_id: null } : f)
    );
  }
```

No changes needed to the existing `onFaceAssigned` — it already handles the subject update correctly.

- [ ] **Step 2: Add pencil icon to sidebar People section in `lightbox.component.html`**

Face overlay clicks stay unchanged (navigate to subject). The reassignment UI lives in the sidebar's People section.

Find the sidebar People section in `lightbox.component.html`. Replace the `@for (face of assignedFaces())` block with:

```html
              @for (face of assignedFaces(); track face.id) {
                <div
                  class="person-item"
                  [class.active]="activeFaceId() === face.id"
                  (mouseenter)="setActiveFace(face.id)"
                  (mouseleave)="setActiveFace(null)">
                  <div class="person-avatar" (click)="navigateToSubject(face.subject_id)">👤</div>
                  <div class="person-name" (click)="navigateToSubject(face.subject_id)">{{ getSubjectName(face.subject_id) }}</div>
                  <app-face-assign-popover
                    class="ml-auto"
                    [face]="face"
                    (assigned)="onFaceAssigned($event)"
                    (removed)="onFaceRemoved($event)" />
                </div>
              }
```

The person-avatar and person-name click to navigate to subject (unchanged). The `app-face-assign-popover` renders the pencil icon + reassign popover, pushed to the right with `ml-auto`.

The old `(click)="navigateToSubject(face.subject_id)"` on the outer `person-item` div is removed — navigation is now on the avatar and name elements specifically.

- [ ] **Step 3: Verify the frontend builds**

Run: `cd /home/pi/nebula && npx ng build --configuration development 2>&1 | tail -20`
Expected: Build succeeds

- [ ] **Step 4: Commit**

```bash
git add src/app/components/lightbox/
git commit -m "feat: add face reassignment UI to lightbox sidebar"
```

---

### Task 9: Full Build Verification

- [ ] **Step 1: Run full Rust build**

Run: `cd /home/pi/nebula/src-tauri && cargo build`
Expected: Compiles successfully

- [ ] **Step 2: Run full Angular build**

Run: `cd /home/pi/nebula && npx ng build --configuration development`
Expected: Build succeeds with no errors

- [ ] **Step 3: Run `cargo clippy` on the Rust code**

Run: `cd /home/pi/nebula/src-tauri && cargo clippy -- -D warnings 2>&1 | tail -30`
Expected: No errors (warnings are OK but fix if easy)

- [ ] **Step 4: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "fix: address build issues from face reassignment feature"
```
