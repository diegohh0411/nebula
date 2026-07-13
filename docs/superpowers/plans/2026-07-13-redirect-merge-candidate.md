# Redirect Merge Candidate to a Different Existing Subject — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user in the merge-review modal redirect a merge candidate to a different,
already-named subject (via a footer typeahead, or via an upgraded name-collision prompt),
re-target-then-confirm, without any backend changes and without auto-writing a `cannot_link`
between the bypassed original keep subject and the new target.

**Architecture:** All changes are confined to `MergeReviewComponent`
(`src/app/components/merge-review/merge-review.component.{ts,html,css}` +
`.spec.ts`). Three new signals (`showRedirectPicker`, `targetOverride`, `redirectSource`)
extend the existing footer state machine (`showExitConfirm`) as a third mutually-exclusive
mode. `mergeTarget` becomes redirect-aware. No new files, no backend changes, no new
services.

**Tech Stack:** Angular 17+ standalone components, signals/computed, Vitest + Angular
`TestBed`, existing `PhotoService`.

## Global Constraints

- No `src-tauri` changes (verified in the design spec — `merge_subjects` already accepts
  arbitrary target/source ids; see
  `docs/superpowers/specs/2026-07-13-redirect-merge-candidate-design.md`).
- Never call `dismissMergeSuggestion` anywhere in the redirect path (Part 3 of the spec —
  this is the mechanism by which a `cannot_link` constraint gets written, and it must never
  fire for the bypassed original A/C pair).
- Typeahead rows show avatar + name only — no face count in v1 (spec, Part 1).
- Face-count/backend changes, a "revert to original suggestion" button, and drag-and-drop
  are explicitly out of scope (spec, Out of scope).
- Follow the existing code style in `merge-review.component.ts`: `protected` for
  template-facing members, `signal()`/`computed()`, no `RxJS` introduced.

---

## File Map

- Modify: `src/app/components/merge-review/merge-review.component.ts` — new signals, redirect
  logic, structured collision error, deleted-target guard.
- Modify: `src/app/components/merge-review/merge-review.component.html` — third footer state
  (typeahead), upgraded collision error markup.
- Modify: `src/app/components/merge-review/merge-review.component.css` — combobox row
  styling, tertiary link styling.
- Modify: `src/app/components/merge-review/merge-review.component.spec.ts` — new test
  suites for every task below.

---

### Task 1: `mergeTarget` becomes redirect-aware (`targetOverride` / `redirectSource`)

Lay the state/logic foundation before any UI exists. `mergeTarget` must special-case a
pending redirect **before** falling back to the existing named/id tiebreak.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts:22-25` (the
  `MergeTarget` interface — unchanged shape), `:78-90` (the `mergeTarget` getter).
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Produces: `protected targetOverride = signal<Subject | null>(null)`,
  `protected redirectSource = signal<Subject | null>(null)`. Later tasks set these directly;
  no setter method is introduced (matches existing direct-signal-mutation style in this file,
  e.g. `showExitConfirm.set(...)`).
- Produces: `mergeTarget` getter behavior — when `targetOverride()` is non-null, returns
  `{ target: targetOverride()!, source: redirectSource()! }` and ignores `subjectA`/
  `subjectB` entirely.

- [ ] **Step 1: Write the failing test**

Add to `merge-review.component.spec.ts` (after the existing `mergeTarget` tests around line
103):

```ts
  it('mergeTarget uses targetOverride/redirectSource when a redirect is active', () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    component.suggestion = makeSuggestion(a, b); // normally: target=a, source=b

    const roberto = makeSubject(99, 'Roberto');
    component.targetOverride.set(roberto);
    component.redirectSource.set(b);

    expect(component.mergeTarget).toEqual({ target: roberto, source: b });
  });

  it('mergeTarget falls back to normal tiebreak when targetOverride is null', () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    component.suggestion = makeSuggestion(a, b);

    expect(component.targetOverride()).toBeNull();
    expect(component.mergeTarget).toEqual({ target: a, source: b });
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `Property 'targetOverride' does not exist on type 'MergeReviewComponent'`
(TypeScript compile error surfaced through Vitest).

- [ ] **Step 3: Implement the signals and update `mergeTarget`**

In `merge-review.component.ts`, add the two new signals next to the existing state signals
(after `showExitConfirm`, around line 67):

```ts
  protected showExitConfirm = signal(false);
  protected showRedirectPicker = signal(false);
  protected targetOverride = signal<Subject | null>(null);
  protected redirectSource = signal<Subject | null>(null);
```

Update the `mergeTarget` getter (lines 78-90):

```ts
  get mergeTarget(): MergeTarget | null {
    const override = this.targetOverride();
    if (override) {
      const source = this.redirectSource();
      if (!source) return null;
      return { target: override, source };
    }
    const subjectA = this.subjectA();
    const subjectB = this.subjectB();
    if (!subjectA || !subjectB) return null;
    const aName = !!subjectA.name;
    const bName = !!subjectB.name;
    if (aName && !bName) return { target: subjectA, source: subjectB };
    if (bName && !aName) return { target: subjectB, source: subjectA };
    // Both named or both unnamed: lower id wins
    return subjectA.id <= subjectB.id
      ? { target: subjectA, source: subjectB }
      : { target: subjectB, source: subjectA };
  }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS (all tests, including the two new ones and the pre-existing suite).

- [ ] **Step 5: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): make mergeTarget redirect-aware via targetOverride/redirectSource"
```

---

### Task 2: `applyRedirect()` — the shared re-target method

Both the footer typeahead (Task 3) and the upgraded collision prompt (Task 5) need to do the
exact same thing on pick: set the override, reset picker UI, and reload the new target's
faces into whichever photo slot the original keep subject occupied. Write this once.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`.
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `targetOverride`, `redirectSource` (Task 1); `photoService.getSubjectPhotosWithFaces`
  (existing); `photosA`/`photosB`/`_loadGen` (existing, from `loadPhotos()`).
- Produces: `protected async applyRedirect(picked: Subject): Promise<void>` — callable from
  template event handlers in Task 3 and Task 5.

- [ ] **Step 1: Write the failing test**

```ts
  it('applyRedirect sets override/source and reloads faces into the original keep slot', async () => {
    const a = makeSubject(1, 'Alice');   // named -> original target/keep, column A
    const b = makeSubject(2, null);      // unnamed -> original source, column B
    const roberto = makeSubject(99, 'Roberto');

    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) => {
      if (id === 99) return [makePhoto(500)];
      return [];
    });

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await component.applyRedirect(roberto);

    expect(component.targetOverride()).toEqual(roberto);
    expect(component.redirectSource()).toEqual(b); // original non-target participant
    expect(photoService.getSubjectPhotosWithFaces).toHaveBeenCalledWith(99);
    // Column A held the original keep subject's (Alice's) faces; it now shows Roberto's.
    expect(component.photosA().map(f => f.face_id)).toEqual([500]);
    expect(component.showRedirectPicker()).toBe(false);
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `component.applyRedirect is not a function`.

- [ ] **Step 3: Implement `applyRedirect`**

Add a private helper to know which signal (`photosA`/`photosB`) corresponds to the original
keep subject, then the public method. Insert after `loadPhotos` (around line 154):

```ts
  /** Which photo signal (A or B) currently displays the given subject's faces, based on
   *  the *original* suggestion (not the redirect) — used to know where to load the picked
   *  subject's faces once a redirect happens. */
  private photosSignalFor(subjectId: number): typeof this.photosA {
    return this._suggestion?.subject_a.id === subjectId ? this.photosA : this.photosB;
  }

  /** Re-target the merge to `picked` instead of the original keep subject. Does not merge —
   *  the user must still click "Merge as {picked.name}" to confirm (see confirm()). */
  protected async applyRedirect(picked: Subject): Promise<void> {
    const originalTarget = this.mergeTarget; // pre-redirect target/source, before override is set
    if (!originalTarget) return;

    this.redirectSource.set(originalTarget.source);
    this.targetOverride.set(picked);
    this.showRedirectPicker.set(false);

    const gen = ++this._loadGen;
    const photosSig = this.photosSignalFor(originalTarget.target.id);
    try {
      const photos = await this.photoService.getSubjectPhotosWithFaces(picked.id);
      if (gen !== this._loadGen) return; // stale, discard
      photosSig.set(photos);
    } catch (e) {
      console.error('MergeReview: failed to load redirected subject faces', e);
    }
  }
```

Note: `originalTarget.source` must be read **before** `targetOverride` is set, since
`mergeTarget` becomes redirect-aware as soon as the override is non-null (Task 1) — reading
it after would recursively return the just-set override instead of the original source.

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): add applyRedirect() to re-target a merge without committing"
```

---

### Task 3: Footer typeahead — entry point, filtering, keyboard nav, pick

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`.
- Modify: `src/app/components/merge-review/merge-review.component.html:69-106` (the
  `.modal-actions` block).
- Modify: `src/app/components/merge-review/merge-review.component.css`.
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `showRedirectPicker`, `applyRedirect` (Tasks 1-2), `photoService.subjects()`
  (existing signal), `mergeTarget` (existing getter, now redirect-aware).
- Produces: `protected redirectQuery = signal('')`, `protected redirectCandidates =
  computed<Subject[]>`, `protected redirectHighlight = signal(0)`,
  `protected openRedirectPicker()`, `protected onRedirectKeydown(event: KeyboardEvent)`,
  `protected pickRedirectCandidate(subject: Subject)`.

- [ ] **Step 1: Write the failing tests**

```ts
  it('shows the "Merge into someone else…" link in the normal footer, hidden while submitting', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    let link = fixture.debugElement.query(By.css('[data-test="redirect-link"]'));
    expect(link).toBeTruthy();

    component.submitting.set(true);
    fixture.detectChanges();
    link = fixture.debugElement.query(By.css('[data-test="redirect-link"]'));
    expect(link).toBeFalsy();
  });

  it('redirectCandidates excludes the current source and unnamed subjects, filters by query', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null); // source
    photoService.subjects.set([
      a, b,
      makeSubject(3, null),        // unnamed -> excluded
      makeSubject(4, 'Roberto'),
      makeSubject(5, 'Robert Sr.'),
    ]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.openRedirectPicker();
    component.redirectQuery.set('rob');

    const names = component.redirectCandidates().map(s => s.name);
    expect(names).toEqual(['Roberto', 'Robert Sr.']);
  });

  it('redirectCandidates is empty (not thrown) when query matches nothing', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    photoService.subjects.set([a, b, makeSubject(4, 'Roberto')]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.openRedirectPicker();
    component.redirectQuery.set('zzz-no-match');

    expect(component.redirectCandidates()).toEqual([]);
  });

  it('Enter on the highlighted candidate calls applyRedirect', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(4, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    const applySpy = vi.spyOn(component, 'applyRedirect').mockResolvedValue(undefined);
    component.openRedirectPicker();
    component.redirectQuery.set('Roberto');
    component.redirectHighlight.set(0);

    component.onRedirectKeydown({ key: 'Enter', preventDefault: () => {}, stopPropagation: () => {} } as unknown as KeyboardEvent);

    expect(applySpy).toHaveBeenCalledWith(roberto);
  });

  it('Escape closes the picker without applying a redirect and does not propagate', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.openRedirectPicker();
    let propagated = false;
    component.onRedirectKeydown({
      key: 'Escape',
      preventDefault: () => {},
      stopPropagation: () => { propagated = true; },
    } as unknown as KeyboardEvent);

    expect(component.showRedirectPicker()).toBe(false);
    expect(component.targetOverride()).toBeNull();
    expect(propagated).toBe(true); // confirms stopPropagation was called, not that it reached document
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `component.openRedirectPicker is not a function` (and similarly for the
other new members).

- [ ] **Step 3: Implement the picker logic**

Add signals near the Task 1/2 additions:

```ts
  protected redirectQuery = signal('');
  protected redirectHighlight = signal(0);

  protected redirectCandidates = computed<Subject[]>(() => {
    const query = this.redirectQuery().trim().toLowerCase();
    const sourceId = this.mergeTarget?.source.id;
    return this.photoService.subjects().filter((s) => {
      if (!s.name) return false;
      if (s.id === sourceId) return false;
      if (this.targetOverride() && s.id === this.targetOverride()!.id) return false;
      if (!query) return true;
      return s.name.toLowerCase().includes(query);
    });
  });
```

Add the open/keydown/pick methods (after `applyRedirect`):

```ts
  protected openRedirectPicker(): void {
    this.redirectQuery.set('');
    this.redirectHighlight.set(0);
    this.showRedirectPicker.set(true);
  }

  protected onRedirectKeydown(event: KeyboardEvent): void {
    const candidates = this.redirectCandidates();
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      this.redirectHighlight.update((i) => Math.min(i + 1, Math.max(candidates.length - 1, 0)));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      this.redirectHighlight.update((i) => Math.max(i - 1, 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const picked = candidates[this.redirectHighlight()];
      if (picked) void this.applyRedirect(picked);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation(); // must not bubble to @HostListener('document:keydown.escape')
      this.showRedirectPicker.set(false);
    }
  }

  protected pickRedirectCandidate(subject: Subject): void {
    void this.applyRedirect(subject);
  }
```

- [ ] **Step 4: Add the template markup**

Replace the `.modal-actions` block in `merge-review.component.html` (lines 69-106) with a
three-state block:

```html
      <div class="modal-actions">
        @if (showExitConfirm()) {
          <div class="flex items-center gap-3 mr-auto text-sm">
            <span class="text-muted-foreground">Both named "{{ mergeTarget?.target?.name }}".</span>
            <button
              class="px-3 py-1.5 rounded-md border border-border hover:bg-muted transition-colors text-muted-foreground"
              (click)="keepSeparate()"
            >
              Keep separate
            </button>
            <button
              class="px-3 py-1.5 rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
              (click)="confirmFromGuard()"
            >
              Merge
            </button>
          </div>
        } @else if (showRedirectPicker()) {
          <div class="redirect-picker mr-auto" (keydown)="onRedirectKeydown($event)">
            <input
              type="text"
              class="redirect-input"
              placeholder="Type a name…"
              autofocus
              [value]="redirectQuery()"
              (input)="redirectQuery.set($any($event.target).value)"
              data-test="redirect-input"
            />
            <div class="redirect-results">
              @for (candidate of redirectCandidates(); track candidate.id; let i = $index) {
                <button
                  type="button"
                  class="redirect-result-row"
                  [class.highlighted]="i === redirectHighlight()"
                  (click)="pickRedirectCandidate(candidate)"
                >
                  <span class="font-medium">{{ candidate.name }}</span>
                </button>
              } @empty {
                <div class="redirect-empty text-sm text-muted-foreground">No matching subjects</div>
              }
            </div>
          </div>
        } @else {
          <button
            class="redirect-link text-sm text-muted-foreground mr-auto"
            data-test="redirect-link"
            (click)="openRedirectPicker()"
            [disabled]="submitting()"
          >
            Merge into someone else…
          </button>
          <button
            class="px-4 py-2 rounded-md border border-border hover:bg-muted transition-colors text-muted-foreground"
            cdkFocusInitial
            (click)="dismiss()"
            [disabled]="submitting()"
          >
            Not the same person
          </button>
          <button
            class="px-4 py-2 rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
            [class.ring-2]="namesIdentical()"
            [class.ring-primary]="namesIdentical()"
            [class.animate-pulse]="shouldPulse()"
            (click)="confirm()"
            [disabled]="submitting()"
          >
            Merge as <span [class.opacity-50]="!mergeTarget?.target?.name">{{ mergeTarget?.target?.name || 'Unnamed' }}</span>
          </button>
        }
      </div>
```

Note the `data-test="redirect-link"` hook the Step 1 test queries for — this repo's existing
specs query by `By.directive`/`By.css` on structural/class selectors (no prior `data-test`
convention in this file), but a stable hook is warranted here because the link's own class
list (`redirect-link text-sm text-muted-foreground mr-auto`) is purely presentational and
likely to be tuned; add the attribute now to keep the test resilient to styling changes.

- [ ] **Step 5: Add minimal CSS**

Append to `merge-review.component.css`:

```css
.redirect-link {
  background: none;
  border: none;
  cursor: pointer;
  text-decoration: none;
  padding: 0;
}
.redirect-link:hover:not(:disabled) {
  text-decoration: underline;
}
.redirect-link:disabled {
  opacity: 0.5;
  cursor: default;
}

.redirect-picker {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
  max-width: 320px;
}
.redirect-input {
  padding: 0.5rem 0.75rem;
  border: 1px solid hsl(var(--border));
  border-radius: 0.375rem;
  background: hsl(var(--background));
}
.redirect-results {
  max-height: 180px;
  overflow-y: auto;
  border: 1px solid hsl(var(--border));
  border-radius: 0.375rem;
}
.redirect-result-row {
  display: block;
  width: 100%;
  text-align: left;
  padding: 0.5rem 0.75rem;
  background: none;
  border: none;
  cursor: pointer;
}
.redirect-result-row.highlighted,
.redirect-result-row:hover {
  background: hsl(var(--muted));
}
.redirect-empty {
  padding: 0.5rem 0.75rem;
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS (all tests from Tasks 1-3).

- [ ] **Step 7: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.css src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): add footer typeahead to redirect a merge candidate"
```

---

### Task 4: Avatar for each typeahead row

Spec Part 1 requires an avatar per candidate row (name-only rows would make it hard to
recognize the right person — the whole point of the feature). Follows the existing
`getFaceCrop` + `thumbnailUrl` pattern from `subject-person-card.component.ts:44-55`.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`.
- Modify: `src/app/components/merge-review/merge-review.component.html`.
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `redirectCandidates` (Task 3), `photoService.getFaceCrop`,
  `photoService.thumbnailUrl` (existing).
- Produces: `protected redirectAvatarUrls = signal<Map<number, string | null>>(new Map())`,
  private `private loadRedirectAvatars(candidates: Subject[]): void` (fire-and-forget, called
  from an `effect()`).

- [ ] **Step 1: Write the failing test**

```ts
  it('loads an avatar crop for each redirect candidate', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = { ...makeSubject(4, 'Roberto'), thumbnail_face_id: 777 };
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('/crops/777.jpg');
    vi.spyOn(photoService, 'thumbnailUrl').mockImplementation((p) => p);

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.openRedirectPicker();
    fixture.detectChanges();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(photoService.getFaceCrop).toHaveBeenCalledWith(777);
    expect(component.redirectAvatarUrls().get(4)).toBe('/crops/777.jpg');
  });

  it('leaves the avatar entry null when a candidate has no thumbnail_face_id', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const noThumb = makeSubject(4, 'Roberto'); // thumbnail_face_id: null via makeSubject
    photoService.subjects.set([a, b, noThumb]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.openRedirectPicker();
    fixture.detectChanges();
    await new Promise(resolve => setTimeout(resolve, 0));

    expect(component.redirectAvatarUrls().get(4) ?? null).toBeNull();
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `component.redirectAvatarUrls is not a function`.

- [ ] **Step 3: Implement avatar loading**

Add the signal and loader. This uses an `effect()` in the constructor area — but this
component has no constructor today (all wiring is field initializers/`inject()`), so add a
constructor:

```ts
  protected redirectAvatarUrls = signal<Map<number, string | null>>(new Map());

  constructor() {
    effect(() => {
      if (this.showRedirectPicker()) {
        this.loadRedirectAvatars(this.redirectCandidates());
      }
    });
  }

  private loadRedirectAvatars(candidates: Subject[]): void {
    const known = this.redirectAvatarUrls();
    const missing = candidates.filter((c) => !known.has(c.id));
    for (const candidate of missing) {
      if (!candidate.thumbnail_face_id) {
        this.redirectAvatarUrls.update((m) => new Map(m).set(candidate.id, null));
        continue;
      }
      this.photoService.getFaceCrop(candidate.thumbnail_face_id)
        .then((path) => {
          this.redirectAvatarUrls.update((m) => new Map(m).set(candidate.id, this.photoService.thumbnailUrl(path)));
        })
        .catch(() => {
          this.redirectAvatarUrls.update((m) => new Map(m).set(candidate.id, null));
        });
    }
  }
```

Note: `effect()` re-runs whenever `redirectCandidates()` changes (e.g. as the user types),
so `loadRedirectAvatars` must be idempotent for already-known ids — the `known.has(c.id)`
guard above ensures a candidate's crop is fetched at most once per modal lifetime, not once
per keystroke.

- [ ] **Step 4: Render the avatar in the template**

Update the candidate row in `merge-review.component.html` (from Task 3):

```html
                <button
                  type="button"
                  class="redirect-result-row"
                  [class.highlighted]="i === redirectHighlight()"
                  (click)="pickRedirectCandidate(candidate)"
                >
                  @if (redirectAvatarUrls().get(candidate.id); as avatarUrl) {
                    <img [src]="avatarUrl" class="redirect-avatar" alt="" />
                  } @else {
                    <span class="redirect-avatar redirect-avatar-placeholder"></span>
                  }
                  <span class="font-medium">{{ candidate.name }}</span>
                </button>
```

Update `.redirect-result-row` in the CSS to a flex row and add avatar sizing:

```css
.redirect-result-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  width: 100%;
  text-align: left;
  padding: 0.5rem 0.75rem;
  background: none;
  border: none;
  cursor: pointer;
}
.redirect-avatar {
  width: 1.75rem;
  height: 1.75rem;
  border-radius: 9999px;
  object-fit: cover;
  flex-shrink: 0;
}
.redirect-avatar-placeholder {
  background: hsl(var(--muted));
}
```

(Remove the earlier plain `.redirect-result-row` rule from Task 3's CSS step — this one
supersedes it.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.css src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): show face-crop avatars in the redirect typeahead"
```

---

### Task 5: Upgrade the name-collision error into a second redirect entry point

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts:65-66` (the
  `nameErrorA`/`nameErrorB` signal types), `:100-132` (`onNameCommit`).
- Modify: `src/app/components/merge-review/merge-review.component.html:23-25,52-54` (the
  error paragraphs).
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `applyRedirect` (Task 2).
- Produces: `nameErrorA`/`nameErrorB` change type from `signal<string | null>` to
  `signal<{ message: string; conflict: Subject } | null>`.

- [ ] **Step 1: Write the failing test**

```ts
  it('onNameCommit collision populates a structured error with the conflicting subject', async () => {
    const a = makeSubject(1, null);
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await component.onNameCommit('a', 'Roberto');

    expect(component.nameErrorA()).toEqual({
      message: 'A subject named "Roberto" already exists.',
      conflict: roberto,
    });
  });

  it('clicking "Merge into {name}" on the collision error applies the redirect and never calls nameSubject', async () => {
    const a = makeSubject(1, null);
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const nameSubjectSpy = vi.spyOn(photoService, 'nameSubject');
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await component.onNameCommit('a', 'Roberto');
    const applySpy = vi.spyOn(component, 'applyRedirect').mockResolvedValue(undefined);

    const conflict = component.nameErrorA()!.conflict;
    await component.applyRedirect(conflict);

    expect(applySpy).toHaveBeenCalledWith(roberto);
    expect(nameSubjectSpy).not.toHaveBeenCalled();
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `nameErrorA()` returns a string, not `{ message, conflict }`
(assertion mismatch).

- [ ] **Step 3: Update the error signal type and `onNameCommit`**

Change lines 65-66:

```ts
  protected nameErrorA = signal<{ message: string; conflict: Subject } | null>(null);
  protected nameErrorB = signal<{ message: string; conflict: Subject } | null>(null);
```

Update the Case-3 block inside `onNameCommit` (lines 111-124):

```ts
    // Case 3: matches a DIFFERENT existing subject (not either column in this modal) →
    // offer a redirect instead of a dead-end.
    if (typed) {
      const other = otherSig();
      const conflict = this.photoService.subjects().find(
        (s) =>
          s.id !== subject.id &&
          s.id !== other?.id &&
          (s.name ?? '').toLowerCase() === typed.toLowerCase(),
      );
      if (conflict) {
        errorSig.set({ message: `A subject named "${typed}" already exists.`, conflict });
        return; // no backend write; EditableText re-displays the unchanged signal value
      }
    }
```

(No change needed to the rest of `onNameCommit` — the empty-commit and same-value paths are
unaffected.)

- [ ] **Step 4: Update the template**

Replace both error paragraphs (lines 23-25 and 52-54) with, e.g. for column A:

```html
          @if (nameErrorA(); as err) {
            <p class="text-xs text-destructive mb-1">
              {{ err.message }}
              <button type="button" class="underline" (click)="applyRedirect(err.conflict)">
                Merge into {{ err.conflict.name }}
              </button>
            </p>
          }
```

(Mirror for `nameErrorB()`.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): upgrade name-collision error into a redirect entry point"
```

---

### Task 6: Deleted-target guard in `confirm()`

Best-effort mitigation for the spec's "picked subject deleted mid-flow" edge case: before
calling `mergeSubjects` on a redirected target, verify the target id is still present in
`photoService.subjects()`.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts:156-169` (`confirm()`).
- Modify: `src/app/components/merge-review/merge-review.component.html` (a new inline error
  slot in the normal footer state, next to the "Merge as X" button).
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `targetOverride`, `showRedirectPicker`, `photoService.subjects()`.
- Produces: `protected redirectGoneError = signal<string | null>(null)`.

- [ ] **Step 1: Write the failing test**

```ts
  it('confirm shows an error and reopens the picker if the redirected target no longer exists', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const mergeSpy = vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.targetOverride.set(roberto);
    component.redirectSource.set(b);
    // Roberto is no longer in the live subjects list (deleted elsewhere mid-flow).
    photoService.subjects.set([a, b]);

    await component.confirm();

    expect(mergeSpy).not.toHaveBeenCalled();
    expect(component.redirectGoneError()).toBe('Roberto is no longer available — pick another subject.');
    expect(component.showRedirectPicker()).toBe(true);
  });

  it('confirm proceeds normally for a redirected target that still exists', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const mergeSpy = vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    photoService.subjects.set([a, b, roberto]);
    component.targetOverride.set(roberto);
    component.redirectSource.set(b);

    await component.confirm();

    expect(mergeSpy).toHaveBeenCalledWith(9, 2);
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `component.redirectGoneError is not a function`.

- [ ] **Step 3: Implement the guard**

Add the signal near `targetOverride`:

```ts
  protected redirectGoneError = signal<string | null>(null);
```

Update `confirm()` (lines 156-169):

```ts
  async confirm() {
    const target = this.mergeTarget;
    if (!target || this.submitting()) return;

    const override = this.targetOverride();
    if (override && !this.photoService.subjects().some((s) => s.id === override.id)) {
      this.redirectGoneError.set(`${override.name} is no longer available — pick another subject.`);
      this.showRedirectPicker.set(true);
      return;
    }

    this.submitting.set(true);
    try {
      await this.runMergeAnimation(target);
      await this.photoService.mergeSubjects(target.target.id, target.source.id);
      this.confirmed.emit(target.target.id);
    } catch (e) {
      console.error('MergeReview: merge failed', e);
    } finally {
      this.submitting.set(false);
    }
  }
```

- [ ] **Step 4: Surface the error in the template**

In the normal footer branch of `merge-review.component.html` (from Task 3), add just above
the "Merge as X" button:

```html
          @if (redirectGoneError()) {
            <span class="text-xs text-destructive mr-2">{{ redirectGoneError() }}</span>
          }
```

Clear it when the picker reopens — update `openRedirectPicker()` (Task 3):

```ts
  protected openRedirectPicker(): void {
    this.redirectQuery.set('');
    this.redirectHighlight.set(0);
    this.redirectGoneError.set(null);
    this.showRedirectPicker.set(true);
  }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): guard confirm() against a redirected target deleted mid-flow"
```

---

### Task 7: Guardrail test — redirect path never writes a `cannot_link`

This is the explicit test the design spec calls out as its own line item (Part 3): the whole
redirect journey (open picker → pick → confirm) must never call `dismissMergeSuggestion`.
Written as an end-to-end-within-the-component test spanning all prior tasks.

**Files:**
- Test only: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `openRedirectPicker`, `applyRedirect`, `confirm` (Tasks 2, 3, 6).

- [ ] **Step 1: Write the test**

```ts
  it('a full redirect-merge journey never calls dismissMergeSuggestion', async () => {
    const a = makeSubject(1, 'Alice'); // original keep, bypassed
    const b = makeSubject(2, null);    // original candidate, actually merged
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion');

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.openRedirectPicker();
    await component.applyRedirect(roberto);
    await component.confirm();

    expect(dismissSpy).not.toHaveBeenCalled();
    expect(photoService.mergeSubjects).toHaveBeenCalledWith(9, 2); // Roberto absorbs B; A untouched
  });
```

- [ ] **Step 2: Run the test**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS immediately (Tasks 1-6 already implement everything this test exercises — this
step is a regression guardrail, not new functionality). If it fails, that indicates a
regression in an earlier task; fix there rather than adding new logic here.

- [ ] **Step 3: Run the full test suite once**

Run: `npx vitest run src/app/components/merge-review/`
Expected: PASS — every test in `merge-review.component.spec.ts` (pre-existing plus all tests
added in Tasks 1-7).

- [ ] **Step 4: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "test(merge-review): guardrail — redirect-merge journey never dismisses/cannot-links"
```

---

## Self-Review Notes (for whoever executes this plan)

- **Spec coverage:** Part 1 (typeahead + re-target/confirm) → Tasks 1-4, 6. Part 2 (collision
  upgrade) → Task 5. Part 3 (no auto-cannot-link) → Task 7 (test-only, since Background in the
  spec establishes no code change is needed — see Global Constraints).
- **Type consistency check performed:** `applyRedirect(picked: Subject)` (Task 2) is the same
  signature called from Task 3's `pickRedirectCandidate`/`onRedirectKeydown` and Task 5's
  template button — verified no divergent name (e.g. no `redirectTo`/`retarget` alias
  introduced elsewhere).
- **Escape key isolation** (Task 3, Step 3) is the one genuinely easy-to-miss detail flagged
  in the spec — `event.stopPropagation()` inside `onRedirectKeydown` before the existing
  `@HostListener('document:keydown.escape') onEscape()` fires. Verify this by hand (or via
  Task 3's Escape test) whenever touching this area again.
