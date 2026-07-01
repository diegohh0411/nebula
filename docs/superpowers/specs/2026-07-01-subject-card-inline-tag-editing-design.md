# Subject card inline tag editing + persisted tag selection

## Problem

The subject card (`subject-person-card`) used in the Tags route (and reused in
the People/Gallery grid) is read-only: renaming a subject or adding/removing
its tags requires navigating into the subject detail page. Additionally, the
Tags route's selected-tag filter is held in a local component signal, so
navigating into a subject and pressing "back" loses the selection.

## Goals

- Edit a subject's name from the card, without opening the detail page.
- Add/remove tags on a subject from the card, without opening the detail page.
- Preserve the previously-selected tag on the Tags route across a
  navigate-into-subject-and-back round trip.

## Non-goals

- Changing how tags are created/renamed/deleted from the Tags route's left
  panel (unaffected by this work).
- Changing the merge-suggestions UI on the detail page beyond extracting the
  shared dialog described below.

## 1. Card component (`subject-person-card`)

### Root element

The card's root changes from `<button class="person-card">` to
`<div class="person-card" role="button" tabindex="0">`. A native `<button>`
cannot legally contain other interactive controls (inputs, nested buttons),
which blocks embedding an editable name and tag controls. Clicking the
photo/scrim area still navigates to the detail page (`(click)="navigate()"`
stays on the root); clicks on the name, tag chips, or the add-tag input call
`$event.stopPropagation()` so they don't also trigger navigation. Keyboard
activation (Enter/Space on the root) still navigates.

### Sizing

Widen the card from `w-40` (160px) to `w-56` (224px) so the tag chips and
add-tag input have room to breathe. Keep the existing aspect-ratio photo tile,
rounded corners, border, and shadow treatment — the new tag-editor footer
(below) is the only structural addition, styled to match the existing chip
aesthetic (`bg-white/20 backdrop-blur-sm` pills) so it doesn't read as a
bolted-on control.

### Name editing

Replace the static `<span class="person-card-name">` with
`<app-editable-text>` (same component already used on the detail page and the
tag list), wired to a new `saveName()` method that calls
`photos.nameSubject(id, name)` directly. The card keeps a local `name` signal,
seeded from `match().subject.name` in `ngOnInit`, updated optimistically on
success.

### Tag editing

Below the photo tile (not overlaid on the scrim, to avoid crowding a
224px-wide card), add a footer strip:

- All of the subject's tags as chips with a `×` to remove, mirroring
  `subject-detail.component.html` lines 33-40.
- An "Add tag…" input with `<datalist>` autocomplete, loading `listTags()` on
  focus — same pattern as `subject-detail`'s `onTagFocus()`/`addTag()`
  (lines 42-59, 212-232).

The card keeps a local `tags` signal, seeded from `match().tags` in
`ngOnInit`, and calls `addSubjectTag`/`removeSubjectTag` directly, updating
the signal optimistically on success. Failures show a brief inline error
under the add-tag input (same `tagError` pattern as the detail page).

### Removing `removable`/`remove`

The existing `removable` input and `remove` output (rendered as a "Remove"
pill, used only by `tags-view` to strip *the currently-viewed tag* from a
subject) become redundant once individual tag chips can be removed directly —
removing the chip for the tag currently being viewed achieves the same thing.
Delete `removable`, `remove`, `.person-card-remove`, and the corresponding
markup/wiring in `tags-view.component.html`.

### New outputs

- `tagAdded = output<Tag>()` — emitted after a successful `addSubjectTag`.
- `tagRemoved = output<number>()` (tag id) — emitted after a successful
  `removeSubjectTag`.
- `merged = output<void>()` — emitted after a successful merge triggered from
  a duplicate-name conflict (see below).

These let containers react without the card knowing about their context.
`gallery.component` (People view) ignores all three — its subject list isn't
filtered by tag or name, so no reaction is needed there. `tags-view` wires up
handlers (see section 2).

### Duplicate-name merge dialog

`subject-detail`'s `saveName()` already handles a "duplicate name" conflict by
showing a modal (merge into the existing subject, or keep separate) — see
`subject-detail.component.ts` lines 140-172 and the modal markup in
`subject-detail.component.html` lines 147-171. The card needs the same
behavior, so extract the modal into a shared `ConfirmMergeDialogComponent`:

- Inputs: none beyond a `open`/visibility flag.
- Outputs: `merge`, `cancel`.
- Markup/copy: identical to the existing inline modal.

Both `subject-detail` and `subject-person-card` render this shared component
instead of inlining the modal. In the card, a duplicate-name conflict shows
the dialog; confirming calls `photos.mergeSubjects(id, conflictId)` and emits
`merged()` on success so the parent can refresh (the *other*, merged-away
subject may still be showing elsewhere in the same list).

## 2. Tags route persistence (`tags-view`)

Move `selectedTag` from local-only signal state into the URL as a query
param, e.g. `/tags?tag=<id>`:

- `selectTag(tag)` calls `router.navigate([], { relativeTo: route,
  queryParams: { tag: tag.id }, queryParamsHandling: 'merge', replaceUrl: true
  })` instead of writing the signal directly. `replaceUrl: true` keeps
  clicking through several tags from spamming browser history — only the
  `/tags` ↔ `/subject/:id` transition should be a "back-able" step.
- `ngOnInit` awaits `loadTags()`, then subscribes to `route.queryParamMap`.
  On each emission (including the replayed initial value), it reads `tag`,
  finds the matching `TagWithCount` in `tags()`, and loads its subjects via
  `getTagSubjects`. This subscription becomes the single source of truth for
  "what's selected," replacing direct signal writes from click handlers.
- `deleteTag()`, when deleting the currently-selected tag, clears the param
  through the same navigate-based path instead of clearing the signal
  directly.

Net effect: select a tag → click into a subject → the existing `goBack()`
(`Location.back()`) in `subject-detail` restores `/tags?tag=<id>` via normal
browser history, and the component reconstructs the same selection from the
URL. No new persistence layer (service/localStorage) is needed.

### Card event handlers in `tags-view`

- `(tagAdded)` → `loadTags()` (refresh left-panel subject counts; the tag may
  be new or an existing one whose count just changed).
- `(tagRemoved)="onTagRemoved(match.subject.id, $event)"` → if the removed
  tag id matches `selectedTag()!.id`, evict that subject from `tagSubjects`
  (same effect as the old `removeSubjectFromTag`); always `loadTags()`
  afterward to refresh counts.
- `(merged)` → `loadTags()` and re-fetch `tagSubjects` for the current tag,
  since the merged-away subject may have been showing in the same list.

## Data flow summary

```
Card (self-contained mutation + optimistic local state)
  ├─ nameSubject() ──────────────► on duplicate ► ConfirmMergeDialogComponent
  │                                                 └─ merge ► mergeSubjects() ► emit merged()
  ├─ addSubjectTag() ─────────────► emit tagAdded(tag)
  └─ removeSubjectTag() ──────────► emit tagRemoved(tagId)

tags-view (listens to emitted events, reloads as needed)
  ├─ tagAdded  → loadTags()
  ├─ tagRemoved → maybe evict from tagSubjects, then loadTags()
  └─ merged    → loadTags() + refetch tagSubjects

gallery.component (People view) — ignores all card events, no filtering to keep in sync
```

## Testing

- `subject-person-card.component.spec.ts`: update for the new `div` root
  (was `button`); add cases for name-edit commit calling `nameSubject`, tag
  removal calling `removeSubjectTag` and emitting `tagRemoved`, add-tag flow
  calling `addSubjectTag` and emitting `tagAdded`, duplicate-name conflict
  showing the shared dialog and emitting `merged` on confirm, and that clicks
  on the name/tag controls don't trigger navigation.
- `subject-detail.component.spec.ts` (if present) / manual check: still works
  identically after switching to the shared `ConfirmMergeDialogComponent`.
- `tags-view`: add coverage for query-param-driven selection (selecting a tag
  updates the URL; loading `/tags?tag=<id>` restores the selection) and the
  new `tagAdded`/`tagRemoved`/`merged` handlers replacing
  `removeSubjectFromTag`.
