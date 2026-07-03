# Merge Review Face Removal (X-Mark) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the merge-review modal, let the user hover a face-crop tile on the subject that will be discarded and click an X to immediately unassign that face, so mismatched faces can be weeded out before confirming a merge.

**Architecture:** Two Angular component changes, no backend changes. `MergePhotoGridComponent` gains an opt-in `removable` input and a hover-revealed remove badge per cell that calls the existing `unassign_face` command; `MergeReviewComponent` enables that input only on the grid for the merge's "source" subject (the one being discarded) and prunes its local `photosA`/`photosB` signal when a face is removed.

**Tech Stack:** Angular (standalone components, signals, `@Input`/`@Output` decorators), Tailwind CSS utility classes, `lucide-angular` icons, Vitest + `@angular/core/testing` (TestBed) for tests.

## Global Constraints

- No backend changes — reuse `unassign_face` (`src-tauri/src/people/commands.rs:254-278`) via the existing `PhotoService.unassignFace(faceId)` (`src/app/services/photo.service.ts:393-395`) exactly as-is.
- The remove badge is disabled (visible but non-interactive) once a grid has 1 image or fewer — a subject must never be emptied out via this UI, because `unassign_face` auto-deletes a subject with zero faces and `MergeReviewComponent` holds direct references to `subject_a`/`subject_b`.
- The remove badge only ever appears on the merge's **source** grid (the subject being discarded into the other), never on the target/kept grid.
- Errors from `unassignFace` are logged with `console.error` and otherwise swallowed — this codebase has no toast/notification system (confirmed: no `ToastService`/`NotificationService` anywhere in `src/app`), and `MergeReviewComponent.confirm()`/`dismiss()` already follow this same try/catch/console.error convention.
- Follow existing code style exactly: `@Input`/`@Output` decorators (not the `input()`/`output()` functional API — `MergePhotoGridComponent` and `MergeReviewComponent` both already use decorators), Tailwind utility classes inline in templates (no new CSS files), `signal()` for internal reactive state.

---

## File Structure

- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.ts` — add `removable` input, `removed` output, `removingIds` state, `onRemove()` handler.
- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.html` — add the hover-revealed remove badge per cell.
- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts` — tests for the above.
- Modify: `src/app/components/merge-review/merge-review.component.html` — bind `[removable]` and `(removed)` on each `app-merge-photo-grid`.
- Modify: `src/app/components/merge-review/merge-review.component.ts` — add `onFaceRemovedA()`/`onFaceRemovedB()` handlers.
- Modify: `src/app/components/merge-review/merge-review.component.spec.ts` — tests for the above.

---

### Task 1: Add removable remove-badge to `MergePhotoGridComponent`

**Files:**
- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`
- Modify: `src/app/components/merge-photo-grid/merge-photo-grid.component.html`
- Test: `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`

**Interfaces:**
- Consumes: `PhotoService.unassignFace(faceId: number): Promise<void>` (`src/app/services/photo.service.ts:393-395`, already exists, no changes).
- Produces (for Task 2): `@Input() removable: boolean` (default `false`) and `@Output() removed: EventEmitter<number>` (emits the removed `face_id` on success) on `MergePhotoGridComponent`.

- [ ] **Step 1: Write the failing tests**

Open `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts` and replace its full contents with:

```ts
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { MergePhotoGridComponent } from './merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectPhotoFace } from '../../models/models';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const makePhoto = (faceId: number, imageId: number): SubjectPhotoFace => ({
  face_id: faceId,
  image_id: imageId,
  path: `/img/${imageId}.jpg`,
  thumbnail_path: `/thumb/${imageId}.jpg`,
  preview_path: null,
  date_taken: null,
  mtime: 0,
  face_bbox: { x: 0, y: 0, w: 0.5, h: 0.5 },
});

describe('MergePhotoGridComponent', () => {
  let component: MergePhotoGridComponent;
  let fixture: ComponentFixture<MergePhotoGridComponent>;
  let getFaceCrop: ReturnType<typeof vi.fn>;
  let unassignFace: ReturnType<typeof vi.fn>;
  let openLightbox: ReturnType<typeof vi.fn>;

  beforeEach(async () => {
    getFaceCrop = vi.fn((faceId: number) => Promise.resolve(`/crops/${faceId}.webp`));
    unassignFace = vi.fn().mockResolvedValue(undefined);
    openLightbox = vi.fn();

    await TestBed.configureTestingModule({
      imports: [MergePhotoGridComponent],
      providers: [
        {
          provide: PhotoService,
          useValue: {
            getFaceCrop,
            thumbnailUrl: (p: string | null) => p,
            openLightbox,
            unassignFace,
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

  it('loads the real face crop for a face and exposes its URL', async () => {
    const img = makePhoto(7, 1);
    component.images = [img];

    // No crop resolved yet.
    expect(component['cropUrl'](img)).toBeNull();

    await component['loadCrop'](7);

    expect(getFaceCrop).toHaveBeenCalledWith(7);
    expect(component['cropUrl'](img)).toBe('/crops/7.webp');
  });

  it('does not refetch a crop that is already cached', async () => {
    await component['loadCrop'](7);
    await component['loadCrop'](7);
    expect(getFaceCrop).toHaveBeenCalledTimes(1);
  });

  it('opens the lightbox with the full mapped list when a cell is clicked', () => {
    component.images = [makePhoto(1, 100), makePhoto(2, 200), makePhoto(3, 300)];

    // Click the middle cell.
    (component as unknown as { onClick: (p: SubjectPhotoFace) => void }).onClick(component.images[1]);

    expect(openLightbox).toHaveBeenCalledTimes(1);
    const [clicked, list] = openLightbox.mock.calls[0];
    expect(clicked.image_id).toBe(200);
    expect(list.map((i: { image_id: number }) => i.image_id)).toEqual([100, 200, 300]);
  });

  it('does not render a remove badge when removable is false (default)', () => {
    component.images = [makePhoto(1, 100)];
    fixture.detectChanges();

    const badge = fixture.debugElement.query(By.css('button[aria-label="Remove face from subject"]'));
    expect(badge).toBeNull();
  });

  it('renders one remove badge per cell when removable is true', () => {
    component.images = [makePhoto(1, 100), makePhoto(2, 200)];
    component.removable = true;
    fixture.detectChanges();

    const badges = fixture.debugElement.queryAll(By.css('button[aria-label="Remove face from subject"]'));
    expect(badges.length).toBe(2);
  });

  it('clicking the remove badge unassigns the face, emits removed, and does not open the lightbox', async () => {
    component.images = [makePhoto(1, 100), makePhoto(2, 200)];
    component.removable = true;
    fixture.detectChanges();

    const removedSpy = vi.fn();
    component.removed.subscribe(removedSpy);

    const badge = fixture.debugElement.queryAll(By.css('button[aria-label="Remove face from subject"]'))[0];
    badge.nativeElement.click();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(unassignFace).toHaveBeenCalledWith(1);
    expect(removedSpy).toHaveBeenCalledWith(1);
    expect(openLightbox).not.toHaveBeenCalled();
  });

  it('disables the remove badge when only one face remains', () => {
    component.images = [makePhoto(1, 100)];
    component.removable = true;
    fixture.detectChanges();

    const badge = fixture.debugElement.query(By.css('button[aria-label="Remove face from subject"]'));
    expect(badge.nativeElement.disabled).toBe(true);
  });

  it('logs an error and does not emit removed when unassignFace fails', async () => {
    unassignFace.mockRejectedValueOnce(new Error('db error'));
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    component.images = [makePhoto(1, 100), makePhoto(2, 200)];
    component.removable = true;
    fixture.detectChanges();

    const removedSpy = vi.fn();
    component.removed.subscribe(removedSpy);

    const badge = fixture.debugElement.queryAll(By.css('button[aria-label="Remove face from subject"]'))[0];
    badge.nativeElement.click();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(errorSpy).toHaveBeenCalled();
    expect(removedSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });
});
```

- [ ] **Step 2: Run the tests to verify the new ones fail**

Run: `npx vitest run src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`
Expected: The three pre-existing tests (`creates`, crop-loading, lightbox) still PASS. The five new tests FAIL — either a TypeScript error (`Property 'removable' does not exist on type 'MergePhotoGridComponent'` / `Property 'removed' does not exist`) or, if it compiles, the badge queries return `null`/empty because no badge markup exists yet.

- [ ] **Step 3: Implement `removable`/`removed`/`removingIds` on the component**

Open `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`. Update the import block at the top (add `Output`, `EventEmitter`):

```ts
import {
  Component,
  Input,
  Output,
  EventEmitter,
  ChangeDetectionStrategy,
  inject,
  ElementRef,
  AfterViewInit,
  OnDestroy,
  OnChanges,
  SimpleChanges,
  signal,
} from '@angular/core';
```

Then, inside the `MergePhotoGridComponent` class, add these members right after the existing `@Input() images: SubjectPhotoFace[] = [];` line:

```ts
  @Input() removable = false;
  @Output() removed = new EventEmitter<number>();

  /** Face ids with an in-flight unassign request, so the badge can't be double-clicked. */
  protected removingIds = signal<Set<number>>(new Set());
```

Then add this method near `onClick` (e.g., right after it):

```ts
  protected async onRemove(event: MouseEvent, img: SubjectPhotoFace): Promise<void> {
    event.stopPropagation();
    if (this.images.length <= 1 || this.removingIds().has(img.face_id)) return;

    this.removingIds.update((ids) => new Set(ids).add(img.face_id));
    try {
      await this.photos.unassignFace(img.face_id);
      this.removed.emit(img.face_id);
    } catch (e) {
      console.error(`MergePhotoGrid: failed to remove face ${img.face_id}`, e);
    } finally {
      this.removingIds.update((ids) => {
        const next = new Set(ids);
        next.delete(img.face_id);
        return next;
      });
    }
  }
```

- [ ] **Step 4: Add the remove badge to the template**

Open `src/app/components/merge-photo-grid/merge-photo-grid.component.html` and replace its contents with:

```html
<div class="merge-photo-grid">
  @for (img of images; track img.face_id) {
    @let crop = cropUrl(img);
    <div
      class="merge-photo-cell group"
      [attr.data-id]="img.face_id"
      (click)="onClick(img)"
    >
      @if (crop) {
        <img
          class="merge-photo-thumb"
          [src]="crop"
          [alt]="img.path"
          loading="lazy"
          decoding="async"
        />
      } @else {
        <div class="merge-photo-placeholder">
          <lucide-icon name="image" size="24" class="merge-photo-placeholder-icon"></lucide-icon>
        </div>
      }
      @if (removable) {
        <button
          type="button"
          class="absolute top-2 right-2 p-1.5 rounded-full bg-background/80 text-muted-foreground opacity-0 group-hover:opacity-100 transition-opacity disabled:opacity-40 disabled:cursor-not-allowed enabled:hover:bg-destructive enabled:hover:text-white enabled:hover:opacity-100"
          [disabled]="images.length <= 1 || removingIds().has(img.face_id)"
          aria-label="Remove face from subject"
          title="Remove face from subject"
          (click)="onRemove($event, img)"
        >
          <lucide-icon name="x" [size]="14"></lucide-icon>
        </button>
      }
    </div>
  }
</div>
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `npx vitest run src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts`
Expected: All 8 tests PASS.

- [ ] **Step 6: Typecheck and lint**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: No errors.

- [ ] **Step 7: Commit**

```bash
git add src/app/components/merge-photo-grid/merge-photo-grid.component.ts src/app/components/merge-photo-grid/merge-photo-grid.component.html src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts
git commit -m "feat(merge-photo-grid): add opt-in hover remove badge per face tile"
```

---

### Task 2: Wire the remove badge into `MergeReviewComponent`'s source-subject grid

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.html`
- Modify: `src/app/components/merge-review/merge-review.component.ts`
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`

**Interfaces:**
- Consumes: `MergePhotoGridComponent.removable: boolean` input and `removed: EventEmitter<number>` output, both from Task 1.
- Consumes: existing `MergeReviewComponent.mergeTarget` getter (`merge-review.component.ts:60-71`), which returns `{ target: Subject; source: Subject } | null`.
- Consumes: existing `photosA`/`photosB` signals (`merge-review.component.ts:55-56`).

- [ ] **Step 1: Write the failing tests**

Open `src/app/components/merge-review/merge-review.component.spec.ts`. Add `MergePhotoGridComponent` to the imports at the top:

```ts
import { MergePhotoGridComponent } from '../merge-photo-grid/merge-photo-grid.component';
```

(Full import block becomes:)

```ts
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { MergeReviewComponent } from './merge-review.component';
import { MergePhotoGridComponent } from '../merge-photo-grid/merge-photo-grid.component';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { MergeSuggestion, SubjectPhotoFace, Subject } from '../../models/models';
import { Subject as RxSubject } from 'rxjs';
```

Then add these three tests inside the existing `describe('MergeReviewComponent', ...)` block, after the `'mergeTarget returns lower id as target when both named'` test:

```ts
  it('marks only the source subject\'s grid as removable', async () => {
    const subA = makeSubject(1, 'Alice'); // named -> target
    const subB = makeSubject(2, null);    // unnamed -> source
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);

    component.suggestion = makeSuggestion(subA, subB);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    const grids = fixture.debugElement.queryAll(By.directive(MergePhotoGridComponent));
    expect(grids.length).toBe(2);
    expect((grids[0].componentInstance as MergePhotoGridComponent).removable).toBe(false); // col A = subject_a = target
    expect((grids[1].componentInstance as MergePhotoGridComponent).removable).toBe(true);  // col B = subject_b = source
  });

  it('removing a face from grid A filters it out of photosA', async () => {
    const subA = makeSubject(1, null);    // unnamed -> source
    const subB = makeSubject(2, 'Bob');   // named -> target
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) =>
      id === 1 ? [makePhoto(10), makePhoto(20)] : []
    );

    component.suggestion = makeSuggestion(subA, subB);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    const grids = fixture.debugElement.queryAll(By.directive(MergePhotoGridComponent));
    (grids[0].componentInstance as MergePhotoGridComponent).removed.emit(10);

    expect(component.photosA().map(f => f.face_id)).toEqual([20]);
  });

  it('removing a face from grid B filters it out of photosB', async () => {
    const subA = makeSubject(1, 'Alice'); // named -> target
    const subB = makeSubject(2, null);    // unnamed -> source
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) =>
      id === 2 ? [makePhoto(30), makePhoto(40)] : []
    );

    component.suggestion = makeSuggestion(subA, subB);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    const grids = fixture.debugElement.queryAll(By.directive(MergePhotoGridComponent));
    (grids[1].componentInstance as MergePhotoGridComponent).removed.emit(30);

    expect(component.photosB().map(f => f.face_id)).toEqual([40]);
  });
```

- [ ] **Step 2: Run the tests to verify the new ones fail**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: Pre-existing tests still PASS. The 3 new tests FAIL — `grids[0].componentInstance.removable` is `undefined`/`false` for both columns (no binding wired yet), and emitting `removed` on the child does nothing because `merge-review.component.html` has no `(removed)` listener yet (calling `.emit()` on an `EventEmitter` with no template listener is harmless, so this test fails on the `expect(component.photosA()...)` assertion, not with a runtime error).

- [ ] **Step 3: Add the handler methods**

Open `src/app/components/merge-review/merge-review.component.ts`. Add these two methods right after the `mergeTarget` getter (after line 71, before `private async loadPhotos`):

```ts
  protected onFaceRemovedA(faceId: number): void {
    this.photosA.update((list) => list.filter((f) => f.face_id !== faceId));
  }

  protected onFaceRemovedB(faceId: number): void {
    this.photosB.update((list) => list.filter((f) => f.face_id !== faceId));
  }
```

- [ ] **Step 4: Wire the template bindings**

Open `src/app/components/merge-review/merge-review.component.html`. Replace the line:

```html
            <app-merge-photo-grid class="subject-photos" [images]="photosA()" />
```

with:

```html
            <app-merge-photo-grid
              class="subject-photos"
              [images]="photosA()"
              [removable]="mergeTarget?.source?.id === suggestion.subject_a.id"
              (removed)="onFaceRemovedA($event)"
            />
```

And replace the line:

```html
            <app-merge-photo-grid class="subject-photos" [images]="photosB()" />
```

with:

```html
            <app-merge-photo-grid
              class="subject-photos"
              [images]="photosB()"
              [removable]="mergeTarget?.source?.id === suggestion.subject_b.id"
              (removed)="onFaceRemovedB($event)"
            />
```

(This mirrors the existing `mergeTarget?.target?.id === suggestion.subject_a.id` pattern already used a few lines above for the `keep-badge`, so `suggestion` is already known non-null inside this `@if (suggestion)` block.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: All tests PASS (8 pre-existing + 3 new = 11).

- [ ] **Step 6: Typecheck**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: No errors.

- [ ] **Step 7: Run the full frontend test suite**

Run: `npx vitest run`
Expected: All tests PASS, no regressions elsewhere.

- [ ] **Step 8: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): show remove badge only on the source subject's grid"
```

---

## Manual Verification (after both tasks)

Since this touches the merge-review modal's interactive UI, verify by hand after both tasks are committed:

1. Run the app (`npm run tauri dev` or the project's existing dev-run command).
2. Get an unnamed subject with a couple of mismatched faces clustered into it (or synthetically create one by unassigning faces from two different named subjects and letting clustering group them).
3. Trigger a merge-suggestion review (or navigate to a subject and use its "Similar Subjects" section, whichever surfaces `MergeReviewComponent`) for a named + unnamed pair.
4. Hover a face tile on the unnamed (source) side — confirm the X fades in top-right; hover a tile on the named (target) side — confirm no X appears.
5. Click X on a mismatched face — confirm the tile disappears immediately and the "N faces" count decrements.
6. Remove faces down to 1 remaining on the source side — confirm the X badge is now greyed out and unresponsive to clicks.
7. Confirm the merge — verify only the remaining (correct) faces moved into the named subject, and the previously removed face is now unassigned (visible as unclustered / re-groupable elsewhere in the People view), not deleted.
