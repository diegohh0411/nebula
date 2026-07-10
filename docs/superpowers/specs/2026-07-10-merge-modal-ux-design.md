# Merge Modal UX Improvements — Design

**Date:** 2026-07-10
**Status:** Approved, ready for implementation planning

## Goal

Two cohesive UX improvements that route every subject-merge interaction through the
single `MergeReviewComponent` ("Review Possible Duplicate" modal), making the merge
flow consistent across the app:

1. **Inline name editing** of both subjects directly inside the merge modal.
2. **Replace the old similar-subjects merge flow** on the subject-detail page with the
   merge modal.

## Background — current state

- **The modal** is `MergeReviewComponent` (`app-merge-review`,
  `src/app/components/merge-review/`). Two columns (each subject's name, face count, and
  face grid), a "keep" badge on the surviving subject, and `Dismiss` / `Merge as X`
  actions. Subject names are currently **static text**. It is used by `people-view` for
  both merge suggestions and naming-conflict cases.
- **The old flow** lives inline in `subject-detail`'s "Similar Subjects" section: a
  compact list of rows, each with inline `Merge` / `Dismiss` buttons wired to
  `mergeSimilar()` / `dismissSimilar()` — no review modal.
- `EditableTextComponent` (`src/app/components/editable-text/`) already provides
  click-to-edit inline text (used in the subject-detail header) and is the building
  block for feature 1.
- Backend `name_subject` (`src-tauri/src/people/commands.rs:19`): checks whether
  **another** subject already has the exact name (case-insensitive, `COLLATE NOCASE`)
  and returns it as `duplicate_subject_id`, then **applies the rename unconditionally**
  and recalculates merge suggestions. It never blocks.
- `photoService.subjects` is a shared `signal<Subject[]>` both parent views keep loaded.
- `mergeSubjects(targetId, sourceId)` preserves **all** faces from both subjects; only
  the surviving name / thumbnail / id differ. No face data is ever lost by a merge.
- `dismissMergeSuggestion` (`dismiss_merge_suggestion`, repo.rs:579) does **not** just
  hide the suggestion — it also inserts a `cannot_link` constraint (`source='dismiss'`)
  between representative faces of the two subjects, suppressing future auto-merge
  suggestions. Dismissing is therefore an active "not the same person" feedback mark.

## Feature 1 — Inline name editing in `MergeReviewComponent`

### State model

Convert the modal from a static `@Input suggestion` + `mergeTarget` getter to a
reactive local model:

- When the `suggestion` input is set, seed two local signals `subjectA` / `subjectB`
  (copies of `suggestion.subject_a` / `subject_b`).
- Names render from these signals.
- `mergeTarget` becomes a `computed()` derived from `subjectA` / `subjectB` using the
  **existing** rule (named beats unnamed; both-named → lower id wins). The keep badge,
  the removable column, and the "Merge as X" label all follow reactively.

### Editing

Each column's name becomes an `<app-editable-text>` (placeholder `"Unnamed"`),
committing on Enter/blur.

### On commit — validation ladder (case-insensitive, mirroring the backend)

Let `typed` be the trimmed committed value for the edited subject:

1. **Empty** → allow. Clears the name back to Unnamed (`nameSubject(id, null)`).
2. **Matches the other column** in this modal (Case 2) → allow. This is redundant
   confirmation that the two are the same person, not a conflict. Detected by comparing
   `typed` against the other column's current name.
3. **Matches any _other_ subject** in `photoService.subjects()` — i.e. a subject whose
   id is neither `subjectA.id` nor `subjectB.id` (Case 3) → **block**. Do **not** call
   the backend. Show an inline error under the field
   (*"A subject named '{typed}' already exists."*). The field reverts to its prior value
   automatically because the local name signal is not updated and `EditableTextComponent`
   re-displays its `value` input.
4. **Otherwise** (Case 1, a new unique name) → call `nameSubject(id, typed)`, update the
   local subject signal, recompute target.

Because Case 3 is caught client-side **before** writing, we never trigger the backend's
unconditional-rename-then-revert behavior.

### Left button label — "Not the same person"

Because dismissing records a `cannot_link` mark (see Background), the left button is
labeled **"Not the same person"** (not "Dismiss") in both modes — its consequence is a
definitive judgment, and the label should say so. This replaces the current
`canDismiss ? 'Dismiss' : 'Not the same person'` ternary with a single constant label.
Behavior is unchanged per mode: `canDismiss=true` calls `dismissMergeSuggestion` (writes
the constraint); `canDismiss=false` (naming-conflict path) still only emits `dismissed`
with no API call — out of scope to change.

### Duplicate guard — nudge + exit guard

`namesIdentical` = both column names non-empty and equal (case-insensitive). While it is
true and the merge has not yet been performed:

- **Nudge:** the `Merge as X` button gets visual emphasis (pulse/ring); the
  "Not the same person" button is de-emphasized.
- **Exit guard:** every leave path — the "Not the same person" button, backdrop click,
  and `Esc` — is intercepted and instead shows an inline confirm strip:
  *"Both named '{name}'."* with **Keep separate** and **Merge** (runs `confirm()`).

**"Keep separate" must call `close()` — never `dismissMergeSuggestion`.** Recording a
`cannot_link` ("not the same person") mark on two subjects the user just named
*identically* is self-contradictory and would poison clustering. So "Keep separate"
simply abandons the merge (no API, no constraint), leaving both subjects and their names
as-is. This is a deliberate contrast with the main "Not the same person" button, which
*does* write the constraint when names differ.

Auto-merging on rename is explicitly **not** done: the modal supports removing individual
faces first, so the user may legitimately still be reviewing after the names match.

### Output change

`confirmed` changes from `EventEmitter<void>` to **`EventEmitter<number>`**, emitting the
surviving subject's id at confirm time (necessary because inline renaming can flip which
subject survives). `people-view`'s `(confirmed)="onConfirmed()"` ignores the argument
(backward compatible); `subject-detail` uses it for navigation (feature 2).

## Feature 2 — Subject-detail uses the merge modal

Keep the "Similar Subjects" list rows (thumbnail + name + match %). Replace the two
inline `Merge` / `Dismiss` buttons with a single **Review** action that opens
`MergeReviewComponent`. Merge and dismiss then happen inside the modal, identically to
`people-view`.

### Wiring (mirrors `people-view`)

- Add `reviewingSuggestion = signal<MergeSuggestion | null>(null)` and `openReview(s)`.
- Import `MergeReviewComponent`; render one instance:
  `<app-merge-review [suggestion]="reviewingSuggestion()" (confirmed)="…"
  (dismissed)="…" (closed)="…" />`.
- **Remove** `mergeSimilar()` and `dismissSimilar()` and the inline buttons. Keep
  `getOtherSubject()` / `getSimilarThumbUrl()` for rendering the rows.

### Navigation on confirm

Using the surviving id emitted by `confirmed`:

- surviving id **== current subject** → reload in place (`loadData(id)`).
- surviving id **!= current subject** (the viewed subject was absorbed) →
  `router.navigate(['/subject', survivingId])`.

This reuses the pattern the tagging composable already uses for `onMerged`.

### Other handlers

- **dismissed** → remove that suggestion from `similarSubjects()` (the modal already
  called the dismiss API).
- **closed** → clear `reviewingSuggestion`.

## Cleanup

No whole component becomes orphaned — every merge component stays in use:

- `merge-review` — now used by **both** people-view and subject-detail (the win).
- `merge-photo-grid` — the modal's internal face grid.
- `confirm-merge-dialog` — still used by the separate **name-conflict rename** flow in
  both `subject-detail` and `subject-person-card`. Untouched.
- `editable-text` — used widely.

The cleanup is therefore bounded to **dead code inside `subject-detail`**: the
`mergeSimilar()` / `dismissSimilar()` methods, their inline Merge/Dismiss markup, and any
import left unused as a result. No file deletions.

## Testing

- **MergeReviewComponent** (extend existing `.spec.ts`):
  - Case 1 rename → `nameSubject` called, local name + `mergeTarget` update.
  - Case 2 (other column's name) → allowed, no error.
  - Case 3 (third subject's name) → `nameSubject` **not** called, inline error shown,
    field reverts. Case-insensitive.
  - Empty commit → clears name.
  - Left button label is "Not the same person" in both `canDismiss` modes;
    `canDismiss=true` click calls `dismissMergeSuggestion`, `canDismiss=false` does not.
  - `namesIdentical` → nudge state on; the left button / Esc / backdrop show the confirm
    strip; **"Keep separate" calls `close()` and never `dismissMergeSuggestion`** (no
    `cannot_link`); "Merge" calls `confirm()`.
  - `confirmed` emits the surviving subject id (verify both target orientations).
- **SubjectDetailComponent** (extend existing `.spec.ts`):
  - "Review" opens the modal with the right suggestion.
  - `confirmed` with surviving == current → reloads; surviving != current → navigates.
  - `dismissed` removes the row.

## Out of scope

- The name-conflict rename flow (`confirm-merge-dialog`) and `subject-person-card`.
- Changing the merge-target tiebreak rule (kept as-is for consistency with people-view).
- Backend changes — all validation is client-side against existing APIs.
