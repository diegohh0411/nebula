# Inline "Add a name" on People grid cards

**Task:** TT-26  
**Date:** 2026-06-08  
**Status:** Approved for implementation

## Goal

Let the user name an unnamed cluster directly from the People grid without navigating to the detail page — Google-Photos-style "Add a name" inline affordance.

## Decisions

- **Affordance trigger:** Hover-reveal — caption area is empty by default; hovering shows a "+ Add a name" ghost hint; clicking the hint opens the input (stops propagation so card routerLink does not fire).
- **Conflict flow:** Extend `MergeReviewComponent` with `@Input() canDismiss = true`. When `false`, the dismiss button becomes "Cancel" and emits `dismissed` directly without calling `dismissMergeSuggestion`. The full side-by-side photo review modal is shown so the user can verify faces before merging.

## State (PeopleViewComponent)

| Signal | Type | Purpose |
|---|---|---|
| `editingSubjectId` | `Signal<number \| null>` | Which card is in editing mode |
| `editingName` | `Signal<string>` | Live input value |
| `namingConflict` | `Signal<MergeSuggestion \| null>` | Synthetic suggestion on name conflict |

All state lives in `PeopleViewComponent`. No new child component is needed for the input.

## Component Changes

### `people-view.component.ts`

Five new methods:

- **`startEditing(subject, event)`** — `event.stopPropagation()`, set `editingSubjectId(subject.id)`, clear `editingName`
- **`commitName(subject)`** — trim value; if empty call `cancelEditing()`; otherwise optimistically update `photoService.subjects`, call `nameSubject`, handle result
- **`cancelEditing()`** — clear both editing signals
- **`onKeydown(event, subject)`** — Enter → `commitName`; Escape → `cancelEditing`; Tab → `commitName` then move focus to next unnamed card's input
- **`onConflictConfirmed()`** — call `mergeSubjects(currentId, conflictId)`, reload subjects + thumbnails, clear `namingConflict`
- **`onConflictDismissed()`** — revert optimistic name update, clear `namingConflict`

### `people-view.component.html`

Caption area per card:

- **Named:** `<span>{{ subject.name }}</span>` (unchanged)
- **Unnamed, not editing:** empty; on `group-hover` show `<span class="add-name-hint" (click)="startEditing(subject, $event)">+ Add a name</span>`
- **Unnamed, editing:** `<input (keydown)="onKeydown($event, subject)" (blur)="commitName(subject)" (click)="$event.stopPropagation()" [(ngModel)]="editingName">`

### `merge-review.component.ts`

- Add `@Input() canDismiss = true`
- In `dismiss()`: when `!canDismiss`, skip API call and emit `dismissed` directly
- In template: button label is "Cancel" when `!canDismiss`, "Not the same person" when `canDismiss`

## Data Flow

### Happy path

1. User hovers unnamed card → "+ Add a name" hint appears
2. User clicks hint → `startEditing()` stops propagation, shows focused input
3. User types name → Enter → `commitName()`
4. Optimistic update: `photoService.subjects.update(...)` sets name immediately
5. Editing state cleared; `nameSubject` called in background
6. `nameSubject` resolves `duplicate_subject_id: null` → done

### Conflict path

1. `nameSubject` resolves `duplicate_subject_id: N`
2. Look up subject N from `photoService.subjects()` to build synthetic `MergeSuggestion { id: -1, subject_a: current, subject_b: duplicate, score: 1.0 }`
3. Set `namingConflict` → merge review modal opens with `[canDismiss]="false"`
4. Confirm → `mergeSubjects` → reload subjects + thumbnails → clear conflict
5. Cancel → revert optimistic name → clear conflict

### Error path

If `nameSubject` throws, revert the optimistic update and log to console. No toast (matches existing patterns).

### Card click guard

The `<a [routerLink]>` wraps the whole card. Input and hint both carry `(click)="$event.stopPropagation()"`. Card navigation fires only when neither is the click target.

## Keyboard Flow

- **Enter** on input → commit name
- **Escape** on input → cancel, card returns to hover-reveal state
- **Tab** on input → commit name, then find the next unnamed subject in `photoService.subjects()`, set `editingSubjectId` to it, and use `@ViewChildren` + `setTimeout(0)` to focus the newly rendered input (inputs only exist in the DOM when their card is in editing mode)
- **Blur with value** → commit (handles clicking away)
- **Blur without value** → cancel

## Synthetic MergeSuggestion Construction

When `nameSubject` returns `duplicate_subject_id: N`, build:

```ts
{
  id: -1,
  subject_a: photoService.subjects().find(s => s.id === N),  // existing named subject
  subject_b: currentSubject,                                  // just-named subject
  score: 1.0
}
```

`subject_a` is the pre-existing named subject so `MergeReviewComponent.mergeTarget` shows it on the left column. Both subjects will appear named (due to the optimistic update), so the lower-ID subject wins as merge target — acceptable behavior since ID ordering reflects insertion order.

## Tests

### `people-view.component.spec.ts` (new file)

1. **Inline submit** — render an unnamed subject, click hint, type name, press Enter → `nameSubject` called with correct id and name
2. **Optimistic update** — after Enter, card shows name before service promise resolves
3. **Card click navigates when not editing** — clicking card body (not hint/input) triggers router navigation; clicking while editing does not

### `merge-review.component.spec.ts` (addition)

4. **`canDismiss=false` skips API** — `dismiss()` emits `dismissed` without calling `photoService.dismissMergeSuggestion`

## Out of Scope

- Backend changes (existing `name_subject` command handles everything)
- Mobile/touch-specific hover workarounds (hover CSS works fine; tap opens input directly)
- Detail page naming flow — remains unchanged
