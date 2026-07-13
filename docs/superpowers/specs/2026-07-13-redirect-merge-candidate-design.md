# Redirect a Merge Candidate to a Different Existing Subject — Design

**Date:** 2026-07-13
**Status:** Approved (design direction pre-scoped via advisor-model consultation against the
live `merge-review` component; this document formalizes and verifies it against current code)

## Goal

In the People > Possible Duplicates review modal (`MergeReviewComponent`), give the user a
way to redirect a merge candidate to a *different*, already-named subject that isn't part of
the current algorithmic suggestion — without losing the correct match (today's only options,
"Not the same person" / "Merge as {Name}", force a lossy choice between rejecting the
suggestion outright or accepting the wrong pairing).

Three parts:

1. A "Merge into someone else…" typeahead entry point in the modal footer, with a
   re-target-then-confirm flow.
2. An upgraded name-collision error (existing dead-end in `onNameCommit`) that becomes a
   second, more discoverable entry point into the same re-target flow.
3. An explicit rule that a redirect-merge does **not** auto-write a `cannot_link` constraint
   between the original "keep" subject and the newly-chosen target.

## Background — current state (verified against `.worktrees/redirect-merge-candidate` at
HEAD, commit `b291163`)

Read directly, not assumed:

- `src/app/components/merge-review/merge-review.component.ts` — `subjectA` / `subjectB`
  signals seeded from the `@Input suggestion` (`MergeSuggestion { id, subject_a, subject_b,
  score }`). `mergeTarget` is a **getter** (not yet a `computed()`, contrary to a stale
  reference in the sibling 2026-07-10 spec) that derives `{ target, source }` from
  `subjectA()` / `subjectB()`: named beats unnamed; if both/neither named, lower id wins.
  `confirm()` runs the merge animation, calls `photoService.mergeSubjects(target.target.id,
  target.source.id)`, and emits `confirmed` with the surviving id. `showExitConfirm` is a
  boolean signal that swaps the entire `.modal-actions` footer for a same-name confirm strip
  (`keepSeparate()` / `confirmFromGuard()`) — this is the state-machine pattern the new
  "typeahead" footer mode extends, as a sibling boolean alongside `showExitConfirm`, not a
  replacement for it.
- `onNameCommit(which, rawValue)` (`merge-review.component.ts:100-132`): on a name collision
  with a **third** subject (not either column in the modal), it sets `nameErrorA`/`nameErrorB`
  to `A subject named "X" already exists.` and returns without writing — this is the exact
  dead-end the Notion task calls out, confirmed still present and still blocking.
- `src/app/components/merge-review/merge-review.component.html` — footer (`.modal-actions`,
  lines 69-106) currently renders exactly two states: the exit-confirm strip, or the normal
  two-button row (`Not the same person` / `Merge as {name}`). There is currently **no** third
  footer state and no existing tertiary link — confirms the task's premise that this is new
  UI, not a modification of an existing affordance.
- `src/app/services/photo.service.ts` — `subjects` is a `signal<Subject[]>` populated by
  `loadSubjects()` (called after every `nameSubject` / `mergeSubjects`). `Subject` = `{ id,
  name: string | null, thumbnail_face_id, type, added_at }` (`src/app/models/models.ts:83-89`).
  `getSubjectPhotosWithFaces(subjectId)` returns `SubjectPhotoFace[]` and is already the exact
  call `loadPhotos()` uses for both columns — reusable as-is for loading the redirected
  target's faces. `mergeSubjects(targetId, sourceId)` and `dismissMergeSuggestion(id)` exist
  as described.
- **Backend — "no backend changes needed" claim verified true.**
  `src-tauri/src/people/commands.rs:182-199` (`#[tauri::command] merge_subjects`) takes
  `target_id: i64, source_id: i64` with no relationship constraint between them, and
  `src-tauri/src/people/repo.rs:473-558` (`repo::merge_subjects`) performs the merge purely
  by id: reassigns `faces.subject_id`, writes `must_link` constraints between every face pair,
  unions `subject_tags`, deletes any `merge_suggestions` row referencing **either** id, and
  deletes the source subject row — all in one transaction. Nothing in this path assumes the
  two ids came from the same `MergeSuggestion`; arbitrary target/source pairs already work
  today. **This also independently confirms part 3's "no special handling needed" claim**:
  the `DELETE FROM merge_suggestions WHERE subject_id_a = ? OR subject_id_b = ? OR
  subject_id_a = ? OR subject_id_b = ?` (bound to target_id, target_id, source_id, source_id)
  removes the original suggestion row purely because the source subject (B) stops existing —
  no code needs to specifically "know" a redirect happened.
- `src-tauri/src/people/repo.rs:579-619` (`dismiss_merge_suggestion`) is the only path in the
  codebase that writes a `cannot_link` constraint from the merge-review flow (`source =
  'dismiss'`). `merge_subjects` never writes `cannot_link` — only `must_link`. This confirms
  part 3 requires **no backend guard**: there is no existing code path that would
  auto-write a cannot-link between the original keep subject (A) and the new target (C) after
  a redirect-merge, because nothing currently connects A and C at all once B is gone. The
  design requirement in part 3 is therefore satisfied by construction — it is a "must not add"
  constraint on the implementation, not new logic to write.

No divergence from the Notion task's design was found. The task's line-number reference
("~111-124") for the collision block is now `100-132` in this checkout (the method grew
slightly, contents match) — noted for whoever implements this, not a design change.

## Part 1 — "Merge into someone else…" typeahead footer

### Entry point

A quiet tertiary link, left of the two existing footer buttons, muted/underline-on-hover
styling so it doesn't compete visually with the primary confirm/reject actions:
**"Merge into someone else…"**. Rendered only in the normal (non-`showExitConfirm`) footer
state. Hidden while `submitting()` (consistent with the existing buttons' disabled state).

### Footer state machine

Add `protected showRedirectPicker = signal(false)`, a third mutually-exclusive footer mode
alongside `showExitConfirm`. The three states are mutually exclusive — `.modal-actions`
renders exactly one:

1. Normal (two buttons + the new tertiary link)
2. `showExitConfirm` (same-name exit guard, unchanged)
3. `showRedirectPicker` (new — the inline combobox)

Clicking "Merge into someone else…" sets `showRedirectPicker.set(true)`. The combobox input's
own `(keydown.escape)` handler must call `$event.stopPropagation()` before restoring the
normal footer (`showRedirectPicker.set(false)`) — otherwise the event bubbles to the existing
`@HostListener('document:keydown.escape') onEscape()`, which unconditionally calls
`this.close()` and would close the whole modal instead of just collapsing the picker. This
mirrors no existing pattern in the component (the exit-confirm strip has no such per-state
Escape handling today) and is called out here specifically because it is easy to miss.

### Combobox behavior

- Autofocused text input on entry. **Implementation note:** the plain HTML `autofocus`
  attribute does not reliably focus an element that appears via a structural directive
  (`@if`) after initial render in most browsers; use a `viewChild` reference to the input and
  call `.focus()` explicitly (e.g. in an `afterNextRender`/effect triggered by
  `showRedirectPicker()` becoming true), not the `autofocus` HTML attribute alone.
- Filters `photoService.subjects()` client-side (no new backend call) to: named subjects
  (`s.name` truthy) whose id is neither the current candidate/source subject's id.
  ("Current candidate/source" = `mergeTarget.source.id` at the moment the picker opens — see
  Data flow below for why this is captured once, not re-derived live.)
- Each result row: avatar + name. Avatar is `getFaceCrop(subject.thumbnail_face_id)` (the
  same lookup subject-detail/people-view already use for subject thumbnails); if
  `thumbnail_face_id` is null, fall back to a generic placeholder avatar (existing pattern —
  same fallback other subject lists already use, no new asset needed).
  **Face count is out of scope for v1**: neither `Subject` (`id, name, thumbnail_face_id,
  type, added_at`) nor `SubjectMatch` (`subject, tags` — checked directly, no count field)
  carries a face count today, and adding one would mean a new backend query or a lookup per
  row. Since the primary thing the user needs to recognize the right subject is the face
  itself, the avatar already carries that signal; showing count is a nice-to-have deferred
  past v1, not a blocker.
- Keyboard: Up/Down to move the highlighted row, Enter to select the highlighted (or sole
  exact) match, Escape to cancel back to the normal footer without picking.
- Empty results (typed text matches no named subject, or there are zero other named
  subjects in the library): show an inert "No matching subjects" row — no action, the input
  stays open so the user can keep typing or press Escape. This is not an error state; it is
  expected the first time a library has very few named subjects.

### On pick — re-target, don't merge immediately

Add `protected targetOverride = signal<Subject | null>(null)`. Selecting a subject (call it
C) does, in order:

1. `targetOverride.set(C)`.
2. `showRedirectPicker.set(false)` (collapse back to the normal footer, now showing the
   redirected state).
3. Reload C's faces into the slot that currently shows the keep column's faces (see Data flow
   — this reuses the existing `photosA`/`photosB` + `loading` signal pair, keyed by whichever
   of A/B was the original target).
4. `mergeTarget` reflects the override: `target = C`, `source` = whichever of subjectA/B is
   the non-`target` original participant (i.e. the original *candidate*, not the original
   keep subject — see the worked example below). The `keep` badge moves to C's slot, the
   match-% chip in the modal header is hidden or relabeled "manual reassignment" (the score
   was computed against the original suggestion; it has no meaning for the picked subject).
5. The primary button label recomputes ("Merge as {C.name}") — but does **not** merge yet.
   One more click on that button runs the existing `confirm()` path unchanged.

Requiring the extra click (rather than merging immediately on pick) preserves the modal's
existing visual-verification contract: the user sees C's actual faces next to the candidate's
faces before committing, same as an algorithm-suggested merge. Redirecting from memory of a
face is exactly the case where a quick visual double-check matters most, since there's no
algorithmic score to lean on.

### Data flow / state ownership

`subjectA` / `subjectB` remain the two original suggestion participants for the lifetime of
the modal — they are **not** mutated by a redirect (unlike renaming, which does mutate them
in place). Instead:

- `mergeTarget` becomes redirect-aware: if `targetOverride()` is set, return `{ target:
  targetOverride()!, source: <the original non-keep-original participant> }`; otherwise fall
  back to the existing named/id-tiebreak logic unchanged.
- **Which original subject is "source" after a redirect?** The candidate being redirected is
  the subject the user is actively re-homing — i.e. whichever of `subjectA`/`subjectB` was
  **not** the original `mergeTarget.target` at the moment the picker was opened. Capture this
  once, into e.g. `protected redirectSource = signal<Subject | null>(null)`, set alongside
  `targetOverride` in step 1 above (`redirectSource.set(originalMergeTarget.source)`) — using
  a live re-derivation here would be wrong, because once `targetOverride` is set, the
  "original" named/unnamed tiebreak that used to compute `.source` no longer applies (C is
  never one of `subjectA`/`subjectB`, so the getter must special-case this).
- The faces display: the column that used to show the original keep subject's faces now shows
  C's faces (via a fresh `getSubjectPhotosWithFaces(C.id)` call, same loading-guard pattern —
  reuse `_loadGen` — as `loadPhotos()` uses today, so a slow load can't clobber a faster
  concurrent one). The column that showed the redirect source's faces is untouched (still
  showing the original candidate's faces, since that subject hasn't changed).
- Worked example: suggestion pairs A (named "Alex", keep) with B (unnamed candidate). User
  redirects to C ("Roberto"). After pick: `mergeTarget = { target: Roberto, source: B }`. A's
  column and its faces disappear from the merge computation entirely — A is not merged,
  renamed, or touched at all; it simply stops being involved. On confirm, `mergeSubjects(C.id,
  B.id)` runs. A remains exactly as it was before the modal opened.
- **Reset on a new suggestion.** `MergeReviewComponent` is a single long-lived instance reused
  across suggestions — confirmed in `people-view.component.html:89-97`
  (`[suggestion]="reviewingSuggestion()"` on a component that is never destroyed/recreated
  between reviews). The `suggestion` input setter (`merge-review.component.ts:39-45`) already
  resets `subjectA`/`subjectB`/photos but does **not** reset any redirect state. Without an
  explicit reset, confirming a redirect-merge and then advancing to the *next* suggestion
  leaves `targetOverride`/`redirectSource` set from the previous review: `mergeTarget` (which
  checks `targetOverride()` first, unconditionally) would silently redirect the brand-new
  suggestion to the stale target, and one click on "Merge as {stale name}" would merge two
  subjects that have nothing to do with the suggestion on screen. The `suggestion` setter must
  additionally reset `targetOverride`, `redirectSource`, `showRedirectPicker`, `redirectQuery`,
  `redirectGoneError`, and (per the stale-error note below) `nameErrorA`/`nameErrorB` to their
  initial values every time a new suggestion is assigned, including when a redirect was left
  active on the previous suggestion.
- Canceling the picker (`Escape` with nothing selected) leaves `targetOverride` untouched —
  it was never set, so the footer/`mergeTarget` revert to normal automatically (no explicit
  "undo" needed, since nothing was mutated yet).
- **Undoing an active redirect** (user already picked C, sees the "Merge as Roberto" button,
  changes their mind): clicking "Merge into someone else…" again re-opens the picker and
  picking a new subject simply overwrites `targetOverride`/`redirectSource` — no separate
  "revert to original" affordance is needed for v1, since the original suggestion (A/B) is
  still fully intact in `subjectA`/`subjectB` and reopening the modal (close + reopen from
  the suggestions list) is an acceptable way to fully reset. Explicitly out of scope: a
  dedicated "back to original suggestion" button.
- **Repeated redirects and which photo slot gets reloaded.** Re-picking (the case immediately
  above) must not re-derive the photo slot from `mergeTarget.target.id` the second time,
  because once `targetOverride` is already set, `mergeTarget.target.id` is C's id (the
  *previous* pick), which matches neither `subjectA.id` nor `subjectB.id` — a naive
  "find the column whose id equals the current target" lookup would fall through to a
  wrong-column default and clobber the untouched candidate's faces instead of replacing the
  first pick's faces. The photo slot must be decided **once**, from the *original* (pre-any-
  redirect) suggestion — i.e. "is the original keep subject `subjectA` or `subjectB`" — and
  that slot choice must be remembered (e.g. a `redirectColumn: 'a' | 'b'` captured on the
  first redirect and reused, unchanged, by every subsequent pick in the same modal session)
  rather than recomputed from the live (already-redirect-aware) `mergeTarget` on each pick.
- **Column header/name/badge during an active redirect.** The slot that now shows C's faces
  still renders the *original* keep subject's name via `EditableText` and the `keep` badge
  check (`mergeTarget?.target?.id === subjectA()?.id`) matches neither column once
  `targetOverride` is set (C is never `subjectA`/`subjectB`), so the badge silently
  disappears and the name field silently keeps showing/renaming the bypassed original
  subject, not C. During an active redirect this display must be redirect-aware: the
  redirected slot's name must show `targetOverride()!.name` (not the original subject's
  name), the `keep` badge must render on that slot based on `targetOverride()` being set
  (not solely the original id comparison), and the header match-% chip must be hidden or
  relabeled ("manual reassignment") as already specified above. **Accepted trade-off:**
  wiring the redirected slot's rename control to actually target `C` is deferred past v1 —
  `onNameCommit` continues to resolve the subject from `subjectA()`/`subjectB()` as it does
  today, so committing a rename on the redirected slot while a redirect is active still
  renames the bypassed original subject under the hood, even though the *displayed* name now
  reads as `C`'s. This is a narrow edge case (the user would have to type into a name field
  mid-redirect rather than just click "Merge as {C.name}"), and treating it as v1-safe
  requires only that the display (name text, keep badge, score chip) is corrected — not that
  the rename control itself is retargeted.
- **The merge-confirmation animation is skipped during a redirect.** `runMergeAnimation`
  determines the source/target columns via `target.target.id === this._suggestion!.subject_a.id`
  — after a redirect this is always false (C is never `subject_a`), so the fade/scale
  animation would run against the wrong DOM columns (animating as if the redirected slot
  were the *source* fading out, when it is in fact the new target). Rather than reimplement
  column-mapping logic for the animation, `runMergeAnimation` should short-circuit and skip
  the animation entirely whenever `targetOverride()` is set — an accepted v1 simplification,
  not a fix of the general column-mapping problem (out of scope: animating a redirect-merge
  is deferred past v1).

### Edge case — the picked subject (C) is deleted mid-flow

Between picking C and clicking "Merge as Roberto", another part of the app (a background
merge, a delete-subject action elsewhere) could remove C. `confirm()` calls
`mergeSubjects(target.target.id, target.source.id)`; if `target.target.id` no longer exists,
the backend's `merge_subjects` (repo.rs:473) will find no matching row in the `SELECT id, name
FROM subjects WHERE id = ? OR id = ?` lookup for that id, proceed to reassign zero faces (the
`UPDATE faces SET subject_id = ? WHERE subject_id = ?` binds source_id, which is still
valid — this actually still works and silently "merges into a ghost id"). **This is a real
gap**: nothing in the current backend rejects a merge into a nonexistent target id. Given
"no backend changes" is the stated scope for this task, the frontend mitigation is: `confirm()`
catches the error path as it already does (`try { … } catch (e) { console.error(...) }`), and
additionally — before calling `mergeSubjects` when `targetOverride()` is set — the frontend
re-checks that `targetOverride()!.id` still appears in the current `photoService.subjects()`
snapshot (refreshed by any `loadSubjects()` call that happened in the interim, e.g. another
tab's action would not update this client's live signal until an event triggers a reload, so
this check is best-effort, not a hard guarantee). If it's gone, show an inline error in the
redirect state ("Roberto is no longer available — pick another subject.") and reopen the
picker rather than attempting the merge. This is a UX safety net, not a data-integrity
guarantee — true concurrent-delete safety would require a backend existence check, which is
explicitly out of scope per the task.

## Part 2 — Upgrade the name-collision error into a second entry point

Change the Case 3 block in `onNameCommit` (the `if (typed) { ... }` block inside the method
spanning `merge-review.component.ts:100-132`, itself at lines `111-124`) from a
blocking dead-end into an actionable inline prompt. Today: sets `nameErrorA`/`nameErrorB` to a
plain string and returns. New: the error slot becomes a small structured object instead of a
plain string —

```ts
protected nameErrorA = signal<{ message: string; conflict: Subject } | null>(null);
// (nameErrorB mirrors this)
```

Rendered in the template as: *"A subject named 'Roberto' already exists — merge this cluster
into them instead?"* with a **[Merge into Roberto]** button beside it.

Clicking that button reuses the exact same re-target mechanism as Part 1's pick step
(`targetOverride.set(conflict)`, `redirectSource.set(...)`, reload faces, collapse to the
normal footer showing "Merge as Roberto") — Part 2 does not duplicate Part 1's logic, it is a
second caller of the same "apply redirect" method. Because `onNameCommit` already resolved
`conflict` (the colliding subject) to look up the error message, no additional
`photoService.subjects()` lookup is needed at click time — the conflict subject object is
already in hand and stored on the error signal itself.

**Which subject is the redirect `source` for this entry point?** This is *not* necessarily
`mergeTarget.source` at click time, and reusing Part 1's pick logic verbatim (which always
takes `mergeTarget.source` as the thing being redirected) is wrong here. A name collision can
happen while renaming **either** column — including the current `mergeTarget.target` (the
"keep" subject), not only the candidate. The subject that must become the redirect's source
is whichever subject the user was actually typing the colliding name into (i.e. the `subject`
resolved at the top of `onNameCommit`, the same one `conflict` was checked against) —
*regardless* of whether that subject is currently `mergeTarget.target` or `mergeTarget.source`.
Concretely: the shared "apply redirect" method must accept the source subject explicitly as a
parameter for this entry point (`applyRedirect(picked, explicitSource)`), rather than reading
`mergeTarget.source` internally, because by the time the button is clicked the renamed subject
may be the one currently occupying the *target* slot. Passing the wrong one silently merges an
unrelated, untouched subject into C while leaving the subject the user was actually referring
to as "Roberto" untouched — the opposite of what the user asked for. Part 1's picker keeps its
existing default behavior (source = `mergeTarget.source`, since there the redirect always
targets the non-keep candidate slot); only Part 2's caller needs to pass the explicit override.

The blocked rename itself is **not** retried or auto-applied — the user was trying to rename
the candidate to "Roberto", which is now moot once the whole candidate is being merged into
the real Roberto (the name will apply automatically as part of the merge, since Roberto's own
name is what will read as the surviving name). No separate call to `nameSubject` happens on
this path.

## Part 3 — No auto-write of a `cannot_link` between the original keep subject and the new target

After a redirect-merge (B → C, with A the original keep subject bypassed), do **not** write
any `cannot_link` / dismissed-pair record between A and C. The user only asserted "B is C" —
nothing about A and C's relationship. If clustering later independently suggests A and C are
the same person, that suggestion must still be free to surface.

**Implementation implication: this is a "must not add" constraint, not new code.** As
established in Background, `merge_subjects` never writes `cannot_link` today (only
`dismiss_merge_suggestion` does, and this flow never calls it for the A/C pair). The
requirement is satisfied as long as the frontend implementation of Parts 1–2 calls
`mergeSubjects` and nothing else for the merge itself — no call to `dismissMergeSuggestion`
should ever be added to the redirect path, even implicitly (e.g. as a "clean up the old
suggestion" convenience call). This is worth stating explicitly as an implementation guardrail
because it would be an easy, plausible-looking bug to add such a call believing it "cleans up"
the now-stale A/B suggestion — that cleanup already happens for free via the
`DELETE FROM merge_suggestions ... OR subject_id_a = ? OR subject_id_b = ?` clause inside
`merge_subjects` itself (Background). No frontend change is needed to make the old A/B
suggestion disappear from the list.

## Testing

Extend `merge-review.component.spec.ts`:

- **Typeahead entry point:** "Merge into someone else…" link visible in the normal footer,
  hidden during `showExitConfirm`/`showRedirectPicker`/`submitting()`.
- **Filtering:** typeahead list excludes the current source subject and unnamed subjects;
  includes other named subjects; case-insensitive substring match on typed text.
- **Empty results:** typing text matching nothing shows the inert "No matching subjects" row,
  does not throw, Escape still restores the normal footer.
- **Pick → re-target:** selecting a subject sets `targetOverride`, `mergeTarget.target` is the
  picked subject, `mergeTarget.source` is the original candidate (not the original keep
  subject), faces reload for the picked subject (`getSubjectPhotosWithFaces` called with its
  id), footer returns to normal with "Merge as {picked name}".
- **Confirm after redirect:** `confirm()` calls `mergeSubjects(pickedId, originalCandidateId)`
  — verify the original keep subject's id is **never** passed to `mergeSubjects` in this path.
- **Escape cancels cleanly:** opening the picker and pressing Escape without picking leaves
  `targetOverride` null and `mergeTarget` unchanged from its pre-picker value.
- **Name-collision upgrade:** typing a name colliding with a third subject in `onNameCommit`
  populates the structured error with the conflict subject and renders the
  "[Merge into {name}]" button; clicking it produces the same end state as the Part 1 pick
  path (`targetOverride` set to the conflict subject); `nameSubject` is never called for the
  attempted rename on this path.
- **No `cannot_link` side effect:** after a redirect-merge confirm, assert
  `dismissMergeSuggestion` is never called by the component in this flow (spy asserts zero
  calls across the whole redirect journey, not just the confirm step).
- **Deleted-target guard:** with `targetOverride` set to a subject no longer present in
  `photoService.subjects()` at confirm time, `confirm()` shows the inline "no longer
  available" error and reopens the picker instead of calling `mergeSubjects`.
- **State resets on a new suggestion:** with a redirect active (`targetOverride` set,
  `showRedirectPicker` possibly true), assigning a *new* `suggestion` input value resets
  `targetOverride`/`redirectSource`/`showRedirectPicker`/`redirectQuery`/`redirectGoneError`/
  `nameErrorA`/`nameErrorB` to their initial (unset) values — `mergeTarget` for the new
  suggestion reflects only the new `subjectA`/`subjectB`, never a stale override.
- **Repeated redirect does not corrupt the untouched column:** with a redirect already active
  (picked C once), picking a *second* subject D reloads D's faces into the same column that
  showed C's faces (not the column that has held the original candidate's faces all along) —
  assert via `photoService.getSubjectPhotosWithFaces` being called with D's id and the
  untouched column's photos signal being unchanged from its pre-redirect value.
- **Part 2 redirects the subject actually being renamed, not always the non-target one:**
  with a suggestion where the *target* (keep) subject is the one whose rename collides with a
  third subject, clicking "[Merge into {name}]" results in `confirm()` calling
  `mergeSubjects(conflictId, originalTargetId)` — i.e. the subject who was being renamed is
  the one merged into the conflict, not `mergeTarget.source` from before the redirect.
- **Name-collision entry point exercised through the template, not by calling the redirect
  method directly:** the collision-button test must render the component, trigger the
  collision via a simulated name-commit event, query the rendered "[Merge into {name}]"
  button in the DOM, and dispatch a click on it — a test that calls `component.applyRedirect(...)`
  itself instead of clicking the rendered button would pass even if the button were never
  wired to anything in the template, and does not prove Part 2's UI entry point works.
- **Stale collision error is cleared by a redirect:** after a redirect is applied (either
  entry point), `nameErrorA`/`nameErrorB` for the affected column is cleared — a leftover
  "already exists" message must not persist beside a column that is now merging.

## Out of scope

- Backend changes of any kind (verified unnecessary — see Background).
- Face count in the typeahead result rows (v1 shows avatar + name only).
- A dedicated "revert to original suggestion" affordance once a redirect has been picked.
- Hardening `merge_subjects` against a target id that doesn't exist (noted as a latent gap in
  the Deleted-target edge case; the frontend best-effort check is a mitigation, not a fix, and
  a real fix would be a backend change explicitly out of scope here).
- Drag-and-drop or a browse-grid alternative to the typeahead (rejected in the original design
  brief: no natural drop target in this modal, and typeahead better matches the "I already
  know the name" premise of this action).
- **Accepted trade-off — the redirect *source* (B) being deleted/renamed mid-flow is not
  guarded.** The Deleted-target-guard above only re-checks `targetOverride` (C) at confirm
  time; it does not re-check `redirectSource` (B). A concurrent deletion of B is a narrower
  window (B is actively on screen, mid-review, versus C which the user may not have looked at
  since picking it) and follows the same latent "no backend existence check" gap already
  called out for C. Treated as out of scope for the same reason: a real fix needs a backend
  existence check, which this task explicitly does not include.
- **Accepted trade-off — the guardrail that the redirect path never calls
  `dismissMergeSuggestion` (Part 3 / Testing) only covers `MergeReviewComponent` itself.** It
  does not extend to any parent container's post-confirm handling (e.g. `people-view`'s
  `(confirmed)` handler) that could independently decide to call `dismissMergeSuggestion` as
  a "cleanup" step for the bypassed A/B suggestion. Verified: no such call exists in
  `people-view` today, and none is being added by this task, so there is nothing for the
  guardrail to miss right now — but a future change to the parent's `(confirmed)` handler
  would not be caught by this component-level test. Not extending the guardrail to the parent
  is accepted because the parent's handler is out of this task's file map entirely.
- **Accepted trade-off — animating a redirect-merge is not implemented; the animation is
  skipped for a redirected confirm** (see Data flow, "the merge-confirmation animation is
  skipped during a redirect"). Building correct column-mapping for the animation after a
  redirect is deferred past v1.
- **Accepted trade-off — renaming the redirected slot mid-redirect still renames the bypassed
  original subject, not `C`**, even though the display is corrected to show `C`'s name (see
  Part 1, "Column header/name/badge during an active redirect"). Retargeting the rename
  control itself is deferred past v1.
