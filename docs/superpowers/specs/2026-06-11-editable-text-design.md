# EditableText — Click-to-Edit UX Standard

**Date:** 2026-06-11
**Status:** Approved for implementation

## Problem

Two bugs exist in the current UI:

1. **People view — click propagation.** Subject cards are wrapped in a single `<a [routerLink]>`. When the user clicks `+ Add a name`, the input appears but subsequent clicks on it propagate to the anchor and navigate to subject detail instead of letting the user type.

2. **Subject detail — pencil-only editing.** The subject name is only editable by clicking a hover-revealed pencil icon. Clicking the name text itself does nothing.

Additionally, named subjects in the People view have no inline rename affordance at all — clicking their name navigates rather than entering edit mode.

## UX Standard

Any editable text field in Nebula follows this pattern:

- **Click** the displayed text (or placeholder) → enters edit mode; input autofocuses with current value pre-filled.
- **Blur or Enter** → commits. Always emits, including empty string (explicit removal of content).
- **Escape** → cancels; discards draft, reverts to original display without emitting.
- No pencil icon required. `cursor-text` on the display text is the affordance.

## Solution: `app-editable-text` Component

A standalone Angular component that owns the display↔edit toggle, focus management, and commit/cancel keyboard handling. It is a pure UX shell — it emits committed values and lets the parent handle API calls and error recovery.

### Component API

```typescript
@Input() value: string | null          // displayed text; null = show placeholder
@Input() placeholder: string           // e.g. "+ Add a name", "Unnamed Person"
@Input() placeholderClass?: string     // extra Tailwind classes for placeholder styling
@Input() displayClass?: string         // Tailwind classes shared by display text and input
                                       // (controls font size/weight per callsite)
@Output() commit: EventEmitter<string> // emits trimmed value on blur/Enter, including ""
```

### Internal state

- `isEditing` signal — toggles between display and input template
- `draft` signal — the in-progress string while editing

### Behavior details

- Clicking display text or placeholder sets `isEditing(true)`, sets `draft` to current `value ?? ""`, schedules `afterNextRender` focus.
- Blur or Enter: trim draft, emit via `(commit)`, set `isEditing(false)`.
- Escape: set `isEditing(false)`, no emit.
- Tab: `EditableText` does **not** intercept Tab — it lets the event propagate so the parent can handle focus chaining. The People view currently tabs from one unnamed subject to the next; this logic stays in `PeopleViewComponent` by listening to `(keydown.tab)` on the host or by handling it in `onNameCommit`.
- The input uses `border-b border-primary outline-none bg-transparent` internally so it looks consistent everywhere; `displayClass` is layered on top for per-context font styling.
- `cursor-text` is applied to the display element.
- When `value` is null/empty after a commit, the parent converts `""` → `null` in the API call as needed; `EditableText` re-renders the placeholder once `[value]` comes back null.

### Error recovery

The parent holds the source-of-truth signal. On API error or naming conflict, the parent reverts its signal. Because `[value]` is an input binding, `EditableText` reflects the reverted value automatically — no error state inside the component.

## People View — Card Restructure

The card's `<a [routerLink]>` splits into two independent click targets:

```
<div class="group flex flex-col items-center gap-3 ...">
  <a [routerLink]="['/subject', subject.id]">     ← avatar circle only
    <img ... />
  </a>
  <app-editable-text                              ← name area; no routing
    [value]="subject.name"
    placeholder="+ Add a name"
    placeholderClass="text-xs text-muted-foreground opacity-0 group-hover:opacity-100"
    displayClass="text-sm font-medium text-center"
    (commit)="onNameCommit(subject, $event)"
  />
</div>
```

- **Avatar click** → navigates to `/subject/:id` (unchanged intent).
- **Name click** → inline edit, works for both named and unnamed subjects.
- `group` / `hover:scale-105` / `group-hover:border-accent` ring stay on the outer `<div>`.
- Existing `editingSubjectId`, `editingName`, `commitName`, `cancelEditing`, `onKeydown` signals and methods collapse into a single `onNameCommit(subject: Subject, value: string)` handler that calls `photoService.nameSubject()` and handles naming conflicts.

## Subject Detail — Header Name

The `@if (isEditingName())` block (h1 + pencil button ↔ input) is replaced:

```html
<app-editable-text
  [value]="detail()?.subject?.name ?? null"
  placeholder="Unnamed Person"
  placeholderClass="opacity-50"
  displayClass="text-2xl font-bold tracking-tight"
  (commit)="saveName($event)"
/>
```

- Pencil icon button removed.
- `isEditingName`, `editedName`, `isSavingName` signals removed from the component.
- `saveName()` retains the API call and conflict dialog logic; `cancelEdit()` is removed (cancel is handled inside `EditableText`).

## Testing

- **`EditableText` spec** — display→edit on click, commit on blur, commit on Enter, revert on Escape, commit of empty string emits `""`.
- **People view spec** — update existing name-input tests to click display text rather than hint span; assert avatar `<a>` still has correct `routerLink`; assert `onNameCommit` is called with empty string when cleared.
- **Subject detail spec** — if a spec exists, update to click display text instead of pencil icon.

## Files Affected

| File | Change |
|---|---|
| `src/app/components/editable-text/editable-text.component.ts` | **New** — component logic |
| `src/app/components/editable-text/editable-text.component.html` | **New** — template |
| `src/app/components/people-view/people-view.component.html` | Restructure card; use `app-editable-text` |
| `src/app/components/people-view/people-view.component.ts` | Remove edit signals; add `onNameCommit` |
| `src/app/components/subject-detail/subject-detail.component.html` | Replace name block with `app-editable-text` |
| `src/app/components/subject-detail/subject-detail.component.ts` | Remove edit signals; simplify `saveName` |
