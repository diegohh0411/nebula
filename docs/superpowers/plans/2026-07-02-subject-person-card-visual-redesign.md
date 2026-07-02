# Subject Person Card Visual Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore `SubjectPersonCardComponent` to a single full-bleed photo-overlay card (name, subtitle, tags, add-tag input all layered on the photo inside one dark scrim), replacing the current split photo+white-footer layout, while keeping all existing tag/name-editing behavior working exactly as-is.

**Architecture:** Pure template/CSS restructuring of one Angular component. `subject-person-card.component.html` collapses back to a single-element photo tile (matching the pre-tag-editing structure) with a bottom content stack absolutely positioned over the scrim. `subject-person-card.component.css` gets a stronger scrim gradient, glass-styled tag/input classes, `aspect-[3/4]` sizing, and a static-position hover effect (image zoom + glow) instead of the current `translateY` lift. No changes to `.component.ts` or `subject-tagging.composable.ts` — this is CSS/markup-only, and the existing `.spec.ts` must keep passing unmodified because it depends on specific class names (`.person-card-name`, `.person-card-meta input`, `.person-card-add-tag input`, `.person-card-tag-remove`, `.person-card-tag-error`).

**Tech Stack:** Angular 20 standalone components, Tailwind CSS utility classes (`@apply`), Vitest for component specs.

## Global Constraints

- Do not modify `subject-person-card.component.ts`, `subject-person-card.component.spec.ts`, or `subject-tagging.composable.ts` — this is a visual-only change (per spec Non-goals).
- Preserve every CSS class the spec file queries by selector: `.person-card-name`, `.person-card-meta` (must contain the name's `<input>` when editing), `.person-card-add-tag` (must contain the tag `<input>`), `.person-card-tag-remove`, `.person-card-tag-error`. Renaming or moving these breaks `subject-person-card.component.spec.ts`.
- Card returns to `w-56 aspect-[3/4]` (per spec Sizing section).
- Scrim must be a fixed rgba overlay independent of the app's light/dark theme (per spec Scrim section) — do not use theme-aware color tokens (`bg-card`, `text-foreground`, etc.) for the overlay background.
- Hover must not change `.person-card`'s box dimensions or position (no `transform: translate*` on `.person-card` itself) — only the photo (`.person-card-img`) zooms, plus a glow/ring cue (per spec Hover section).
- Respect the existing `@media (prefers-reduced-motion: no-preference)` guard for hover transitions.

---

### Task 1: Restore full-overlay card structure, scrim, and glass-styled tags/input

**Files:**
- Modify: `src/app/components/subject-person-card/subject-person-card.component.html`
- Modify: `src/app/components/subject-person-card/subject-person-card.component.css`
- Test: `src/app/components/subject-person-card/subject-person-card.component.spec.ts` (existing, unmodified — must still pass)

**Interfaces:**
- Consumes: existing `SubjectPersonCardComponent` inputs/outputs and `tagging` composable API (`tagging.name()`, `tagging.tags()`, `tagging.saveName()`, `tagging.newTagName`, `tagging.addTag()`, `tagging.removeTag()`, `tagging.tagError()`, `tagging.allTags()`, `tagging.onTagFocus()`) — unchanged, only the markup around these bindings moves.
- Produces: CSS classes `.person-card`, `.person-card-img`, `.person-card-placeholder`, `.person-card-scrim`, `.person-card-content`, `.person-card-meta`, `.person-card-name`, `.person-card-subtitle`, `.person-card-tags`, `.person-card-tag`, `.person-card-tag-remove`, `.person-card-add-tag`, `.person-card-add-tag-btn`, `.person-card-tag-error` — same names as before, consumed by Task 2's hover CSS and by the existing spec file.

- [ ] **Step 1: Run the existing spec to confirm current baseline passes**

Run: `npx vitest run src/app/components/subject-person-card/subject-person-card.component.spec.ts`
Expected: PASS (9 tests), confirms starting point before refactor.

- [ ] **Step 2: Replace the component template**

Replace the full contents of `src/app/components/subject-person-card/subject-person-card.component.html` with:

```html
<div
  class="person-card group"
  role="button"
  tabindex="0"
  (click)="navigate()"
  (keydown.enter)="navigate()"
  (keydown.space)="navigate()"
>
  @if (cropUrl(); as url) {
    <img [src]="url" [alt]="tagging.name() ?? 'Unnamed'" loading="lazy" decoding="async" class="person-card-img" />
  } @else {
    <span class="person-card-placeholder">👤</span>
  }

  <div class="person-card-scrim"></div>

  <div class="person-card-content" (click)="$event.stopPropagation()" (keydown)="$event.stopPropagation()">
    <div class="person-card-meta">
      <app-editable-text
        [value]="tagging.name()"
        placeholder="Unnamed"
        placeholderClass="opacity-70"
        displayClass="person-card-name"
        (commit)="tagging.saveName($event)"
      />
      @if (subtitle()) {
        <span class="person-card-subtitle">{{ subtitle() }}</span>
      }
    </div>

    @if (tagging.tags().length > 0) {
      <div class="person-card-tags">
        @for (tag of tagging.tags(); track tag.id) {
          <span class="person-card-tag">
            {{ tag.name }}
            <button type="button" class="person-card-tag-remove" (click)="tagging.removeTag(tag.id)" title="Remove tag">×</button>
          </span>
        }
      </div>
    }
    <div class="person-card-add-tag">
      <input
        hlmInput
        class="h-6 flex-1 text-[11px] px-2 py-0.5 bg-white/10 border-white/20 text-white placeholder:text-white/50 focus-visible:ring-white/30"
        placeholder="Add tag…"
        [attr.list]="'person-card-tags-list-' + match().subject.id"
        [value]="tagging.newTagName()"
        (input)="tagging.newTagName.set($any($event.target).value)"
        (focus)="tagging.onTagFocus()"
        (keydown.enter)="tagging.addTag()"
      />
      <datalist [id]="'person-card-tags-list-' + match().subject.id">
        @for (t of tagging.allTags(); track t.id) {
          <option [value]="t.name"></option>
        }
      </datalist>
      <button type="button" class="person-card-add-tag-btn" (click)="tagging.addTag()">Add</button>
    </div>
    @if (tagging.tagError()) {
      <span class="person-card-tag-error">{{ tagging.tagError() }}</span>
    }
  </div>
</div>

<app-confirm-merge-dialog
  [open]="tagging.nameConflict() !== null"
  [error]="tagging.mergeError()"
  (merge)="tagging.confirmMerge()"
  (cancel)="tagging.cancelMerge()"
/>
```

Note what changed vs. the current file: the old `.person-card-photo` wrapper div and the old `.person-card-tags-footer` wrapper div are both gone. Everything — name/subtitle, tags, add-tag input, tag error — now lives in one `.person-card-content` div that sits directly on `.person-card` (which itself becomes the `aspect-[3/4]` tile in Step 3). The `<img>`/placeholder and `.person-card-scrim` are now direct children of `.person-card` instead of children of a nested photo div.

- [ ] **Step 3: Replace the component stylesheet**

Replace the full contents of `src/app/components/subject-person-card/subject-person-card.component.css` with:

```css
.person-card {
  @apply relative flex flex-col w-56 aspect-[3/4] rounded-2xl overflow-hidden flex-shrink-0
         border border-white/10 shadow-lg bg-muted text-left
         focus:outline-none focus-visible:ring-2 focus-visible:ring-ring;
}

.person-card-img {
  @apply absolute inset-0 w-full h-full object-cover transition-transform duration-200 ease-out;
}

.person-card-placeholder {
  @apply absolute inset-0 flex items-center justify-center text-5xl text-muted-foreground;
}

.person-card-scrim {
  @apply absolute inset-0 pointer-events-none;
  background: linear-gradient(to top,
    rgba(8, 8, 11, 0.95) 0%,
    rgba(8, 8, 11, 0.75) 35%,
    rgba(8, 8, 11, 0.25) 60%,
    transparent 80%);
}

.person-card-content {
  @apply absolute inset-x-0 bottom-0 flex flex-col gap-1.5 p-3;
}

.person-card-meta { @apply flex flex-col; }

.person-card-name {
  @apply text-[15px] font-semibold text-white tracking-tight truncate block;
}

.person-card-subtitle {
  @apply text-[12px] text-white/70 tracking-tight truncate;
}

.person-card-tags { @apply flex flex-wrap gap-1.5; }

.person-card-tag {
  @apply inline-flex items-center gap-1 text-[11px] font-medium text-white
         px-2 py-0.5 rounded-full bg-white/15 backdrop-blur-sm border border-white/20;
}

.person-card-tag-remove {
  @apply text-white/70 hover:text-destructive leading-none;
}

.person-card-add-tag { @apply flex items-center gap-1.5; }

.person-card-add-tag-btn {
  @apply text-[11px] text-white hover:text-white/80 shrink-0;
}

.person-card-tag-error { @apply text-[11px] text-destructive; }
```

Hover rules are intentionally omitted here — they're added in Task 2.

- [ ] **Step 4: Run the spec to verify it still passes unmodified**

Run: `npx vitest run src/app/components/subject-person-card/subject-person-card.component.spec.ts`
Expected: PASS (same 9 tests as Step 1), confirming the restructure didn't break name edit, tag add/remove, merge dialog, or navigation behavior.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/subject-person-card/subject-person-card.component.html src/app/components/subject-person-card/subject-person-card.component.css
git commit -m "refactor(ui): collapse subject person card back to full-overlay tile"
```

---

### Task 2: Replace translateY hover lift with static-position zoom + glow

**Files:**
- Modify: `src/app/components/subject-person-card/subject-person-card.component.css`

**Interfaces:**
- Consumes: `.person-card` and `.person-card-img` classes produced by Task 1.
- Produces: nothing consumed by later tasks — this is the final task in the plan.

- [ ] **Step 1: Add the hover rule**

Append to `src/app/components/subject-person-card/subject-person-card.component.css`:

```css
@media (prefers-reduced-motion: no-preference) {
  .person-card {
    transition: box-shadow 150ms ease-out;
  }
  .person-card:hover,
  .person-card:focus-visible {
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35), 0 0 0 2px rgba(255, 255, 255, 0.2);
  }
  .person-card:hover .person-card-img,
  .person-card:focus-visible .person-card-img {
    transform: scale(1.05);
  }
}
```

This replaces the old `.person-card:hover { transform: translateY(-4px); }` rule (already removed in Task 1's stylesheet rewrite — it's not present in the Task 1 CSS). `.person-card` itself never gets a `transform`, so its layout box and position are unaffected; only the clipped, absolutely-positioned `.person-card-img` scales, and the box-shadow/ring provides the glow cue. `overflow-hidden` on `.person-card` (already present) clips the zoomed image so it can't visually spill outside the tile.

- [ ] **Step 2: Manual visual check**

Run: `npx ng serve` (or the project's existing dev-server command), navigate to a page rendering `app-subject-person-card` (e.g. the reports view), and hover a card.
Expected: card stays in place (no shift/jump), photo zooms in slightly, a soft white glow/shadow appears around the card edge, no white/light-colored footer is visible at any point, and this holds in both light and dark app theme.

- [ ] **Step 3: Run full spec suite once more**

Run: `npx vitest run src/app/components/subject-person-card/subject-person-card.component.spec.ts`
Expected: PASS (still 9 tests) — hover CSS doesn't touch any behavior the spec exercises.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/subject-person-card/subject-person-card.component.css
git commit -m "style(ui): replace subject card hover lift with static zoom + glow"
```
