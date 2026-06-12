# UI/UX Polish Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land a focused front-end polish pass on the Nebula Angular app — fix the test environment, centralize scrolling, restyle subject cards, fix people-naming bugs, adopt a shared Spartan input, add collapsible relative-date groups, clean up the sidebar/header, and surface processing ETA.

**Architecture:** Pure front-end work in `src/app/`. The sidebar stays fixed and each routed view becomes its own vertical scroll owner. A new shared `SubjectPersonCard` standalone component renders the gradient-scrim person card (loading face crops via the existing `PhotoService.getFaceCrop` → `thumbnailUrl` pattern) and is consumed by both the gallery search row and the Tags view. Collapsible date groups are driven by a `collapsedDates` signal in `PhotoService`. Naming-bug fixes are local signal/focus corrections.

**Tech Stack:** Angular 20 (standalone components, signals, `OnPush`, `@if`/`@for`), Tailwind 3 + `@apply`, Spartan-NG (`@spartan-ng/helm`/`brain`), Angular CDK virtual scroll, Vitest + jsdom, Tauri.

**Spec:** `docs/superpowers/specs/2026-06-12-ui-ux-polish-pass-design.md`

**Conventions for every task:** Run the full suite with `pnpm test` (vitest, single run). The branch is `feat/ui-ux-polish-pass` (already created; the design spec is already committed there). Commit after each task with the message shown. Keep existing `data-testid` attributes intact.

---

## File map

**Create:**
- `src/app/components/subject-person-card/subject-person-card.component.ts` — shared gradient-scrim person card (loads its own crop)
- `src/app/components/subject-person-card/subject-person-card.component.html`
- `src/app/components/subject-person-card/subject-person-card.component.css`
- `src/app/components/subject-person-card/subject-person-card.component.spec.ts`
- `src/app/libs/ui/input/...` — Spartan helm input (generated via CLI)

**Modify:**
- `src/test-setup.ts` — jsdom `IntersectionObserver`/`ResizeObserver` stubs (Task 0)
- `src/app/app.component.ts` — shell scroll ownership (Task 1)
- `src/app/components/settings/settings.component.html` + `.css` — scroll + section spacing (Task 2)
- `src/app/components/timeline-scrubber/timeline-scrubber.component.css` (+ `.ts` if needed) — de-clutter (Task 3)
- `src/app/components/search-bar/search-bar.component.html` + `.css` — remove app title (Task 4), ETA (Task 11)
- `src/app/components/sidebar/sidebar.component.html` + `.css` — grouping (Task 5)
- `src/app/components/editable-text/editable-text.component.ts` + `.spec.ts` — focus on every edit (Task 6)
- `src/app/components/subject-detail/subject-detail.component.ts` — immutable name update (Task 7)
- `src/app/components/gallery/gallery.component.html` + `.css` — People row inside viewport + use card (Task 9)
- `src/app/components/tags-view/tags-view.component.html` + `.ts` — use card (Task 9)
- `src/app/services/photo.service.ts` + `src/app/models/models.ts` — relative labels + collapse (Task 10)
- `src/app/components/subject-detail/subject-detail.component.html`, `tags-view.component.html` — adopt helm input (Task 8)

---

## Task 0: Fix the pre-existing test failures

`pnpm test` reports 9 failures, all in `people-view.component.spec.ts`, from `ReferenceError: IntersectionObserver is not defined` in `PeopleViewComponent.ngAfterViewInit`. jsdom lacks the global. Add no-op `IntersectionObserver`/`ResizeObserver` stubs to the shared test setup.

**Files:**
- Modify: `src/test-setup.ts`

- [ ] **Step 1: Confirm the failure exists**

Run: `pnpm test`
Expected: `9 failed | 45 passed`, errors mentioning `IntersectionObserver is not defined`.

- [ ] **Step 2: Add observer stubs to the test setup**

Replace the contents of `src/test-setup.ts` with:

```ts
import '@analogjs/vitest-angular/setup-zone';
import { setupTestBed } from '@analogjs/vitest-angular/setup-testbed';

// jsdom provides neither IntersectionObserver nor ResizeObserver; several
// components (people-view, gallery, photo-grid, lightbox) construct them in
// lifecycle hooks. Provide inert stubs so component mounts don't throw.
class NoopObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
  takeRecords(): [] { return []; }
}

if (!('IntersectionObserver' in globalThis)) {
  (globalThis as unknown as { IntersectionObserver: unknown }).IntersectionObserver = NoopObserver;
}
if (!('ResizeObserver' in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = NoopObserver;
}

setupTestBed({ zoneless: false });
```

- [ ] **Step 3: Verify the whole suite passes**

Run: `pnpm test`
Expected: `0 failed`, all tests passing (54 passed).

- [ ] **Step 4: Commit**

```bash
git add src/test-setup.ts
git commit -m "test: stub IntersectionObserver/ResizeObserver in jsdom setup

Fixes 9 pre-existing people-view spec failures caused by jsdom lacking
IntersectionObserver."
```

---

## Task 1: Shell owns scrolling, sidebar fixed

Make the routed content column the single vertical scroll owner so no route relies on document scroll and the sidebar never scrolls away.

**Files:**
- Modify: `src/app/app.component.ts`

- [ ] **Step 1: Update the shell template**

In `src/app/app.component.ts`, the `template` is currently:

```ts
template: `
  <div class="flex h-screen bg-background text-foreground overflow-hidden">
    <app-sidebar class="flex-shrink-0" />
    <div class="flex flex-col flex-1 min-w-0 h-full">
      <router-outlet />
    </div>
  </div>
`,
```

Change the content column so it both fills height and contains overflow, giving routed views a bounded box to scroll within:

```ts
template: `
  <div class="flex h-screen bg-background text-foreground overflow-hidden">
    <app-sidebar class="flex-shrink-0" />
    <main class="flex flex-col flex-1 min-w-0 h-full overflow-hidden">
      <router-outlet />
    </main>
  </div>
`,
```

(Routed components must themselves be `h-full min-h-0` flex columns whose own region scrolls — gallery already is via `:host`; settings is fixed in Task 2.)

- [ ] **Step 2: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Step 3: Commit**

```bash
git add src/app/app.component.ts
git commit -m "fix(layout): content column contains overflow so routes own their scroll"
```

> Manual check (not a test): run `pnpm tauri dev`, resize the window — no document-level scrollbar appears and the sidebar stays put.

---

## Task 2: Settings view scrolls + even section spacing

`.settings-container` has no height/overflow (content clipped) and `.settings-content` has no `gap` (sections flush).

**Files:**
- Modify: `src/app/components/settings/settings.component.css`

- [ ] **Step 1: Make the container scrollable and space the sections**

In `src/app/components/settings/settings.component.css`, replace the `.settings-container` rule:

```css
.settings-container {
  @apply p-8 max-w-[640px] mx-auto;
}
```

with a version that fills the column and scrolls, and add a `.settings-content` spacing rule:

```css
.settings-container {
  @apply p-8 pb-16 max-w-[640px] mx-auto w-full h-full overflow-y-auto;
}

.settings-content {
  @apply flex flex-col gap-10;
}
```

- [ ] **Step 2: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Step 3: Commit**

```bash
git add src/app/components/settings/settings.component.css
git commit -m "fix(settings): scrollable container and even section spacing"
```

> Manual check: open Settings — you can scroll to the bottom of "People Recognition", and the gap between the two sections matches the header gap.

---

## Task 3: De-clutter the timeline scrubber (kill the "double scrollbar")

The scrubber is an always-on `fixed`, full-height, blurred bar layered over the real CDK scrollbar. Make it reveal on hover only and stop reading as a second scrollbar.

**Files:**
- Modify: `src/app/components/timeline-scrubber/timeline-scrubber.component.css`

- [ ] **Step 1: Make the scrubber idle-invisible and hover-revealed**

In `src/app/components/timeline-scrubber/timeline-scrubber.component.css`, replace the `:host` rule:

```css
:host {
  @apply fixed right-0 top-16 bottom-0 w-8 z-40 bg-background/20 backdrop-blur-sm
         border-l border-border/50 hover:bg-background/80 transition-colors duration-200;
}
```

with one that is transparent and chrome-free at rest, only surfacing affordances on hover:

```css
:host {
  @apply fixed right-0 top-16 bottom-0 w-6 z-40 bg-transparent
         opacity-0 hover:opacity-100 transition-opacity duration-200;
}

:host:hover {
  @apply bg-background/70 backdrop-blur-sm border-l border-border/50;
}
```

- [ ] **Step 2: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Step 3: Commit**

```bash
git add src/app/components/timeline-scrubber/timeline-scrubber.component.css
git commit -m "fix(gallery): scrubber reveals on hover, no longer mimics a second scrollbar"
```

> Manual check: at rest only the native scrollbar shows; hovering the right edge reveals the date scrubber.

---

## Task 4: Remove the app title/star from the gallery header

**Files:**
- Modify: `src/app/components/search-bar/search-bar.component.html`, `search-bar.component.css`

- [ ] **Step 1: Delete the app-name block**

In `src/app/components/search-bar/search-bar.component.html`, remove the entire `<span class="app-name"> … Nebula </span>` block (the star `<svg class="app-icon">` plus the "Nebula" text) at the top of `<header class="search-bar">`. Leave the search input wrap and processing badge.

- [ ] **Step 2: Remove the now-unused CSS**

In `src/app/components/search-bar/search-bar.component.css`, delete the `.app-name` and `.app-icon` rules. If `.search-bar` uses a leading gap that now leaves the input indented oddly, adjust its left padding so the search field sits cleanly at the start.

- [ ] **Step 3: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/search-bar/search-bar.component.html src/app/components/search-bar/search-bar.component.css
git commit -m "fix(header): remove Nebula title/star, leave search + processing badge"
```

---

## Task 5: Honest sidebar grouping

"Folders" currently heads non-folder nav (All Photos/People/Tags) too. Regroup so "Folders" only labels real folders.

**Files:**
- Modify: `src/app/components/sidebar/sidebar.component.html`, `sidebar.component.css`

- [ ] **Step 1: Re-label the groups**

In `src/app/components/sidebar/sidebar.component.html`:
- Change the header `<span class="sidebar-title">Folders</span>` to `<span class="sidebar-title">Library</span>` (this now heads All Photos / People / Tags).
- After the existing `<div class="sidebar-divider"></div>` (which sits above the `@for (folder of photos.folders())` loop), insert a folders heading immediately before the loop:

```html
<div class="sidebar-divider"></div>

<span class="sidebar-title sidebar-title--sub">Folders</span>

@for (folder of photos.folders(); track folder.id) {
```

- [ ] **Step 2: Style the sub-heading**

In `src/app/components/sidebar/sidebar.component.css`, add a modifier so the in-list heading reads as a section label (reuse the existing `.sidebar-title` look, add top spacing):

```css
.sidebar-title--sub {
  @apply block mt-2 mb-1;
}
```

(If `.sidebar-title` is scoped to the header only, copy its font/size utilities onto `.sidebar-title--sub` so both look consistent.)

- [ ] **Step 3: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/sidebar/sidebar.component.html src/app/components/sidebar/sidebar.component.css
git commit -m "fix(sidebar): 'Folders' heading now labels only real folders (Library group above)"
```

---

## Task 6: `editable-text` focuses on every edit (single-click)

Focus is applied via `afterNextRender` registered once in the constructor, so only the first edit ever focuses. Re-focus on every entry into edit mode.

**Files:**
- Modify: `src/app/components/editable-text/editable-text.component.ts`
- Test: `src/app/components/editable-text/editable-text.component.spec.ts`

- [ ] **Step 1: Write a failing single-click focus test**

Add to `src/app/components/editable-text/editable-text.component.spec.ts` (inside the existing describe; it already renders the component — match its existing harness for creating the fixture):

```ts
it('focuses the input on a single click into edit mode', async () => {
  const fixture = TestBed.createComponent(EditableTextComponent);
  fixture.componentInstance.placeholder = '+ Add a name';
  fixture.detectChanges();

  // First entry into edit mode (single click on the placeholder span).
  const trigger = fixture.nativeElement.querySelector('[role="button"]') as HTMLElement;
  trigger.click();
  fixture.detectChanges();
  await fixture.whenStable();

  const input = fixture.nativeElement.querySelector('input') as HTMLInputElement;
  expect(input).toBeTruthy();
  expect(document.activeElement).toBe(input);
});
```

- [ ] **Step 2: Run it and watch it fail**

Run: `pnpm test`
Expected: the new test FAILS (`document.activeElement` is not the input — focus never applied on first programmatic entry).

- [ ] **Step 3: Re-focus on every edit entry via an injector-scoped afterNextRender**

In `src/app/components/editable-text/editable-text.component.ts`:

- Add `Injector` to the `@angular/core` import and remove the now-unused `_focusPending` signal.
- Inject an injector: `private injector = inject(Injector);`
- Replace the constructor's one-shot `afterNextRender` with a reusable focus helper and call it wherever edit mode is entered.

Resulting relevant members:

```ts
import {
  Component, Input, Output, EventEmitter, signal,
  ViewChild, ElementRef, afterNextRender, inject, Injector,
} from '@angular/core';

// ...

@Input() set startEditing(trigger: boolean) {
  if (trigger && !this.isEditing()) {
    this.draft.set(this.value ?? '');
    this.isEditing.set(true);
    this.focusInput();
  }
}

protected isEditing = signal(false);
protected draft = signal('');
private injector = inject(Injector);

@ViewChild('inputEl') private inputRef?: ElementRef<HTMLInputElement>;

// (delete the constructor entirely — its only job was the one-shot focus)

protected startEdit(): void {
  this.draft.set(this.value ?? '');
  this.isEditing.set(true);
  this.focusInput();
}

private focusInput(): void {
  // Runs after the @if renders the input, on every edit entry (afterNextRender
  // accepts an injector so it can be scheduled outside the constructor).
  afterNextRender(
    () => {
      const el = this.inputRef?.nativeElement;
      el?.focus();
      el?.select();
    },
    { injector: this.injector },
  );
}
```

- [ ] **Step 4: Run tests to verify focus test passes**

Run: `pnpm test`
Expected: `0 failed`, including the new single-click focus test.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/editable-text/editable-text.component.ts src/app/components/editable-text/editable-text.component.spec.ts
git commit -m "fix(editable-text): focus input on every edit entry, not just the first"
```

---

## Task 7: Subject name updates immediately (immutable signal write)

`saveName()` mutates the `detail` object and returns the same reference, so the signal never notifies. Write a new reference.

**Files:**
- Modify: `src/app/components/subject-detail/subject-detail.component.ts`

- [ ] **Step 1: Make the update immutable**

In `src/app/components/subject-detail/subject-detail.component.ts`, in `saveName()`, replace:

```ts
this.detail.update((d) => {
  if (d) d.subject.name = name;
  return d;
});
```

with:

```ts
this.detail.update((d) =>
  d ? { ...d, subject: { ...d.subject, name } } : d,
);
```

- [ ] **Step 2: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Step 3: Commit**

```bash
git add src/app/components/subject-detail/subject-detail.component.ts
git commit -m "fix(subject-detail): emit new detail reference so renamed name renders immediately"
```

> Manual check: rename a subject in the detail view — the header updates on Enter/blur with no navigation round-trip.

---

## Task 8: Shared Spartan input (consistent text fields)

Generate a helm input and adopt it for the bespoke "Add tag" / "new tag" fields.

**Files:**
- Create: `src/app/libs/ui/input/...` (via Spartan CLI)
- Modify: `src/app/components/subject-detail/subject-detail.component.html` (+ `.ts` import), `src/app/components/tags-view/tags-view.component.html` (+ `.ts` import)

- [ ] **Step 1: Generate the helm input component**

Run (from repo root):

```bash
pnpm dlx @spartan-ng/cli@0.0.1-alpha.668 ui add input
```

Expected: a new `input` directory under `src/app/libs/ui/input` exporting an `hlmInput` directive, importable via the `@spartan-ng/helm` alias (matching the existing `button`/`card` helm components). If the CLI prompts, accept the default path `src/app/libs/ui`.

- [ ] **Step 2: Adopt it in subject-detail "Add tag"**

In `src/app/components/subject-detail/subject-detail.component.html`, find the "Add tag" text `<input>` (bound to `newTagName`) and apply the `hlmInput` directive to it, removing the ad-hoc utility classes that duplicated input styling. Import the input directive in `subject-detail.component.ts`'s `imports` array (same import style as the existing helm usage). Keep the existing `[value]`/`(input)`/`(keydown.enter)` bindings and the adjacent add button; ensure they align (same height).

- [ ] **Step 3: Adopt it in tags-view "new tag"**

In `src/app/components/tags-view/tags-view.component.html`, replace the bespoke `class="flex-1 text-sm border border-border rounded px-3 py-1.5 bg-background"` on the "New tag name…" input with the `hlmInput` directive (keep `flex-1` for layout). Import the directive in `tags-view.component.ts`.

- [ ] **Step 4: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`. (If a tags-view/subject-detail spec asserts the old class string, update it to assert behavior/`data-testid` instead.)

- [ ] **Step 5: Commit**

```bash
git add src/app/libs/ui/input src/app/components/subject-detail src/app/components/tags-view components.json
git commit -m "feat(ui): add shared Spartan helm input; adopt for tag inputs"
```

---

## Task 9: `SubjectPersonCard` shared component (gradient scrim) + adoption

One reusable card, used by the gallery search "People" row and the Tags view. Loads its own face crop; renders Design A.

**Files:**
- Create: `src/app/components/subject-person-card/subject-person-card.component.ts`, `.html`, `.css`, `.spec.ts`
- Modify: `src/app/components/gallery/gallery.component.html`, `gallery.component.css`; `src/app/components/tags-view/tags-view.component.html`, `tags-view.component.ts`

- [ ] **Step 1: Write the component spec first**

Create `src/app/components/subject-person-card/subject-person-card.component.spec.ts`:

```ts
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { SubjectPersonCardComponent } from './subject-person-card.component';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch } from '../../models/models';

class PhotoServiceStub {
  getFaceCrop = vi.fn().mockResolvedValue('/cache/face-7.png');
  thumbnailUrl = vi.fn((p: string | null) => (p ? `asset://${p}` : null));
}

function match(over: Partial<SubjectMatch['subject']> = {}): SubjectMatch {
  return {
    subject: { id: 1, name: 'Sofía', thumbnail_face_id: 7, type: 'person', added_at: 0, ...over },
    tags: [],
  };
}

describe('SubjectPersonCardComponent', () => {
  beforeEach(() => {
    TestBed.configureTestingModule({
      imports: [SubjectPersonCardComponent],
      providers: [provideRouter([]), { provide: PhotoService, useClass: PhotoServiceStub }],
    });
  });

  it('loads and renders the face crop image when thumbnail_face_id is present', async () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match());
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    const img = fixture.nativeElement.querySelector('img') as HTMLImageElement | null;
    expect(img).toBeTruthy();
    expect(img!.getAttribute('src')).toBe('asset:///cache/face-7.png');
  });

  it('renders the placeholder (no img) when there is no thumbnail_face_id', async () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match({ thumbnail_face_id: null }));
    fixture.detectChanges();
    await fixture.whenStable();
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('img')).toBeNull();
    expect(fixture.nativeElement.textContent).toContain('👤');
  });

  it('shows "Unnamed" when the subject has no name', () => {
    const fixture = TestBed.createComponent(SubjectPersonCardComponent);
    fixture.componentRef.setInput('match', match({ name: null }));
    fixture.detectChanges();
    expect(fixture.nativeElement.textContent).toContain('Unnamed');
  });
});
```

- [ ] **Step 2: Run the spec to verify it fails**

Run: `pnpm test`
Expected: FAIL — `SubjectPersonCardComponent` does not exist yet.

- [ ] **Step 3: Implement the component class**

Create `src/app/components/subject-person-card/subject-person-card.component.ts`:

```ts
import {
  Component, ChangeDetectionStrategy, inject, input, output, signal, OnInit,
} from '@angular/core';
import { Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch } from '../../models/models';

@Component({
  selector: 'app-subject-person-card',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './subject-person-card.component.html',
  styleUrl: './subject-person-card.component.css',
})
export class SubjectPersonCardComponent implements OnInit {
  private photos = inject(PhotoService);
  private router = inject(Router);

  /** The subject + its tags to render. */
  readonly match = input.required<SubjectMatch>();
  /** When true, show a "Remove" affordance (Tags view). */
  readonly removable = input(false);
  /** Emitted when the Remove affordance is clicked (subject id). */
  readonly remove = output<number>();

  protected readonly cropUrl = signal<string | null>(null);

  async ngOnInit(): Promise<void> {
    const subject = this.match().subject;
    if (!subject.thumbnail_face_id) return;
    try {
      const path = await this.photos.getFaceCrop(subject.thumbnail_face_id);
      this.cropUrl.set(this.photos.thumbnailUrl(path));
    } catch {
      /* fall back to placeholder */
    }
  }

  protected get displayName(): string {
    return this.match().subject.name ?? 'Unnamed';
  }

  protected navigate(): void {
    void this.router.navigate(['/subject', this.match().subject.id]);
  }

  protected onRemove(event: Event): void {
    event.stopPropagation();
    this.remove.emit(this.match().subject.id);
  }
}
```

- [ ] **Step 4: Implement the template (Design A — gradient scrim)**

Create `src/app/components/subject-person-card/subject-person-card.component.html`:

```html
<button type="button" class="person-card group" (click)="navigate()">
  @if (cropUrl(); as url) {
    <img [src]="url" [alt]="displayName" loading="lazy" decoding="async" class="person-card-img" />
  } @else {
    <span class="person-card-placeholder">👤</span>
  }

  <div class="person-card-scrim"></div>

  <div class="person-card-meta">
    @if (match().tags.length > 0) {
      <div class="person-card-tags">
        @for (tag of match().tags.slice(0, 2); track tag.id) {
          <span class="person-card-tag">{{ tag.name }}</span>
        }
        @if (match().tags.length > 2) {
          <span class="person-card-tag">+{{ match().tags.length - 2 }}</span>
        }
      </div>
    }
    <span class="person-card-name">{{ displayName }}</span>
  </div>

  @if (removable()) {
    <span class="person-card-remove" role="button" (click)="onRemove($event)">Remove</span>
  }
</button>
```

- [ ] **Step 5: Implement the styles (Design A)**

Create `src/app/components/subject-person-card/subject-person-card.component.css`:

```css
.person-card {
  @apply relative block w-40 aspect-[3/4] rounded-2xl overflow-hidden flex-shrink-0
         border border-white/10 shadow-lg bg-muted text-left
         transition-transform duration-150 ease-out
         focus:outline-none focus-visible:ring-2 focus-visible:ring-ring;
}

@media (prefers-reduced-motion: no-preference) {
  .person-card:hover { transform: translateY(-4px); }
  .person-card:hover .person-card-img { transform: scale(1.05); }
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
    rgba(8, 8, 11, 0.92) 0%, rgba(8, 8, 11, 0.45) 28%, transparent 55%);
}

.person-card-meta {
  @apply absolute left-3 right-3 bottom-3 flex flex-col gap-1.5;
}

.person-card-tags { @apply flex gap-1.5; }

.person-card-tag {
  @apply text-[10px] font-semibold text-white px-2 py-0.5 rounded-full
         bg-white/20 backdrop-blur-sm;
}

.person-card-name {
  @apply text-[15px] font-semibold text-white tracking-tight truncate;
}

.person-card-remove {
  @apply absolute top-2 right-2 text-[11px] text-white/90 px-2 py-0.5 rounded-full
         bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity
         hover:bg-destructive;
}
```

- [ ] **Step 6: Run the spec to verify it passes**

Run: `pnpm test`
Expected: `0 failed`, including the three `SubjectPersonCardComponent` tests.

- [ ] **Step 7: Add a `people` virtual-row variant (so the People row scrolls inside the viewport — spec §2a)**

The People row currently sits *outside* `<cdk-virtual-scroll-viewport>`, so it stays pinned. Make it a leading virtual row instead. In `src/app/models/models.ts`, extend the `VirtualRow` union with a people variant (import `SubjectMatch` at the top of the file if not already imported there — it is defined in this same file, so just reference it):

```ts
export type VirtualRow =
  | { type: 'people'; matches: SubjectMatch[] }
  | { type: 'header'; label: string; date: string }
  | { type: 'row'; images: (Image | SearchResult)[]; rowHeight: number };
```

(Task 10 later adds `collapsed`/`count` to the `header` variant — leave the `people` variant as-is then.)

- [ ] **Step 8: Prepend the people row when a search has subject matches**

In `src/app/services/photo.service.ts`, update the `virtualRows` computed to prepend a `people` row when search is active and there are matches (both `searchResults` and `subjectMatches` are signals on this service):

```ts
readonly virtualRows = computed<VirtualRow[]>(() => {
  const base = flattenToVirtualRowsJustified(
    this.dayGroups(),
    this.viewportWidth(),
    this.targetRowHeight(),
  );
  const matches = this.subjectMatches();
  if (this.searchResults() !== null && matches.length > 0) {
    return [{ type: 'people', matches }, ...base];
  }
  return base;
});
```

- [ ] **Step 9: Render the people row inside the viewport; delete the pinned block**

In `src/app/components/gallery/gallery.component.html`:

1. **Delete** the entire pinned `@if (photos.searchResults() !== null && photos.subjectMatches().length > 0) { <div class="subjects-row"> … </div> }` block that sits *above* `<cdk-virtual-scroll-viewport>`.
2. Inside the viewport's `*cdkVirtualFor` item, add a `people` branch before the header branch:

```html
<div *cdkVirtualFor="let row of photos.virtualRows(); trackBy: trackRow" class="viewport-item">
  @if (row.type === 'people') {
    <div class="subjects-row">
      <span class="subjects-row-label">People</span>
      <div class="subjects-row-cards">
        @for (match of row.matches; track match.subject.id) {
          <app-subject-person-card [match]="match" />
        }
      </div>
    </div>
  } @else if (row.type === 'header') {
    <div class="day-header">{{ row.label }}</div>
  } @else {
    <app-photo-grid [images]="row.images" [rowHeight]="row.rowHeight" />
  }
</div>
```

In `gallery.component.ts`: add `SubjectPersonCardComponent` to `imports`; update `trackRow` to handle the new variant (add `if (row.type === 'people') return 'people-row';` at the top). The viewport is `autosize`, so the people row's height is measured automatically — no `getRowHeight` change needed.

In `gallery.component.css`: keep `.subjects-row`, `.subjects-row-label`, `.subjects-row-cards`; remove the now-unused `.subject-card`, `.subject-avatar`, `.subject-avatar-placeholder`, `.subject-name`, `.subject-tags`, `.subject-tag` rules (dead after the card swap).

- [ ] **Step 10: Adopt the card in the Tags view**

In `src/app/components/tags-view/tags-view.component.html`, replace the `@for (match of tagSubjects())` inner `<div class="subject-card"> … </div>` with:

```html
@for (match of tagSubjects(); track match.subject.id) {
  <app-subject-person-card [match]="match" [removable]="true" (remove)="removeSubjectFromTag($event)" />
}
```

In `tags-view.component.ts`, add `SubjectPersonCardComponent` to `imports`. Confirm `removeSubjectFromTag` accepts a subject id (it already does: `removeSubjectFromTag(match.subject.id)`).

- [ ] **Step 11: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`. (Update any tags-view/gallery spec that asserted the old `.subject-card` markup to target the new component or a `data-testid`. If a spec builds a header `VirtualRow` literal it is unaffected; the `people` variant is new and additive.)

- [ ] **Step 12: Commit**

```bash
git add src/app/components/subject-person-card src/app/components/gallery src/app/components/tags-view
git commit -m "feat(people): shared SubjectPersonCard (gradient scrim) with real face crops in search + tags"
```

---

## Task 10: Collapsible, relative-date photo groups

Add "This Week"/"Last Week" relative labels and let day headers collapse their photos.

**Files:**
- Modify: `src/app/models/models.ts` (extend the header `VirtualRow`)
- Modify: `src/app/services/photo.service.ts` (relative labels, `collapsedDates` signal, skip collapsed rows)
- Modify: `src/app/components/gallery/gallery.component.html` (clickable header), `gallery.component.css`

- [ ] **Step 1: Extend the header row type**

In `src/app/models/models.ts`, add `collapsed`/`count` to the `header` variant (the `people` variant from Task 9 stays unchanged):

```ts
export type VirtualRow =
  | { type: 'people'; matches: SubjectMatch[] }
  | { type: 'header'; label: string; date: string; collapsed: boolean; count: number }
  | { type: 'row'; images: (Image | SearchResult)[]; rowHeight: number };
```

- [ ] **Step 2: Add relative-week labels in `groupByDay`**

In `src/app/services/photo.service.ts`, inside `groupByDay`, after computing `today`/`yesterday`, add week boundaries and extend the label branch:

```ts
const weekAgo = new Date(today);
weekAgo.setDate(today.getDate() - 7);
const twoWeeksAgo = new Date(today);
twoWeeksAgo.setDate(today.getDate() - 14);
```

and replace the label `else` branch:

```ts
if (d.getTime() === today.getTime()) {
  label = 'Today';
} else if (d.getTime() === yesterday.getTime()) {
  label = 'Yesterday';
} else if (d.getTime() > weekAgo.getTime()) {
  label = 'This Week';
} else if (d.getTime() > twoWeeksAgo.getTime()) {
  label = 'Last Week';
} else {
  label = d.toLocaleDateString('en-US', { year: 'numeric', month: 'long', day: 'numeric' });
}
```

(Multiple days can now share the "This Week"/"Last Week" label but keep distinct `date` keys — that's fine; headers remain per-day.)

- [ ] **Step 3: Add a `collapsedDates` signal and a toggle**

In `src/app/services/photo.service.ts`, near the other signals (e.g. after `targetRowHeight`), add:

```ts
/** ISO date keys whose photo rows are collapsed/hidden in the gallery. */
readonly collapsedDates = signal<Set<string>>(new Set());

toggleDateCollapsed(date: string): void {
  this.collapsedDates.update((set) => {
    const next = new Set(set);
    next.has(date) ? next.delete(date) : next.add(date);
    return next;
  });
}
```

- [ ] **Step 4: Honor collapse + counts when building rows**

In `src/app/services/photo.service.ts`, make `virtualRows` pass collapsed state into the flattener while keeping the Task 9 people-row prepend:

```ts
readonly virtualRows = computed<VirtualRow[]>(() => {
  const base = flattenToVirtualRowsJustified(
    this.dayGroups(),
    this.viewportWidth(),
    this.targetRowHeight(),
    this.collapsedDates(),
  );
  const matches = this.subjectMatches();
  if (this.searchResults() !== null && matches.length > 0) {
    return [{ type: 'people', matches }, ...base];
  }
  return base;
});
```

and update `flattenToVirtualRowsJustified`:

```ts
function flattenToVirtualRowsJustified(
  groups: DayGroup[],
  containerWidth: number,
  targetHeight: number,
  collapsed: Set<string> = new Set(),
): VirtualRow[] {
  const rows: VirtualRow[] = [];
  for (const group of groups) {
    const isCollapsed = collapsed.has(group.date);
    rows.push({
      type: 'header',
      label: group.label,
      date: group.date,
      collapsed: isCollapsed,
      count: group.images.length,
    });
    if (isCollapsed) continue;
    const justifiedRows = buildJustifiedRows(group.images, containerWidth, targetHeight, 4);
    for (const row of justifiedRows) {
      rows.push({ type: 'row', images: row.images, rowHeight: row.rowHeight });
    }
  }
  return rows;
}
```

- [ ] **Step 5: Make the header clickable with a chevron + count**

In `src/app/components/gallery/gallery.component.html`, replace the header branch:

```html
@if (row.type === 'header') {
  <div class="day-header">{{ row.label }}</div>
}
```

with:

```html
@if (row.type === 'header') {
  <button type="button" class="day-header" (click)="photos.toggleDateCollapsed(row.date)">
    <svg class="day-header-chevron" [class.is-collapsed]="row.collapsed"
         width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
      <polyline points="6 9 12 15 18 9" />
    </svg>
    <span>{{ row.label }}</span>
    <span class="day-header-count">{{ row.count }}</span>
  </button>
}
```

- [ ] **Step 6: Style the clickable header**

In `src/app/components/gallery/gallery.component.css`, extend the `.day-header` rule (it currently sets sticky/typography) to lay out as a row and add chevron/count styles:

```css
.day-header {
  @apply w-full gap-2;  /* add to existing flex/align rules already present */
}

.day-header-chevron {
  @apply transition-transform duration-150;
}

.day-header-chevron.is-collapsed {
  @apply -rotate-90;
}

.day-header-count {
  @apply ml-auto text-[11px] font-medium normal-case tracking-normal text-muted-foreground;
}
```

(The existing `.day-header` already has `display:flex; align-items:center;` — keep those; just ensure the button resets default button styling by relying on the existing utility classes.)

- [ ] **Step 7: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`. (If a photo.service or gallery spec constructs a header `VirtualRow` literal, add the new `collapsed`/`count` fields there.)

- [ ] **Step 8: Commit**

```bash
git add src/app/models/models.ts src/app/services/photo.service.ts src/app/components/gallery/gallery.component.html src/app/components/gallery/gallery.component.css
git commit -m "feat(gallery): relative-date labels + collapsible day groups"
```

> Manual check: clicking a date header hides/shows that day's photos; chevron rotates; recent groups read "Today/Yesterday/This Week/Last Week".

---

## Task 11: Processing badge shows activity + img/s + ETA

The badge hides everything until `images_per_sec >= 0.1`. Always show activity when there's a backlog, and add an ETA once a rate is known.

**Files:**
- Modify: `src/app/components/search-bar/search-bar.component.ts`, `search-bar.component.html`

- [ ] **Step 1: Add an `etaLabel` computed to the component**

In `src/app/components/search-bar/search-bar.component.ts`, add a `computed` import and an ETA helper that reads `pipelineStats()`:

```ts
import { Component, ChangeDetectionStrategy, inject, signal, effect, computed, OnDestroy } from '@angular/core';

// ...inside the class, near the other fields:
protected readonly etaLabel = computed<string | null>(() => {
  const { total_pending, images_per_sec } = this.photos.pipelineStats();
  if (total_pending <= 0 || images_per_sec < 0.1) return null;
  const seconds = Math.ceil(total_pending / images_per_sec);
  if (seconds < 60) return `~${seconds}s left`;
  const minutes = Math.ceil(seconds / 60);
  if (minutes < 60) return `~${minutes} min left`;
  const hours = Math.ceil(minutes / 60);
  return `~${hours} h left`;
});
```

- [ ] **Step 2: Show activity immediately + rate + ETA in the badge**

In `src/app/components/search-bar/search-bar.component.html`, the active badge branch currently reads:

```html
@if (badgeState() === 'active') {
  <span class="embed-badge-dot"></span>
  Processing {{ photos.pipelineStats().total_pending }} images
  @if (photos.pipelineStats().images_per_sec >= 0.1) {
    · {{ photos.pipelineStats().images_per_sec | number:'1.0-1' }} img/s
  }
}
```

Append the ETA after the img/s segment:

```html
@if (badgeState() === 'active') {
  <span class="embed-badge-dot"></span>
  Processing {{ photos.pipelineStats().total_pending }} images
  @if (photos.pipelineStats().images_per_sec >= 0.1) {
    · {{ photos.pipelineStats().images_per_sec | number:'1.0-1' }} img/s
  }
  @if (etaLabel(); as eta) {
    · {{ eta }}
  }
}
```

(The "Processing N images" text already shows immediately because the badge goes `active` as soon as `total_pending > 0` — see the constructor effect — so activity is visible before any rate is measured.)

- [ ] **Step 3: Verify suite still green**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/search-bar/search-bar.component.ts src/app/components/search-bar/search-bar.component.html
git commit -m "feat(processing): show activity immediately + img/s + estimated time remaining"
```

> Manual check: add a folder — badge immediately shows "Processing N images"; once throughput is known it appends "· X img/s · ~Y min left" that counts down.

---

## Final verification

- [ ] **Run the full suite once more**

Run: `pnpm test`
Expected: `0 failed`.

- [ ] **Manual smoke (`pnpm tauri dev`):** verify each acceptance bullet from the spec — single scrollbar, settings scroll, People row scrolls with grid + pretty cards with real faces, tags-view faces, single-click name focus, immediate rename, consistent tag inputs, collapsible relative-date groups, clean header, honest sidebar, processing ETA.

- [ ] **Open the PR** (do not merge to `main`); set the Notion epic status to **Ready for review**.

---

## Notes for the implementer

- `PhotoService.getFaceCrop(faceId)` returns a path; `PhotoService.thumbnailUrl(path)` converts it to a usable `src` (returns `null` for `null` input). The `SubjectPersonCard` loads per-instance on `ngOnInit` — fine for the modest counts in a search People row / tag subject list (no IntersectionObserver needed here).
- `SubjectMatch` shape: `{ subject: Subject, tags: Tag[] }`; `Subject.name` is nullable; `Subject.thumbnail_face_id` is nullable.
- Spec §2a (People row scrolls with the grid) is implemented in Task 9 Steps 7–9 by turning the People row into a leading `people` virtual row inside `<cdk-virtual-scroll-viewport>` and deleting the old pinned block. The viewport is `autosize`, so the row's height is measured — no fixed item-size wiring needed.
```
