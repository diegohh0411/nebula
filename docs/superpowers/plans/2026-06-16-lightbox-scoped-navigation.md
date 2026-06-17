# Lightbox Scoped Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the lightbox carry its own ordered source list so prev/next arrows navigate within whatever scoped view it was opened from (main gallery, subject detail, merge modal).

**Architecture:** A single `lightboxItems` signal in `PhotoService` holds the exact ordered list the user is browsing. It is set at open, read by navigate, and cleared on close — there is no global-state fallback. Every opener supplies its list; a `required` input on `photo-grid` makes that a compile-time guarantee.

**Tech Stack:** Angular (standalone components, signals), TypeScript, Vitest + `@angular/core/testing`. Test runner: `npm test` (`vitest run`).

---

## Spec

See `docs/superpowers/specs/2026-06-16-lightbox-scoped-navigation-design.md`.

## File Structure

- `src/app/services/photo.service.ts` — add `lightboxItems` signal + `galleryImages` computed; rework `openLightbox`/`navigateLightbox`/`closeLightbox`.
- `src/app/services/photo.service.spec.ts` — service nav unit tests.
- `src/app/components/photo-grid/photo-grid.component.ts` — required `navigationItems` input; forward it on click.
- `src/app/components/gallery/gallery.component.html` — bind `navigationItems`.
- `src/app/components/subject-detail/subject-detail.component.html` — bind `navigationItems`.
- `src/app/components/merge-photo-grid/merge-photo-grid.component.ts` — map full list; pass on click.
- `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts` — extend with nav test.

**Compile-order note:** Task 1 changes `openLightbox` to a required 2-argument signature, which breaks both existing callers. Task 1 therefore lands the service change *and* updates every caller + template binding in the same task, so the build is green at the Task 1 commit. Tasks 2 and 3 then refine and test the merge-grid path and verify end-to-end.

---

## Task 1: Service source list + wire all openers

**Files:**
- Modify: `src/app/services/photo.service.ts` (lightbox block ~lines 57–101, `dayGroups` ~104–117)
- Modify: `src/app/services/photo.service.spec.ts`
- Modify: `src/app/components/photo-grid/photo-grid.component.ts:93-102`
- Modify: `src/app/components/gallery/gallery.component.html:50`
- Modify: `src/app/components/subject-detail/subject-detail.component.html:140`
- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.ts:92-104`

- [ ] **Step 1: Write the failing service tests**

Append a new `describe` block to `src/app/services/photo.service.spec.ts`. It uses the same `vi.mock('@tauri-apps/api/core', ...)` already at the top of the file, so no new mock is needed.

```typescript
describe('PhotoService — lightbox navigation', () => {
  let service: PhotoService;

  const img = (id: number): Image => ({
    id,
    folder_id: 1,
    path: `/img/${id}.jpg`,
    file_hash: '',
    hash_status: 'ok',
    date_taken: null,
    mtime: 0,
    thumbnail_path: null,
    preview_path: null,
    semantic_analysis_done: true,
    subject_analysis_done: true,
    added_at: 0,
    updated_at: 0,
    deleted_at: null,
  });

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$: new Subject(),
            imageAdded$: new Subject(),
            imageUpdated$: new Subject(),
            imageRemoved$: new Subject(),
            modelDownloadProgress$: new Subject(),
          },
        },
      ],
    });
    service = TestBed.inject(PhotoService);
  });

  it('openLightbox stores the image and its source list', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[1], items);
    expect(service.selectedImage()).toBe(items[1]);
    expect(service.lightboxItems()).toBe(items);
  });

  it('navigateLightbox moves forward within the supplied list', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[0], items);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(2);
  });

  it('navigateLightbox wraps from last to first', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[2], items);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(1);
  });

  it('navigateLightbox wraps from first to last going backward', () => {
    const items = [img(1), img(2), img(3)];
    service.openLightbox(items[0], items);
    service.navigateLightbox(-1);
    expect((service.selectedImage() as Image).id).toBe(3);
  });

  it('navigateLightbox is a no-op when the source list is empty', () => {
    service.openLightbox(img(1), []);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(1);
  });

  it('navigateLightbox is a no-op when the current image is not in the list', () => {
    const items = [img(1), img(2)];
    service.openLightbox(img(99), items);
    service.navigateLightbox(1);
    expect((service.selectedImage() as Image).id).toBe(99);
  });

  it('closeLightbox clears the source list', () => {
    const items = [img(1), img(2)];
    service.openLightbox(items[0], items);
    service.closeLightbox();
    expect(service.selectedImage()).toBeNull();
    expect(service.lightboxItems()).toEqual([]);
  });

  it('galleryImages flattens dayGroups in visual order (search results)', () => {
    const results: SearchResult[] = [
      { image_id: 10, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: null, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
      { image_id: 11, path: '', thumbnail_path: null, preview_path: null, score: 1, date_taken: null, mtime: 0, semantic_analysis_done: true, subject_analysis_done: true },
    ];
    service.searchResults.set(results);
    expect(service.galleryImages().map((i) => ('id' in i ? i.id : i.image_id))).toEqual([10, 11]);
  });
});
```

Add `Image` and `SearchResult` to the existing model import at the top of the spec:

```typescript
import { ImageUpdatedEvent, PipelineStats, Image, SearchResult } from '../models/models';
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npm test -- src/app/services/photo.service.spec.ts`
Expected: FAIL — `openLightbox` currently takes one argument and `lightboxItems`/`galleryImages` do not exist (TypeScript / assertion errors).

- [ ] **Step 3: Add the `lightboxItems` signal**

In `src/app/services/photo.service.ts`, in the `// ---- Lightbox state ----` block (just after `selectedImage`, ~line 58), add:

```typescript
  readonly lightboxItems = signal<(Image | SearchResult)[]>([]);
```

- [ ] **Step 4: Rework `openLightbox`, `closeLightbox`, `navigateLightbox`**

Replace the existing `openLightbox` (lines ~77–80), `closeLightbox` (~82–85), and `navigateLightbox` (~87–101) with:

```typescript
  openLightbox(img: Image | SearchResult, items: (Image | SearchResult)[]): void {
    this.transitioningImageId.set('id' in img ? img.id : img.image_id);
    this.lightboxItems.set(items);
    this.selectedImage.set(img);
  }

  closeLightbox(): void {
    this.selectedImage.set(null);
    this.lightboxItems.set([]);
    // Note: transitioningImageId stays set during the transition back, then cleared in the component.
  }

  navigateLightbox(direction: number): void {
    const current = this.selectedImage();
    if (!current) return;

    const items = this.lightboxItems();
    if (items.length === 0) return;

    const currentId = 'id' in current ? current.id : current.image_id;
    const idx = items.findIndex((i) => ('id' in i ? i.id : i.image_id) === currentId);
    if (idx === -1) return;

    const nextIdx = (idx + direction + items.length) % items.length;
    const nextImg = items[nextIdx];
    this.selectedImage.set(nextImg);
    this.transitioningImageId.set('id' in nextImg ? nextImg.id : nextImg.image_id);
  }
```

- [ ] **Step 5: Add the `galleryImages` computed**

In `src/app/services/photo.service.ts`, immediately after the `dayGroups` computed (after line ~117), add:

```typescript
  /** Full visual-ordered list backing the main gallery / search lightbox. */
  readonly galleryImages = computed<(Image | SearchResult)[]>(() =>
    this.dayGroups().flatMap((g) => g.images)
  );
```

(`computed`, `Image`, and `SearchResult` are already imported in this file.)

- [ ] **Step 6: Update the `photo-grid` opener**

In `src/app/components/photo-grid/photo-grid.component.ts`, add the required input near the other `@Input()` declarations (top of the class):

```typescript
  @Input({ required: true }) navigationItems: (Image | SearchResult)[] = [];
```

Ensure `Input`, `Image`, and `SearchResult` are imported (the file already imports `Image`/`SearchResult` for its existing methods; add `Input` to the `@angular/core` import if missing).

Then change `onPhotoClick` (line ~100) from `this.photos.openLightbox(img);` to:

```typescript
      this.photos.openLightbox(img, this.navigationItems);
```

- [ ] **Step 7: Bind `navigationItems` in the gallery template**

In `src/app/components/gallery/gallery.component.html:50`, change:

```html
        <app-photo-grid [images]="row.images" [rowHeight]="row.rowHeight" />
```

to:

```html
        <app-photo-grid [images]="row.images" [rowHeight]="row.rowHeight" [navigationItems]="photos.galleryImages()" />
```

Confirm the gallery component exposes `photos` (it renders `photos.selectedImage()` at line 66, so `photos` is already a public member).

- [ ] **Step 8: Bind `navigationItems` in the subject-detail template**

In `src/app/components/subject-detail/subject-detail.component.html:140`, change:

```html
            <app-photo-grid [images]="row.images" [rowHeight]="row.rowHeight" />
```

to:

```html
            <app-photo-grid [images]="row.images" [rowHeight]="row.rowHeight" [navigationItems]="subjectPhotos()" />
```

(`subjectPhotos` is a `signal<SearchResult[]>` on the component and is already used in the template.)

- [ ] **Step 9: Update the merge-photo-grid opener (minimal, full refinement in Task 2)**

In `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`, replace `onClick` (lines ~92–104) so it passes a list. For now pass the single mapped image wrapped in an array so the build compiles; Task 2 replaces this with the full mapped list:

```typescript
  protected onClick(img: SubjectPhotoFace): void {
    const mapped = this.toLightboxImage(img);
    this.photos.openLightbox(mapped, [mapped]);
  }

  private toLightboxImage(img: SubjectPhotoFace): SearchResult {
    return {
      image_id: img.image_id,
      path: img.path,
      thumbnail_path: img.thumbnail_path,
      preview_path: img.preview_path,
      score: 0,
      date_taken: img.date_taken,
      mtime: img.mtime,
      semantic_analysis_done: true,
      subject_analysis_done: true,
    };
  }
```

Add `SearchResult` to the model import in this file:

```typescript
import { SubjectPhotoFace, SearchResult } from '../../models/models';
```

- [ ] **Step 10: Run the full test suite and type-check**

Run: `npm test`
Expected: PASS — all new service tests pass and existing suites (incl. `merge-photo-grid.component.spec.ts`, which mocks `openLightbox`) remain green.

Run: `npx tsc -p tsconfig.app.json --noEmit` (or `npm run build` if no dedicated typecheck script)
Expected: no type errors — every `openLightbox` caller now passes two arguments and every `<app-photo-grid>` binds `navigationItems`.

- [ ] **Step 11: Commit**

```bash
git add src/app/services/photo.service.ts src/app/services/photo.service.spec.ts \
  src/app/components/photo-grid/photo-grid.component.ts \
  src/app/components/gallery/gallery.component.html \
  src/app/components/subject-detail/subject-detail.component.html \
  src/app/components/merge-photo-grid/merge-photo-grid.component.ts
git commit -m "feat(lightbox): carry explicit source list for prev/next navigation (TT-80)"
```

---

## Task 2: Merge grid navigates across the full subject list

**Files:**
- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`
- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`

- [ ] **Step 1: Write the failing component test**

In `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`, the existing `PhotoService` mock provides `openLightbox: vi.fn()`. Add a test that clicking a cell opens the lightbox with the **full** mapped list, in `images` order.

Add inside the existing `describe('MergePhotoGridComponent', ...)` block:

```typescript
  it('opens the lightbox with the full mapped list when a cell is clicked', () => {
    const openLightbox = TestBed.inject(PhotoService).openLightbox as ReturnType<typeof vi.fn>;
    component.images = [makePhoto(1, 100), makePhoto(2, 200), makePhoto(3, 300)];

    // Click the middle cell.
    (component as unknown as { onClick: (p: SubjectPhotoFace) => void }).onClick(component.images[1]);

    expect(openLightbox).toHaveBeenCalledTimes(1);
    const [clicked, list] = openLightbox.mock.calls[0];
    expect(clicked.image_id).toBe(200);
    expect(list.map((i: { image_id: number }) => i.image_id)).toEqual([100, 200, 300]);
  });
```

If the existing `beforeEach` injects `PhotoService` via `TestBed`, `TestBed.inject(PhotoService)` returns the same mock object; confirm the mock is registered as a provider (it is — see the top of the file). If `component` is not already assigned in `beforeEach`, assign it from `fixture.componentInstance` as the other tests do.

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`
Expected: FAIL — `onClick` currently passes `[mapped]` (length 1), so `list` is `[200]`, not `[100, 200, 300]`.

- [ ] **Step 3: Implement the full-list mapping**

In `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`, replace `onClick` from Task 1 with a version that maps the whole `images` input and passes the clicked item plus the full ordered list:

```typescript
  protected onClick(img: SubjectPhotoFace): void {
    const list = this.images.map((i) => this.toLightboxImage(i));
    const clicked = list.find((i) => i.image_id === img.image_id) ?? this.toLightboxImage(img);
    this.photos.openLightbox(clicked, list);
  }
```

Keep the `toLightboxImage` helper added in Task 1 unchanged.

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/merge-photo-grid/merge-photo-grid.component.ts \
  src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts
git commit -m "feat(merge): lightbox cycles the full subject photo set (TT-80)"
```

---

## Task 3: Full regression + manual verification

**Files:** none (verification only)

- [ ] **Step 1: Run the entire test suite**

Run: `npm test`
Expected: PASS — all suites green.

- [ ] **Step 2: Type-check / build**

Run: `npx tsc -p tsconfig.app.json --noEmit` (or `npm run build`)
Expected: no errors.

- [ ] **Step 3: Manual verification (Tauri dev app)**

Launch the app (`npm run tauri dev` or the project's usual command) and confirm each acceptance criterion:

- Main gallery: open the lightbox on a photo, press ← / → → navigates across the gallery in on-screen (day-grouped) order; wraps at ends.
- Search results: run a text search, open the lightbox on a result, arrow through → cycles the search results.
- Subject detail: open a subject, click a photo, arrow → cycles that subject's photos only.
- Merge / Review Possible Duplicate modal: open the lightbox from a subject's grid, arrow → cycles that subject's photos; the other subject's grid (B) cycles its own set independently.

- [ ] **Step 4: Final commit (only if Step 3 surfaced fixes)**

If manual testing required no changes, there is nothing to commit. Otherwise commit the fix with a `fix(lightbox): ... (TT-80)` message.

---

## Self-Review

**Spec coverage:**
- `lightboxItems` signal + open/navigate/close rework → Task 1 Steps 3–4. ✓
- `galleryImages` computed → Task 1 Step 5. ✓
- `photo-grid` required input + forward → Task 1 Step 6. ✓
- gallery + subject-detail template bindings → Task 1 Steps 7–8. ✓
- merge-photo-grid full mapped list → Task 2. ✓
- Service tests (wrap both directions, empty no-op, missing no-op, close clears) → Task 1 Step 1. ✓
- merge-photo-grid spec extension → Task 2 Step 1. ✓
- Regression of search/gallery nav → Task 3 Step 3. ✓
- Edge cases (empty / not-found no-op) covered by guards + tests; collapsed rows & deletion explicitly out of scope per spec. ✓

**Placeholder scan:** No TBDs; every code step shows full code; commands have expected output.

**Type consistency:** `openLightbox(img, items)`, `lightboxItems`, `galleryImages`, `navigationItems`, and `toLightboxImage` are named identically across all tasks; `(Image | SearchResult)[]` used consistently; `SubjectPhotoFace` → `SearchResult` mapping matches `models.ts`.
