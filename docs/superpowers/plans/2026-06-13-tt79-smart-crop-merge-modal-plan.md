# TT-79 Smart-crop Merge Modal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make merge-modal thumbnails larger and face-centered by adding a backend endpoint that returns one row per subject face occurrence, and a new frontend grid that smart-crops each cell around the detected face.

**Architecture:** Add a Rust `SubjectPhotoFace` entity and `get_subject_photos_with_faces` command that flattens image+face rows ordered by date. Add an Angular `SubjectPhotoFace` model, a `PhotoService` method, a dedicated `MergePhotoGridComponent`, and update `MergeReviewComponent` to use the new grid and data shape.

**Tech Stack:** Rust (Tauri, sqlx, serde), Angular 20 standalone components, TypeScript, Vitest, Tailwind CSS.

---

## File map

- **Create:**
  - `src-tauri/src/models/entities.rs` additions: `SubjectPhotoFace`, `FaceBBox`
  - `src-tauri/src/people/repo.rs` addition: `list_faces_for_subject_with_images`
  - `src-tauri/src/people/commands.rs` addition: `get_subject_photos_with_faces`
  - `src/app/models/models.ts` additions: `SubjectPhotoFace`, `FaceBBox`
  - `src/app/services/photo.service.ts` addition: `getSubjectPhotosWithFaces`
  - `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`
  - `src/app/components/merge-photo-grid/merge-photo-grid.component.html`
  - `src/app/components/merge-photo-grid/merge-photo-grid.component.css`
  - `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`

- **Modify:**
  - `src-tauri/src/app/mod.rs`: register new command
  - `src/app/components/merge-review/merge-review.component.ts`: load `SubjectPhotoFace[]` instead of `SearchResult[]`
  - `src/app/components/merge-review/merge-review.component.html`: use `app-merge-photo-grid`, update labels
  - `src/app/components/merge-review/merge-review.component.spec.ts`: update tests to use new data shape

---

### Task 1: Add backend data model

**Files:**
- Modify: `src-tauri/src/models/entities.rs`

- [ ] **Step 1: Add `FaceBBox` and `SubjectPhotoFace` structs**

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FaceBBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubjectPhotoFace {
    pub image_id: i64,
    pub path: String,
    pub thumbnail_path: Option<String>,
    pub preview_path: Option<String>,
    pub date_taken: Option<i64>,
    pub mtime: i64,
    pub face_bbox: FaceBBox,
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/models/entities.rs
git commit -m "feat(models): add SubjectPhotoFace and FaceBBox entities"
```

---

### Task 2: Add repo query for subject faces with images

**Files:**
- Modify: `src-tauri/src/people/repo.rs`

- [ ] **Step 1: Write the repo function**

Insert below `list_images_for_subject`:

```rust
pub async fn list_faces_for_subject_with_images(
    pool: &SqlitePool,
    subject_id: i64,
) -> Result<Vec<crate::models::SubjectPhotoFace>> {
    let rows = sqlx::query(
        r#"SELECT i.id AS image_id, i.path, i.thumbnail_path, i.preview_path,
                  i.date_taken, i.mtime,
                  f.bbox_x, f.bbox_y, f.bbox_w, f.bbox_h
           FROM faces f
           JOIN images i ON i.id = f.image_id
           WHERE f.subject_id = ? AND i.deleted_at IS NULL
           ORDER BY COALESCE(i.date_taken, i.mtime) DESC"#,
    )
    .bind(subject_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| crate::models::SubjectPhotoFace {
            image_id: r.get("image_id"),
            path: r.get("path"),
            thumbnail_path: r.get("thumbnail_path"),
            preview_path: r.get("preview_path"),
            date_taken: r.get("date_taken"),
            mtime: r.get("mtime"),
            face_bbox: crate::models::FaceBBox {
                x: r.get("bbox_x"),
                y: r.get("bbox_y"),
                w: r.get("bbox_w"),
                h: r.get("bbox_h"),
            },
        })
        .collect())
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/people/repo.rs
git commit -m "feat(people): add list_faces_for_subject_with_images repo query"
```

---

### Task 3: Add Tauri command

**Files:**
- Modify: `src-tauri/src/people/commands.rs`
- Modify: `src-tauri/src/app/mod.rs`

- [ ] **Step 1: Add command handler**

In `src-tauri/src/people/commands.rs`, add after `get_subject_photos`:

```rust
#[tauri::command]
pub async fn get_subject_photos_with_faces(
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::models::SubjectPhotoFace>, String> {
    repo::list_faces_for_subject_with_images(&state.pool, subject_id)
        .await
        .map_err(map_err)
}
```

- [ ] **Step 2: Register command**

In `src-tauri/src/app/mod.rs`, add `crate::people::commands::get_subject_photos_with_faces,` immediately after the existing `get_subject_photos` entry in `generate_handler!`.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/people/commands.rs src-tauri/src/app/mod.rs
git commit -m "feat(people): add get_subject_photos_with_faces command"
```

---

### Task 4: Add frontend data model and service method

**Files:**
- Modify: `src/app/models/models.ts`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Add TypeScript interfaces**

In `src/app/models/models.ts`, add after `MergeSuggestion`:

```ts
export interface FaceBBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface SubjectPhotoFace {
  image_id: number;
  path: string;
  thumbnail_path: string | null;
  preview_path: string | null;
  date_taken: number | null;
  mtime: number;
  face_bbox: FaceBBox;
}
```

- [ ] **Step 2: Add service method**

In `src/app/services/photo.service.ts`, add after `getSubjectPhotos`:

```ts
async getSubjectPhotosWithFaces(subjectId: number): Promise<SubjectPhotoFace[]> {
  return await invoke<SubjectPhotoFace[]>('get_subject_photos_with_faces', { subjectId });
}
```

Ensure `SubjectPhotoFace` and `FaceBBox` are imported in the existing models import.

- [ ] **Step 3: Commit**

```bash
git add src/app/models/models.ts src/app/services/photo.service.ts
git commit -m "feat(photo-service): add getSubjectPhotosWithFaces and SubjectPhotoFace model"
```

---

### Task 5: Create merge-photo-grid component

**Files:**
- Create: `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`
- Create: `src/app/components/merge-photo-grid/merge-photo-grid.component.html`
- Create: `src/app/components/merge-photo-grid/merge-photo-grid.component.css`
- Create: `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`

- [ ] **Step 1: Write component**

```ts
import {
  Component,
  Input,
  ChangeDetectionStrategy,
  inject,
  ElementRef,
  AfterViewInit,
  OnDestroy,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { LucideAngularModule } from 'lucide-angular';
import { SubjectPhotoFace } from '../../models/models';
import { PhotoService } from '../../services/photo.service';

function focusPercent(bbox: { x: number; y: number; w: number; h: number }): { x: string; y: string } {
  const cx = bbox.x + bbox.w / 2;
  const cy = bbox.y + bbox.h / 2;
  return {
    x: `${Math.max(0, Math.min(100, cx * 100))}%`,
    y: `${Math.max(0, Math.min(100, cy * 100))}%`,
  };
}

@Component({
  selector: 'app-merge-photo-grid',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, LucideAngularModule],
  templateUrl: './merge-photo-grid.component.html',
  styleUrl: './merge-photo-grid.component.css',
})
export class MergePhotoGridComponent implements AfterViewInit, OnDestroy {
  private photos = inject(PhotoService);
  private host = inject(ElementRef<HTMLElement>);

  private observer?: IntersectionObserver;
  private visible = new Set<number>();
  private flushTimer: ReturnType<typeof setTimeout> | null = null;

  @Input() images: SubjectPhotoFace[] = [];

  ngAfterViewInit(): void {
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const id = Number((e.target as HTMLElement).dataset['id']);
          if (Number.isNaN(id)) continue;
          if (e.isIntersecting) this.visible.add(id);
          else this.visible.delete(id);
        }
        this.scheduleFlush();
      },
      { root: null, rootMargin: '400px', threshold: 0.01 }
    );
    this.observeCells();
  }

  private observeCells(): void {
    if (!this.observer) return;
    this.observer.disconnect();
    const cells = this.host.nativeElement.querySelectorAll('.merge-photo-cell[data-id]');
    cells.forEach((el: Element) => this.observer!.observe(el));
  }

  private scheduleFlush(): void {
    if (this.flushTimer) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      if (this.visible.size > 0) {
        this.photos.prioritizePreviews([...this.visible]);
      }
    }, 100);
  }

  ngOnDestroy(): void {
    this.observer?.disconnect();
    if (this.flushTimer) clearTimeout(this.flushTimer);
  }

  protected thumbUrl(img: SubjectPhotoFace): string | null {
    return this.photos.thumbnailUrl(img.thumbnail_path ?? img.preview_path);
  }

  protected focus(img: SubjectPhotoFace): { x: string; y: string } {
    return focusPercent(img.face_bbox);
  }

  protected onClick(img: SubjectPhotoFace): void {
    this.photos.openLightbox({
      image_id: img.image_id,
      path: img.path,
      thumbnail_path: img.thumbnail_path,
      preview_path: img.preview_path,
      score: 0,
      date_taken: img.date_taken,
      mtime: img.mtime,
      semantic_analysis_done: true,
      subject_analysis_done: true,
    });
  }
}
```

- [ ] **Step 2: Write template**

```html
<div class="merge-photo-grid">
  @for (img of images; track img.image_id + '-' + img.face_bbox.x + '-' + img.face_bbox.y) {
    <div
      class="merge-photo-cell group"
      [attr.data-id]="img.image_id"
      (click)="onClick(img)"
    >
      @if (thumbUrl(img)) {
        <img
          class="merge-photo-thumb"
          [src]="thumbUrl(img)!"
          [alt]="img.path"
          loading="lazy"
          decoding="async"
          [style.object-position]="focus(img).x + ' ' + focus(img).y"
        />
      } @else {
        <div class="merge-photo-placeholder">
          <lucide-icon name="image" size="24" class="merge-photo-placeholder-icon"></lucide-icon>
        </div>
      }
    </div>
  }
</div>
```

- [ ] **Step 3: Write styles**

```css
.merge-photo-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(clamp(140px, 25vw, 220px), 1fr));
  gap: 0.5rem;
  overflow-y: auto;
}

.merge-photo-cell {
  aspect-ratio: 1 / 1;
  overflow: hidden;
  border-radius: 0.5rem;
  background: hsl(var(--muted));
  cursor: pointer;
  position: relative;
}

.merge-photo-thumb {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.merge-photo-placeholder {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

.merge-photo-placeholder-icon {
  color: hsl(var(--muted-foreground));
}
```

- [ ] **Step 4: Write component tests**

```ts
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { MergePhotoGridComponent } from './merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectPhotoFace } from '../../models/models';
import { Subject as RxSubject } from 'rxjs';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const makePhoto = (
  imageId: number,
  x: number,
  y: number,
  w: number,
  h: number
): SubjectPhotoFace => ({
  image_id: imageId,
  path: `/img/${imageId}.jpg`,
  thumbnail_path: `/thumb/${imageId}.jpg`,
  preview_path: null,
  date_taken: null,
  mtime: 0,
  face_bbox: { x, y, w, h },
});

describe('MergePhotoGridComponent', () => {
  let component: MergePhotoGridComponent;
  let fixture: ComponentFixture<MergePhotoGridComponent>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [MergePhotoGridComponent],
      providers: [
        PhotoService,
        {
          provide: PhotoService,
          useValue: {
            thumbnailUrl: (p: string | null) => p,
            prioritizePreviews: vi.fn().mockResolvedValue(undefined),
            openLightbox: vi.fn(),
          },
        },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(MergePhotoGridComponent);
    component = fixture.componentInstance;
  });

  it('creates', () => {
    expect(component).toBeTruthy();
  });

  it('computes object-position from face bbox center', () => {
    const img = makePhoto(1, 0.25, 0.25, 0.5, 0.5);
    expect(component.focus(img)).toEqual({ x: '50%', y: '50%' });
  });

  it('clamps object-position to 0..100', () => {
    const img = makePhoto(2, -0.1, 1.2, 0.5, 0.5);
    expect(component.focus(img)).toEqual({ x: '15%', y: '100%' });
  });
});
```

- [ ] **Step 5: Commit**

```bash
git add src/app/components/merge-photo-grid/
git commit -m "feat(merge-photo-grid): add face-centered responsive grid component"
```

---

### Task 6: Update merge-review component

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`
- Modify: `src/app/components/merge-review/merge-review.component.html`
- Modify: `src/app/components/merge-review/merge-review.component.css`

- [ ] **Step 1: Update component TS**

In `src/app/components/merge-review/merge-review.component.ts`:

- Replace `SearchResult` import with `SubjectPhotoFace`.
- Replace `PhotoGridComponent` import with `MergePhotoGridComponent`.
- Change `photosA` and `photosB` signals from `Signal<SearchResult[]>` to `Signal<SubjectPhotoFace[]>`.
- Update `loadPhotos` to call `getSubjectPhotosWithFaces` instead of `getSubjectPhotos`.

Relevant changed sections:

```ts
import { MergeSuggestion, SubjectPhotoFace, Subject } from '../../models/models';
import { MergePhotoGridComponent } from '../merge-photo-grid/merge-photo-grid.component';

@Component({
  ...
  imports: [CommonModule, MergePhotoGridComponent, CdkTrapFocus],
  ...
})
export class MergeReviewComponent {
  photosA = signal<SubjectPhotoFace[]>([]);
  photosB = signal<SubjectPhotoFace[]>([]);

  private async loadPhotos(value: MergeSuggestion | null) {
    ...
    const [photosA, photosB] = await Promise.all([
      this.photoService.getSubjectPhotosWithFaces(value.subject_a.id),
      this.photoService.getSubjectPhotosWithFaces(value.subject_b.id),
    ]);
    ...
  }
}
```

- [ ] **Step 2: Update component template**

In `src/app/components/merge-review/merge-review.component.html`:

- Replace both `app-photo-grid` usages with `app-merge-photo-grid`:

```html
<app-merge-photo-grid class="subject-photos" [images]="photosA()" />
```

- Update the count label from "photos" to "faces":

```html
<div class="photo-count text-xs text-muted-foreground mb-2">{{ photosA().length }} faces</div>
```

```html
<div class="photo-count text-xs text-muted-foreground mb-2">{{ photosB().length }} faces</div>
```

- [ ] **Step 3: Tweak styles for scroll area**

In `src/app/components/merge-review/merge-review.component.css`, ensure `.subject-photos` keeps its `flex: 1; min-height: 0; overflow-y: auto;` so the new grid scrolls inside the column.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/merge-review/
git commit -m "feat(merge-review): use face-centered grid and face counts"
```

---

### Task 7: Update merge-review tests

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.spec.ts`

- [ ] **Step 1: Update imports and helpers**

Replace `SearchResult` import with `SubjectPhotoFace`.

Replace `makePhoto` helper:

```ts
const makePhoto = (id: number, x = 0.5, y = 0.5, w = 0.3, h = 0.3): SubjectPhotoFace => ({
  image_id: id,
  path: `/img/${id}.jpg`,
  thumbnail_path: `/thumb/${id}.jpg`,
  preview_path: null,
  date_taken: null,
  mtime: 0,
  face_bbox: { x, y, w, h },
});
```

- [ ] **Step 2: Replace `getSubjectPhotos` with `getSubjectPhotosWithFaces`**

In the tests that spy on `photoService.getSubjectPhotos`, switch the spy to `photoService.getSubjectPhotosWithFaces` and update assertions.

For example, in the first test:

```ts
vi.spyOn(photoService, 'getSubjectPhotosWithFaces')
  .mockImplementation(async (id: number) =>
    id === 1 ? [makePhoto(10)] : [makePhoto(20)]
  );

expect(photoService.getSubjectPhotosWithFaces).toHaveBeenCalledWith(1);
expect(photoService.getSubjectPhotosWithFaces).toHaveBeenCalledWith(2);
```

Apply the same replacement to all other tests that mock `getSubjectPhotos`.

- [ ] **Step 3: Run tests**

```bash
npm test -- src/app/components/merge-review/merge-review.component.spec.ts
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "test(merge-review): update tests for SubjectPhotoFace data shape"
```

---

### Task 8: Verify backend compiles

**Files:**
- Modify: none

- [ ] **Step 1: Run cargo check**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: no errors.

- [ ] **Step 2: Commit if any fixes were needed**

```bash
git diff --stat
# if changes exist:
git add -A
git commit -m "fix(backend): resolve compilation issues for SubjectPhotoFace"
```

---

### Task 9: Run full frontend test suite

**Files:**
- Modify: none

- [ ] **Step 1: Run all tests**

```bash
npm test
```

Expected: all tests pass.

- [ ] **Step 2: Commit if fixes were needed**

```bash
git diff --stat
# if changes exist:
git add -A
git commit -m "fix(frontend): resolve test failures after merge modal changes"
```

---

## Spec coverage self-check

| Spec requirement | Task |
|---|---|
| Add `SubjectPhotoFace` and `FaceBBox` backend types | Task 1 |
| Add `get_subject_photos_with_faces` command | Task 3 |
| Flattened rows, one per face, ordered by date | Task 2 |
| Add frontend `SubjectPhotoFace` model | Task 4 |
| Add `PhotoService.getSubjectPhotosWithFaces` | Task 4 |
| Create responsive face-centered grid | Task 5 |
| Replace grid in merge modal | Task 6 |
| Update count label to "faces" | Task 6 |
| Keep internal scroll and pinned actions | Task 6 |
| Tests for bbox math and merge-review | Tasks 5, 7 |

## Placeholder scan

No placeholders. Every step includes exact code, file paths, and commands.
