# Merge Review: Per-Face Removal (X-Mark)

## Problem

When merging a named subject with an unnamed subject in the People view, the
unnamed subject frequently has a handful of mismatched faces mixed in (faces
that belong to someone else, misclustered). Today the merge-review modal
(`MergeReviewComponent`) is all-or-nothing: it shows every face in both
subjects and merging folds *all* of the source subject's faces into the
target. There's no way to weed out the mismatched faces before confirming.

## Goal

Let the user click an X on a mismatched face's crop, in the merge-review
modal, to immediately detach that face from the subject being discarded —
before confirming the merge — so the merge that follows is clean.

## Scope

- Applies only inside the merge-review modal (`MergeReviewComponent`), not to
  the general subject-detail face grid or `face-picker`.
- Applies only to the **source** subject's grid — the subject that will be
  discarded/folded away, per the existing `mergeTarget` getter
  (`merge-review.component.ts:60-71`, which already prefers the named subject
  as target when merging a named + unnamed pair). The target/kept subject's
  grid never shows the X — it can't be affected by this feature.
- No new backend work. Reuses the existing `unassign_face` Tauri command
  (`src-tauri/src/people/commands.rs:254-278`), which already:
  - unassigns the face (`subject_id = NULL`)
  - writes `cannot_link` constraints against the rest of its old subject's
    faces, so clustering won't immediately re-merge it
  - reassigns thumbnails if needed
  - deletes the source subject if it has zero faces left (see Guardrail below
    for why this path is prevented from firing during this flow)
  - recomputes merge suggestions

## UX Behavior

1. User opens merge review for a named + unnamed subject pair (or any pair —
   the rule is symmetric on source/target, not literally "unnamed").
2. Hovering a face tile in the **source** grid reveals an X badge, top-right
   of the tile (same visual language as the existing star badge in
   `face-picker.component.html:34-46`).
3. Clicking the X immediately unassigns that face via `unassign_face`. On
   success the tile is removed from the grid and the "N faces" count above
   the grid updates (it's already bound to the `photosA`/`photosB` signal
   length).
4. Clicking elsewhere on a tile still opens the lightbox — the X button uses
   `stopPropagation()` so the two behaviors don't conflict.
5. **Guardrail:** once a source grid is down to 1 face, its X badge is shown
   in a disabled/greyed state (not hidden) — no more removals from that grid
   are possible. This exists because `unassign_face` auto-deletes a subject
   once its last face is removed; letting a subject disappear mid-review
   would break the modal (it holds direct references to `subject_a`/
   `subject_b`). The target grid never has an X at all, so it can't hit this
   path either.
6. If the `unassign_face` call fails, log via `console.error` (matching the
   existing convention in this codebase — there is no toast/notification
   system) and leave the tile in place so nothing is silently lost.

## Implementation

### `MergePhotoGridComponent` (`src/app/components/merge-photo-grid/`)

- New `@Input() removable = false`.
- New `@Output() removed = new EventEmitter<number>()` — emits the removed
  face's `face_id` on success.
- New internal `removingIds = signal<Set<number>>(new Set())` to disable a
  tile mid-request and block double-clicks.
- Template: when `removable` is true, render a badge per cell following the
  `face-picker` hover-badge pattern:
  - `absolute top-2 right-2 p-1.5 rounded-full bg-background/80
    text-muted-foreground opacity-0 group-hover:opacity-100
    transition-opacity` (cell already has the `.group` class)
  - Enabled hover state: `hover:bg-destructive hover:text-destructive-foreground`
  - `x` lucide icon, size 14
  - `aria-label="Remove face from subject"`
  - Disabled when `images.length <= 1` or the face id is in `removingIds`:
    reduced opacity, `cursor-not-allowed`, no hover state, click is a no-op
- Click handler (`onRemove(event, img)`):
  1. `event.stopPropagation()`
  2. bail if disabled/in-flight
  3. add to `removingIds`
  4. `await this.photos.unassignFace(img.face_id)`
  5. on success: emit `removed.emit(img.face_id)`
  6. on failure: `console.error(...)`
  7. `finally`: remove from `removingIds`

### `MergeReviewComponent` (`src/app/components/merge-review/`)

- Two derived booleans (getters or inline template expressions) comparing
  `mergeTarget?.source?.id` against `suggestion.subject_a.id` /
  `suggestion.subject_b.id`.
- Pass `[removable]="<derived boolean>"` to each `app-merge-photo-grid`.
- Wire `(removed)="$event"` on each grid to filter the removed `face_id` out
  of the corresponding `photosA`/`photosB` signal:
  ```ts
  photosA.update(list => list.filter(f => f.face_id !== faceId));
  ```
- No changes to `confirm()`, `dismiss()`, `close()`, or the merge animation —
  they already operate on `mergeTarget.target`/`mergeTarget.source` ids, which
  are unaffected by removing individual faces.

### Backend

No changes.

## Out of Scope

- Multi-select / bulk removal.
- Reassigning a removed face directly to a different subject from this modal
  (existing `FaceAssignPopoverComponent` flow in the lightbox already covers
  reassignment elsewhere).
- Removal from the target subject's grid.
- Removal from the general subject-detail or `face-picker` grids.
