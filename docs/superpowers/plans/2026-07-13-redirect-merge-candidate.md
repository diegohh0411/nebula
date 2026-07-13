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

### Task 1: `mergeTarget` becomes redirect-aware (`targetOverride` / `redirectSource`) + reset on new suggestion

Lay the state/logic foundation before any UI exists. `mergeTarget` must special-case a
pending redirect **before** falling back to the existing named/id tiebreak.

**Blocking correctness note (added after adversarial review):** `MergeReviewComponent` is a
single long-lived instance reused across suggestions (`people-view.component.html:89-97`,
`[suggestion]="reviewingSuggestion()"` — the component is never destroyed/recreated between
reviews). The `suggestion` input setter resets `subjectA`/`subjectB`/photos today but nothing
else. Without an explicit reset, confirming one redirect-merge and then advancing to the next
suggestion would leave `targetOverride`/`redirectSource` set from the *previous* review —
`mergeTarget` checks `targetOverride()` first, unconditionally, so the brand-new suggestion
would silently redirect to the stale target. This task's Step 3 therefore also updates the
`suggestion` setter, not only the `mergeTarget` getter.

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
- Produces: the `suggestion` input setter additionally resets `targetOverride`,
  `redirectSource`, `showRedirectPicker`, `redirectQuery`, `redirectGoneError`, `nameErrorA`,
  `nameErrorB` to their initial values on every assignment (this task only adds the reset
  calls for the two signals it introduces; later tasks that introduce the remaining signals
  must add their own reset call at the same site — flagged again in Tasks 3/5/6 below so it
  isn't missed).

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

  it('assigning a new suggestion resets an active redirect from the previous suggestion', () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    component.suggestion = makeSuggestion(a, b);

    const roberto = makeSubject(99, 'Roberto');
    component.targetOverride.set(roberto);
    component.redirectSource.set(b);

    const c = makeSubject(3, 'Cara');
    const d = makeSubject(4, null);
    component.suggestion = makeSuggestion(c, d); // simulates advancing to the next review

    expect(component.targetOverride()).toBeNull();
    expect(component.redirectSource()).toBeNull();
    expect(component.mergeTarget).toEqual({ target: c, source: d }); // not the stale Roberto
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `Property 'targetOverride' does not exist on type 'MergeReviewComponent'`
surfaced as a runtime `TypeError` (this repo's Vitest config runs through esbuild, which
transpiles but does not type-check, so a missing-member error only becomes visible at
runtime, not as a `tsc` compile error — the plan's later "expected failure" notes for new
members follow the same pattern and should be read the same way).

- [ ] **Step 3: Implement the signals and update `mergeTarget`, and reset state in the `suggestion` setter**

In `merge-review.component.ts`, add the two new signals next to the existing state signals
(after `showExitConfirm`, around line 67):

```ts
  protected showExitConfirm = signal(false);
  protected showRedirectPicker = signal(false);
  protected targetOverride = signal<Subject | null>(null);
  protected redirectSource = signal<Subject | null>(null);
```

Update the `suggestion` input setter (lines 39-45) to reset the new signals on every
assignment — this must happen even when a redirect was left active from a *previous*
suggestion, since this component instance is reused across reviews (`people-view` never
destroys/recreates it):

```ts
  @Input()
  set suggestion(value: MergeSuggestion | null) {
    this._suggestion = value;
    this.subjectA.set(value?.subject_a ?? null);
    this.subjectB.set(value?.subject_b ?? null);
    this.targetOverride.set(null);
    this.redirectSource.set(null);
    this.showRedirectPicker.set(false);
    // Tasks 3/5/6 add further resets here (redirectQuery, redirectGoneError, nameErrorA/B) —
    // do not lose this reset block when those tasks touch this setter.
    void this.loadPhotos(value);
  }
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

Both the footer typeahead (Task 3) and the upgraded collision prompt (Task 6) need to do the
exact same thing on pick: set the override, reset picker UI, and reload the new target's
faces into whichever photo slot the original keep subject occupied. Write this once.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`.
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `targetOverride`, `redirectSource` (Task 1); `photoService.getSubjectPhotosWithFaces`
  (existing); `photosA`/`photosB`/`_loadGen` (existing, from `loadPhotos()`).
- Produces: `protected async applyRedirect(picked: Subject, explicitSource?: Subject): Promise<void>`
  — callable from template event handlers in Task 3 (calls with one arg — the picker always
  redirects the current `mergeTarget.source`) and Task 6 (calls with two args — a name
  collision can happen while renaming *either* column, including the current
  `mergeTarget.target`, so Task 6's caller must pass the actually-renamed subject explicitly
  rather than rely on `applyRedirect` re-deriving it from `mergeTarget.source`, which would be
  wrong whenever the collision happened on the target/keep column).
- Produces: `protected redirectColumn = signal<'a' | 'b' | null>(null)` — captures, once, which
  of `subjectA`/`subjectB` is the slot whose faces get replaced by a redirect, so a *second*
  pick (re-opening the picker after already redirecting once) reloads into the same slot
  instead of re-deriving it from `mergeTarget.target.id`, which after the first redirect is
  already `C`'s id and matches neither `subjectA.id` nor `subjectB.id`.

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

  it('applyRedirect uses the explicit source when provided, not mergeTarget.source', async () => {
    const a = makeSubject(1, 'Alice');   // named -> original target/keep, column A
    const b = makeSubject(2, null);      // unnamed -> original source, column B
    const roberto = makeSubject(99, 'Roberto');

    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    // Simulate a Part-2-style collision caught while renaming the *target* column (Alice),
    // not the source column — mergeTarget.source is still `b`, but the actual redirect must
    // treat `a` as the source, since Alice is the one being redirected into Roberto.
    await component.applyRedirect(roberto, a);

    expect(component.redirectSource()).toEqual(a);
  });

  it('a second applyRedirect call reloads into the same slot as the first, not a re-derived one', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(99, 'Roberto');
    const carla = makeSubject(100, 'Carla');

    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockImplementation(async (id: number) => {
      if (id === 99) return [makePhoto(500)];
      if (id === 100) return [makePhoto(600)];
      return [];
    });

    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await component.applyRedirect(roberto);   // first pick -> column A (Alice's original slot)
    await component.applyRedirect(carla);     // re-pick -> must still land in column A, not B

    expect(component.photosA().map(f => f.face_id)).toEqual([600]); // Carla's faces
    expect(component.photosB().map(f => f.face_id)).toEqual([]);    // B (the real candidate) untouched
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `component.applyRedirect is not a function` (a runtime error, not a `tsc`
compile error — see Task 1's note on this repo's esbuild-based Vitest transform).

- [ ] **Step 3: Implement `applyRedirect`**

Add a private helper to know which signal (`photosA`/`photosB`) corresponds to the original
keep subject, then the public method. Insert after `loadPhotos` (around line 154):

```ts
  /** Which photo signal (A or B) is the redirect slot. Decided once, on the FIRST redirect
   *  in this modal session, from the *original* (pre-any-redirect) suggestion, and reused
   *  unchanged by every subsequent pick — recomputing this from the live `mergeTarget` on a
   *  second pick would be wrong, because after the first redirect `mergeTarget.target.id` is
   *  already the previously-picked subject's id, which matches neither `subjectA.id` nor
   *  `subjectB.id`. */
  private photosSignalFor(originalTargetId: number): typeof this.photosA {
    const column = this.redirectColumn() ?? (this._suggestion?.subject_a.id === originalTargetId ? 'a' : 'b');
    if (this.redirectColumn() === null) this.redirectColumn.set(column);
    return column === 'a' ? this.photosA : this.photosB;
  }

  /** Re-target the merge to `picked` instead of the original keep subject (or, when
   *  `explicitSource` is given, instead of whichever subject is explicitly passed — used by
   *  the Part 2 collision entry point, where the colliding rename may have happened on
   *  either column, not necessarily the current `mergeTarget.source`). Does not merge — the
   *  user must still click "Merge as {picked.name}" to confirm (see confirm()). */
  protected async applyRedirect(picked: Subject, explicitSource?: Subject): Promise<void> {
    const originalTarget = this.mergeTarget; // pre-redirect target/source, before override is set
    if (!originalTarget && !explicitSource) return;

    const source = explicitSource ?? originalTarget!.source;
    // The slot to reload is always keyed off the ORIGINAL target id on the first redirect —
    // even when called with an explicitSource, the slot being replaced is the one that
    // currently shows the "keep" subject's faces, i.e. mergeTarget.target, not `source`.
    const slotAnchorId = originalTarget?.target.id ?? this._suggestion!.subject_a.id;

    this.redirectSource.set(source);
    this.targetOverride.set(picked);
    this.showRedirectPicker.set(false);
    this.nameErrorA.set(null);
    this.nameErrorB.set(null);

    const gen = ++this._loadGen;
    const photosSig = this.photosSignalFor(slotAnchorId);
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

Also update `runMergeAnimation` (existing method, `merge-review.component.ts:226-251`) to
skip the animation entirely when a redirect is active — its column-mapping
(`target.target.id === this._suggestion!.subject_a.id`) assumes `target.target` is always one
of the two original suggestion participants, which is false once `targetOverride` is set (C
is never `subject_a`), so it would animate the wrong DOM column:

```ts
  protected async runMergeAnimation(target: MergeTarget) {
    if (prefersReducedMotion() || this.targetOverride()) {
      return; // accepted v1 simplification: no animation for a redirected confirm (see spec)
    }
    ...
```

Also update the `suggestion` setter added in Task 1 to reset `redirectColumn` alongside the
other redirect signals (`this.redirectColumn.set(null);`).

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
- Reset reminder: this task's `suggestion`-setter reset (started in Task 1) must also reset
  `redirectQuery` and `redirectHighlight` to their initial values.
- **Resolved contradiction (from adversarial review):** the design spec says the "Merge into
  someone else…" link is *hidden* while `submitting()`; the naive `[disabled]="submitting()"`
  implementation below only disables it, leaving it present in the DOM, which would fail a
  test asserting the link is absent. This plan follows the spec: the link is removed from the
  DOM (via `@if (!submitting())`), matching the entry-point test below. If a future revision
  prefers "visible but disabled" instead, the spec and this test must be updated together —
  do not implement one and test the other.

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

  it('a real Escape keydown on the rendered redirect input does not close the whole modal', async () => {
    // The synthetic-event test above only proves onRedirectKeydown calls stopPropagation on
    // whatever object it's handed; it does not prove the template actually wires
    // (keydown)="onRedirectKeydown($event)" on the real input, nor that a bubbling native
    // Escape event is actually intercepted before reaching @HostListener('document:keydown.escape').
    // This test drives the real DOM to close that gap.
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const closedSpy = vi.fn();
    component.closed.subscribe(closedSpy);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    component.openRedirectPicker();
    fixture.detectChanges();

    const input = fixture.debugElement.query(By.css('.redirect-input')).nativeElement as HTMLInputElement;
    const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    input.dispatchEvent(event);
    fixture.detectChanges();

    expect(component.showRedirectPicker()).toBe(false);
    expect(closedSpy).not.toHaveBeenCalled(); // the modal itself must still be open
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — `component.openRedirectPicker is not a function` at runtime (and similarly
for the other new members; see Task 1's note on this repo's esbuild-based Vitest transform
not type-checking).

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

**Autofocus note — this supersedes the `openRedirectPicker` body just above, it is not an
alternative to run instead of it.** The plain `autofocus` HTML attribute does not reliably
focus an element that first appears via `@else if (showRedirectPicker())` after initial
render (most browsers only honor `autofocus` on the element present at parse time). Use the
`#redirectInput` template reference above with
`@ViewChild('redirectInput') redirectInputRef?: ElementRef<HTMLInputElement>;` and focus it
explicitly once the picker opens, e.g. inside `openRedirectPicker()` after a
microtask/`afterNextRender` tick so the `@else if` branch has rendered — the final body of
`openRedirectPicker()` is the version below, not the one above:

```ts
  protected openRedirectPicker(): void {
    this.redirectQuery.set('');
    this.redirectHighlight.set(0);
    this.showRedirectPicker.set(true);
    queueMicrotask(() => this.redirectInputRef?.nativeElement.focus());
  }
```

(A `queueMicrotask` is sufficient here since Angular's change detection for the `@else if`
branch runs synchronously within the same call stack as `showRedirectPicker.set(true)` under
`OnPush` change detection triggered by signal writes; if this proves unreliable in practice,
fall back to `afterNextRender` injected in the constructor instead.)

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
              #redirectInput
              type="text"
              class="redirect-input"
              placeholder="Type a name…"
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
          @if (!submitting()) {
            <button
              class="redirect-link text-sm text-muted-foreground mr-auto"
              data-test="redirect-link"
              (click)="openRedirectPicker()"
            >
              Merge into someone else…
            </button>
          }
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

### Task 5: Column display during an active redirect (name, keep badge, score chip)

**Added after adversarial review — this task is not optional cleanup.** Once a redirect is
active (`targetOverride` set), the redirected slot's `<app-editable-text>` still binds to the
*original* bypassed subject's name (`subjectA()?.name` / `subjectB()?.name`), and the `keep`
badge check (`mergeTarget?.target?.id === subjectA()?.id`) compares against `subjectA`/
`subjectB` directly — but `mergeTarget.target` is now `targetOverride()` (`C`), whose id
never equals either original subject's id. Without this task: the badge silently vanishes
from both columns once a redirect is active, and the name field for the redirected slot keeps
showing/renaming the bypassed original subject instead of `C` — a `commit` event there would
call `nameSubject` on the wrong subject entirely. The spec's Part 1 "the `keep` badge moves to
C's slot, the match-% chip ... is hidden or relabeled" has no other task implementing it.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.html:9-66` (both
  `subject-col` blocks), `:6` (the header match-% chip).
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`.

**Interfaces:**
- Consumes: `targetOverride`, `redirectColumn` (Task 2), `mergeTarget` (redirect-aware,
  Task 1).

- [ ] **Step 1: Write the failing tests**

```ts
  it('the redirected column shows the picked subject\'s name and keep badge, not the original subject\'s', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await component.applyRedirect(roberto); // redirects column A (Alice's original slot)
    fixture.detectChanges();

    const colA = fixture.debugElement.query(By.css('.subject-col'));
    expect(colA.nativeElement.textContent).toContain('Roberto');
    expect(colA.nativeElement.textContent).not.toContain('Alice');
    expect(colA.query(By.css('.keep-badge'))).toBeTruthy();
  });

  it('the header match-% chip is hidden or relabeled once a redirect is active', async () => {
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    await component.applyRedirect(roberto);
    fixture.detectChanges();

    const chip = fixture.debugElement.query(By.css('[data-test="match-score-chip"]'));
    expect(chip.nativeElement.textContent).not.toContain('%');
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: FAIL — the redirected column still shows "Alice", not "Roberto"; the keep badge is
absent from both columns; the chip still renders the original percentage.

- [ ] **Step 3: Update the template**

Add a small helper on the component to avoid repeating the "is this column the redirect
column" check three times in the template:

```ts
  /** Display name for a column, accounting for an active redirect into this slot. */
  protected columnDisplayName(which: 'a' | 'b'): string | null {
    if (this.targetOverride() && this.redirectColumn() === which) {
      return this.targetOverride()!.name;
    }
    return which === 'a' ? this.subjectA()?.name ?? null : this.subjectB()?.name ?? null;
  }

  /** Whether the `keep` badge belongs on this column, accounting for an active redirect. */
  protected columnIsKeep(which: 'a' | 'b'): boolean {
    if (this.targetOverride()) return this.redirectColumn() === which;
    const id = which === 'a' ? this.subjectA()?.id : this.subjectB()?.id;
    return this.mergeTarget?.target?.id === id;
  }
```

Update `merge-review.component.html`'s column A block (mirror for column B) — replace the
`[value]` binding and the `keep-badge` `@if`:

```html
            <app-editable-text
              [value]="columnDisplayName('a')"
              placeholder="Unnamed"
              placeholderClass="opacity-50"
              displayClass="font-semibold"
              (commit)="onNameCommit('a', $event)"
            />
            @if (columnIsKeep('a')) {
              <span class="keep-badge">keep</span>
            }
```

Update the header chip (line 6) to hide/relabel once a redirect is active:

```html
        <span class="text-sm text-muted-foreground" data-test="match-score-chip">
          @if (targetOverride()) {
            Manual reassignment
          } @else {
            {{ suggestion.score | percent }} match
          }
        </span>
```

**Not changed by this task, on purpose:** `(commit)="onNameCommit('a', $event)"` still calls
`onNameCommit`, which resolves the subject from `subjectA()`/`subjectB()`, not from
`targetOverride()`. Renaming the redirected slot while a redirect is active therefore still
renames the *original* bypassed subject under the hood, even though the displayed name is
now `C`'s. Wiring rename to actually target `C` when a redirect is active is deferred — see
Out of scope in the design spec ("editing the target's name mid-redirect"); flagged here so
it isn't silently assumed to already work by whoever implements this task.

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): update column name/keep-badge/score-chip display during an active redirect"
```

---

### Task 6: Upgrade the name-collision error into a second redirect entry point

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

  it('clicking "Merge into {name}" on the rendered collision error applies the redirect and never calls nameSubject', async () => {
    // IMPORTANT (fixed after adversarial review — the original version of this test called
    // component.applyRedirect(conflict) directly, i.e. the test performed the action it was
    // supposed to be verifying the UI triggers. That passes even if Step 4's template button
    // is never wired to anything (or doesn't exist at all), so it proves nothing about Part
    // 2's actual entry point. This version drives the real template: it renders the error,
    // finds the button, and clicks it.
    const a = makeSubject(1, null);
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([a, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const nameSubjectSpy = vi.spyOn(photoService, 'nameSubject');
    const applySpy = vi.spyOn(component, 'applyRedirect').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));
    fixture.detectChanges();

    await component.onNameCommit('a', 'Roberto');
    fixture.detectChanges();

    const button = fixture.debugElement.query(By.css('[data-test="collision-redirect-a"]'));
    expect(button).toBeTruthy();
    button.triggerEventHandler('click', null);

    expect(applySpy).toHaveBeenCalledWith(roberto, a); // explicit source: `a` is the renamed subject
    expect(nameSubjectSpy).not.toHaveBeenCalled();
  });

  it('a collision while renaming the currently-kept (target) column redirects that column, not mergeTarget.source', async () => {
    // Regression test for the bug the original spec/plan left ambiguous: Part 2's redirect
    // source must be whichever subject was actually being renamed, not mergeTarget.source —
    // those differ when the collision happens on the target/keep column rather than the
    // candidate column.
    const alice = makeSubject(1, 'Alice');  // named -> mergeTarget.target (the "keep" subject)
    const b = makeSubject(2, null);         // unnamed -> mergeTarget.source
    const roberto = makeSubject(9, 'Roberto');
    photoService.subjects.set([alice, b, roberto]);
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const mergeSpy = vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(alice, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    // User tries to rename Alice (the target/keep column) to "Roberto" -> collides.
    await component.onNameCommit('a', 'Roberto');
    const conflict = component.nameErrorA()!.conflict;
    await component.applyRedirect(conflict, alice); // Part 2 must pass `alice`, not mergeTarget.source (`b`)
    await component.confirm();

    // Alice (the one actually renamed) is merged into Roberto; `b` is left completely alone.
    expect(mergeSpy).toHaveBeenCalledWith(9, 1);
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
              <button
                type="button"
                class="underline"
                data-test="collision-redirect-a"
                (click)="applyRedirect(err.conflict, subjectA()!)"
              >
                Merge into {{ err.conflict.name }}
              </button>
            </p>
          }
```

Note the second argument, `subjectA()!` — this is the subject actually being renamed in this
column, and it is what `applyRedirect`'s `explicitSource` parameter (Task 2) must receive. Do
**not** omit it (which would fall back to `mergeTarget.source`): a collision while renaming
the *target*/keep column would then silently redirect the wrong subject (see Task 6's second
test above).

(Mirror for `nameErrorB()` — `data-test="collision-redirect-b"`, second arg `subjectB()!`.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `npx vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): upgrade name-collision error into a redirect entry point"
```

---

### Task 7: Deleted-target guard in `confirm()`

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
- Reset reminder: the `suggestion`-setter reset (Task 1) must also reset `redirectGoneError`.
- **Accepted trade-off:** this guard only re-checks `targetOverride` (the picked subject C);
  it does not re-check `redirectSource` (the original candidate B) for the same kind of
  concurrent deletion. See the design spec's Out-of-scope section for the rationale (same
  latent "no backend existence check" gap, narrower window since B is on-screen mid-review).

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

  it('the "no longer available" error is actually visible in the DOM, in the reopened picker', async () => {
    // The two tests above only assert component signal state, which would pass even if the
    // error span were placed in a footer branch that's hidden whenever showRedirectPicker()
    // is true (the bug caught in review — confirm() reopens the *picker* branch, not the
    // normal one). This test asserts the rendered DOM instead.
    const a = makeSubject(1, 'Alice');
    const b = makeSubject(2, null);
    const roberto = makeSubject(9, 'Roberto');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'mergeSubjects').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(a, b);
    await new Promise(resolve => setTimeout(resolve, 0));

    component.targetOverride.set(roberto);
    component.redirectSource.set(b);
    photoService.subjects.set([a, b]); // Roberto no longer present

    await component.confirm();
    fixture.detectChanges();

    const errorEl = fixture.debugElement.query(By.css('[data-test="redirect-gone-error"]'));
    expect(errorEl).toBeTruthy();
    expect(errorEl.nativeElement.textContent).toContain('no longer available');
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

**Placement bug caught in adversarial review:** `confirm()` (Step 3) sets
`showRedirectPicker.set(true)` when the guard trips — which switches `.modal-actions` to the
*picker* branch (`@else if (showRedirectPicker())` from Task 3), not the normal branch. A
`redirectGoneError` span placed in the normal footer branch would therefore never be visible
at the moment it's actually set; the user would see the picker reopen with no explanation,
and the error would only become visible later, stale, if the picker closed and the button
render path was reached again without the signal being cleared. The error must render inside
the **picker** branch instead, e.g. just above the `.redirect-input`:

```html
        } @else if (showRedirectPicker()) {
          <div class="redirect-picker mr-auto" (keydown)="onRedirectKeydown($event)">
            @if (redirectGoneError()) {
              <span class="text-xs text-destructive" data-test="redirect-gone-error">{{ redirectGoneError() }}</span>
            }
            <input
              ...
```

Clear it when the picker reopens — update `openRedirectPicker()` (Task 3, already carrying
the `queueMicrotask` focus call from Task 3's autofocus note; add the error reset alongside
it, don't drop the focus call):

```ts
  protected openRedirectPicker(): void {
    this.redirectQuery.set('');
    this.redirectHighlight.set(0);
    this.redirectGoneError.set(null);
    this.showRedirectPicker.set(true);
    queueMicrotask(() => this.redirectInputRef?.nativeElement.focus());
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

### Task 8: Guardrail test — redirect path never writes a `cannot_link`

This is the explicit test the design spec calls out as its own line item (Part 3): the whole
redirect journey (open picker → pick → confirm) must never call `dismissMergeSuggestion`.
Written as an end-to-end-within-the-component test spanning all prior tasks.

**Accepted trade-off (scope of this guardrail):** `photoService` in this spec file is the
real `PhotoService` obtained via `TestBed.inject(PhotoService)` (with `@tauri-apps/api/core`'s
`invoke` module-mocked), so `vi.spyOn(photoService, 'dismissMergeSuggestion')` attaches to a
real method and would catch a call added anywhere inside `MergeReviewComponent`. It does
**not** cover a hypothetical future "cleanup" call added to a *parent* container's handler
(e.g. `people-view`'s `(confirmed)` output handler) — no such call exists today and none is
being added by this plan, so there is nothing this guardrail is failing to catch right now,
but it would not catch one introduced later outside this component. Extending the guardrail
to `people-view` is out of scope (outside this plan's File Map).

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

- **Spec coverage:** Part 1 (typeahead + re-target/confirm + column display) → Tasks 1-5, 7.
  Part 2 (collision upgrade) → Task 6. Part 3 (no auto-cannot-link) → Task 8 (test-only, since
  Background in the spec establishes no code change is needed — see Global Constraints).
- **Type consistency check performed:** `applyRedirect(picked: Subject, explicitSource?: Subject)`
  (Task 2) is the same signature called from Task 3's `pickRedirectCandidate`/`onRedirectKeydown`
  (single-arg form, defaulting to `mergeTarget.source`) and Task 6's template button (two-arg
  form, passing the actually-renamed subject explicitly — see Task 2's note on why these two
  callers cannot share the single-arg form) — verified no divergent name (e.g. no
  `redirectTo`/`retarget` alias introduced elsewhere).
- **Escape key isolation** (Task 3, Step 3) is the one genuinely easy-to-miss detail flagged
  in the spec — `event.stopPropagation()` inside `onRedirectKeydown` before the existing
  `@HostListener('document:keydown.escape') onEscape()` fires. Verify this by hand (or via
  Task 3's Escape test) whenever touching this area again.
- **State-reset discipline** (Tasks 1/2/3/6/7): the `suggestion` setter accumulates reset
  calls for `targetOverride`/`redirectSource`/`showRedirectPicker`/`redirectColumn` (Task 1-2),
  `redirectQuery`/`redirectHighlight` (Task 3), `nameErrorA`/`nameErrorB` (already reset by
  `onNameCommit`, but also by Task 2's `applyRedirect` and must remain reset in the setter for
  the "advance to next suggestion" case), and `redirectGoneError` (Task 7). Whoever implements
  each task must add to the existing reset block, not replace it — verify the full list is
  still present after Task 7 by re-running Task 1's "assigning a new suggestion resets an
  active redirect" test at the end of the whole plan.
- **Wrong-subject-merged risk (Part 2):** the single most important thing to get right when
  implementing Task 6 is that the redirect source is the subject actually being renamed, not
  `mergeTarget.source` — see Task 2's `explicitSource` parameter and Task 6's second test.
  Getting this wrong silently merges the wrong subject with no error, so it will not surface
  as an obvious bug during manual testing unless the collision is deliberately triggered on
  the *target*/keep column rather than the candidate column.
