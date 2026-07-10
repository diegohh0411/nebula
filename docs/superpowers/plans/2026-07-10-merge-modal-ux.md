# Merge Modal UX Improvements — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route every subject-merge interaction through `MergeReviewComponent` — add inline name editing (with a duplicate-name block and an exit guard) and replace the old subject-detail similar-subjects merge flow with the modal.

**Architecture:** `MergeReviewComponent` gains a reactive local subject model (`subjectA`/`subjectB` signals) so names can be edited inline via the existing `EditableTextComponent`. A client-side validation ladder blocks naming a subject after a *third* existing subject. A duplicate-name exit guard prevents leaving two identically-named subjects unmerged. The subject-detail page drops its inline merge buttons and opens the same modal, using a new surviving-subject id emitted by the modal to decide reload-vs-navigate.

**Tech Stack:** Angular 20 (standalone components, signals, OnPush), Spartan-ng helm, Vitest, Tauri IPC via `PhotoService`.

## Global Constraints

- All name matching is **case-insensitive**, mirroring the backend (`find_subject_by_name` uses `COLLATE NOCASE`).
- No backend changes — all validation is client-side against existing `PhotoService` APIs.
- The name-conflict rename flow (`ConfirmMergeDialogComponent`) and `subject-person-card` are **out of scope** — do not touch them.
- Merging preserves all faces; only the surviving name/thumbnail/id differ.
- `dismissMergeSuggestion` writes a `cannot_link` mark — treat "dismiss" as definitive "not the same person" feedback.
- Run a single spec with: `pnpm exec vitest run <path-to-spec>`. Run the whole suite with: `pnpm test`.
- Working branch is `ux-improvements-on-the-merge-dialog` (already checked out) — no new branch needed.

---

### Task 1: Reactive local subject model in `MergeReviewComponent`

Convert the modal from a static `suggestion` field to local `subjectA`/`subjectB` signals so names can later be edited reactively. Behavior is unchanged; all existing tests must still pass.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`
- Modify: `src/app/components/merge-review/merge-review.component.html`
- Test: `src/app/components/merge-review/merge-review.component.spec.ts` (existing tests must stay green)

**Interfaces:**
- Produces: `subjectA: WritableSignal<Subject | null>`, `subjectB: WritableSignal<Subject | null>`, both seeded by the `suggestion` setter. `get mergeTarget()` derives from these signals and still returns `{ target: Subject; source: Subject } | null`.

- [ ] **Step 1: Run the existing spec to confirm a green baseline**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS (all existing tests green).

- [ ] **Step 2: Add the local subject signals and seed them in the setter**

In `merge-review.component.ts`, add two signals next to `photosA`/`photosB`:

```ts
  subjectA = signal<Subject | null>(null);
  subjectB = signal<Subject | null>(null);
```

In the `suggestion` setter, seed them before loading photos:

```ts
  @Input()
  set suggestion(value: MergeSuggestion | null) {
    this._suggestion = value;
    this.subjectA.set(value?.subject_a ?? null);
    this.subjectB.set(value?.subject_b ?? null);
    void this.loadPhotos(value);
  }
```

- [ ] **Step 3: Derive `mergeTarget` from the signals**

Replace the body of the `mergeTarget` getter so it reads the signals instead of `_suggestion`:

```ts
  get mergeTarget(): MergeTarget | null {
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

- [ ] **Step 4: Render names from the signals in the template**

In `merge-review.component.html`, replace the two static name blocks. Column A:

```html
        <div class="subject-name" [class.opacity-50]="!subjectA()?.name">
          {{ subjectA()?.name || 'Unnamed' }}
          @if (mergeTarget?.target?.id === subjectA()?.id) {
            <span class="keep-badge">keep</span>
          }
        </div>
```

Column B (identical shape, using `subjectB()`):

```html
        <div class="subject-name" [class.opacity-50]="!subjectB()?.name">
          {{ subjectB()?.name || 'Unnamed' }}
          @if (mergeTarget?.target?.id === subjectB()?.id) {
            <span class="keep-badge">keep</span>
          }
        </div>
```

Also update the `[removable]` bindings on the two `<app-merge-photo-grid>` elements to compare against the target via the signals — column A:
`[removable]="mergeTarget?.source?.id === subjectA()?.id"` and column B:
`[removable]="mergeTarget?.source?.id === subjectB()?.id"`.

And in the footer "Merge as" button, replace `mergeTarget?.target?.name` usages — these already read `mergeTarget`, so no change is needed there.

- [ ] **Step 5: Run the spec to verify everything still passes**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS (the `mergeTarget` getter and removable/photo tests still pass because the setter seeds the signals synchronously).

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html
git commit -m "refactor(merge-review): reactive local subject signals"
```

---

### Task 2: Relabel the left button to "Not the same person"

The left button records a `cannot_link` mark, so it should read "Not the same person" in both modes instead of the `canDismiss` ternary.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.html`
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`

- [ ] **Step 1: Write the failing test**

Add to `merge-review.component.spec.ts`, inside the `describe('MergeReviewComponent', …)` block:

```ts
  it('labels the left button "Not the same person" in the default (canDismiss=true) mode', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    component.suggestion = makeSuggestion(subA, subB);
    fixture.detectChanges();

    const dismissBtn = fixture.debugElement.query(By.css('button[cdkFocusInitial]'));
    expect(dismissBtn.nativeElement.textContent.trim()).toBe('Not the same person');
  });
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts -t "labels the left button"`
Expected: FAIL — actual text is "Dismiss".

- [ ] **Step 3: Replace the ternary label with a constant**

In `merge-review.component.html`, change the left button's label from:

```html
          {{ canDismiss ? 'Dismiss' : 'Not the same person' }}
```

to:

```html
          Not the same person
```

- [ ] **Step 4: Run the spec to verify it passes**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS (new test passes; the existing `canDismiss=false` label test still passes since the label is now always "Not the same person").

- [ ] **Step 5: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): relabel left button to 'Not the same person'"
```

---

### Task 3: Inline editable names + validation ladder

Make both names editable via `EditableTextComponent`, with the empty/Case-2/Case-3/Case-1 validation ladder.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`
- Modify: `src/app/components/merge-review/merge-review.component.html`
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`

**Interfaces:**
- Consumes: `subjectA`/`subjectB` signals and `get mergeTarget()` from Task 1.
- Produces: `onNameCommit(which: 'a' | 'b', rawValue: string): Promise<void>`, `nameErrorA: WritableSignal<string | null>`, `nameErrorB: WritableSignal<string | null>`.

- [ ] **Step 1: Write the failing tests**

Add to `merge-review.component.spec.ts` inside the main `describe`:

```ts
  it('Case 1: committing a new unique name calls nameSubject and updates the signal', async () => {
    const subA = makeSubject(1, null);
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    photoService.subjects.set([subA, subB]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', 'Charlie');

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, 'Charlie');
    expect(component.subjectA()?.name).toBe('Charlie');
    expect(component.nameErrorA()).toBeNull();
  });

  it('Case 2: naming a column the OTHER column\'s name is allowed (no error)', async () => {
    const subA = makeSubject(1, null);
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: 2 });
    photoService.subjects.set([subA, subB]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', 'bob'); // case-insensitive match of other column

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, 'bob');
    expect(component.nameErrorA()).toBeNull();
  });

  it('Case 3: naming a column after a THIRD subject is blocked (no backend call, error shown)', async () => {
    const subA = makeSubject(1, null);
    const subB = makeSubject(2, 'Bob');
    const third = makeSubject(3, 'Jane');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const nameSpy = vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: 3 });
    photoService.subjects.set([subA, subB, third]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', 'jane'); // case-insensitive match of third subject

    expect(nameSpy).not.toHaveBeenCalled();
    expect(component.subjectA()?.name).toBeNull(); // reverted / unchanged
    expect(component.nameErrorA()).toContain('already exists');
  });

  it('committing an empty value clears the name', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    photoService.subjects.set([subA, subB]);
    component.suggestion = makeSuggestion(subA, subB);

    await component.onNameCommit('a', '   ');

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, null);
    expect(component.subjectA()?.name).toBeNull();
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts -t "Case"`
Expected: FAIL — `component.onNameCommit` is not a function.

- [ ] **Step 3: Implement `onNameCommit` and the error signals**

In `merge-review.component.ts`, add the error signals near the other signals:

```ts
  protected nameErrorA = signal<string | null>(null);
  protected nameErrorB = signal<string | null>(null);
```

Add the method (place it after `onFaceRemovedB`):

```ts
  protected async onNameCommit(which: 'a' | 'b', rawValue: string): Promise<void> {
    const subjSig = which === 'a' ? this.subjectA : this.subjectB;
    const otherSig = which === 'a' ? this.subjectB : this.subjectA;
    const errorSig = which === 'a' ? this.nameErrorA : this.nameErrorB;
    const subject = subjSig();
    if (!subject) return;
    errorSig.set(null);

    const typed = rawValue.trim();
    const newName = typed || null;

    // Case 3: matches a DIFFERENT existing subject (not either column in this modal) → block.
    if (typed) {
      const other = otherSig();
      const conflict = this.photoService.subjects().find(
        (s) =>
          s.id !== subject.id &&
          s.id !== other?.id &&
          (s.name ?? '').toLowerCase() === typed.toLowerCase(),
      );
      if (conflict) {
        errorSig.set(`A subject named "${typed}" already exists.`);
        return; // no backend write; EditableText re-displays the unchanged signal value
      }
    }

    try {
      await this.photoService.nameSubject(subject.id, newName);
      subjSig.set({ ...subject, name: newName });
    } catch (e) {
      console.error('MergeReview: rename failed', e);
    }
  }
```

- [ ] **Step 4: Wire `EditableTextComponent` into the template**

In `merge-review.component.ts`, import and register the component:

```ts
import { EditableTextComponent } from '../editable-text/editable-text.component';
```

Add `EditableTextComponent` to the `imports` array of the `@Component` decorator.

In `merge-review.component.html`, replace column A's name block (from Task 1) with:

```html
        <div class="subject-name">
          <app-editable-text
            [value]="subjectA()?.name ?? null"
            placeholder="Unnamed"
            placeholderClass="opacity-50"
            displayClass="font-semibold"
            (commit)="onNameCommit('a', $event)"
          />
          @if (mergeTarget?.target?.id === subjectA()?.id) {
            <span class="keep-badge">keep</span>
          }
        </div>
        @if (nameErrorA()) {
          <p class="text-xs text-destructive mb-1">{{ nameErrorA() }}</p>
        }
```

Replace column B's name block with the `subjectB()` / `'b'` / `nameErrorB()` equivalent:

```html
        <div class="subject-name">
          <app-editable-text
            [value]="subjectB()?.name ?? null"
            placeholder="Unnamed"
            placeholderClass="opacity-50"
            displayClass="font-semibold"
            (commit)="onNameCommit('b', $event)"
          />
          @if (mergeTarget?.target?.id === subjectB()?.id) {
            <span class="keep-badge">keep</span>
          }
        </div>
        @if (nameErrorB()) {
          <p class="text-xs text-destructive mb-1">{{ nameErrorB() }}</p>
        }
```

- [ ] **Step 5: Run the spec to verify it passes**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS (all Case tests plus the existing suite).

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): inline editable subject names with duplicate-name block"
```

---

### Task 4: Duplicate-name guard — nudge + exit guard

When both columns end up identically named, emphasize Merge and intercept all leave paths with a confirm strip whose "Keep separate" closes without writing a `cannot_link`.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`
- Modify: `src/app/components/merge-review/merge-review.component.html`
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`

**Interfaces:**
- Consumes: `subjectA`/`subjectB` signals; existing `confirm()`, and the dismiss/close logic.
- Produces: `namesIdentical` (computed), `showExitConfirm: WritableSignal<boolean>`, `keepSeparate(): void`. The public `dismiss()`/`close()` gain an interception branch; their original bodies move to `doDismiss()`/`doClose()`.

- [ ] **Step 1: Write the failing tests**

Add to `merge-review.component.spec.ts`:

```ts
  it('dismiss() shows the exit confirm instead of dismissing when names are identical', async () => {
    const subA = makeSubject(1, 'Noah');
    const subB = makeSubject(2, 'noah'); // case-insensitive identical
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(subA, subB);

    component.dismiss();

    expect(component.namesIdentical()).toBe(true);
    expect(component.showExitConfirm()).toBe(true);
    expect(dismissSpy).not.toHaveBeenCalled();
  });

  it('keepSeparate() closes without calling dismissMergeSuggestion (no cannot_link)', async () => {
    const subA = makeSubject(1, 'Noah');
    const subB = makeSubject(2, 'Noah');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    const closedSpy = vi.fn();
    component.closed.subscribe(closedSpy);
    component.suggestion = makeSuggestion(subA, subB);

    component.dismiss();          // opens the guard
    component.keepSeparate();     // choose "Keep separate"

    expect(dismissSpy).not.toHaveBeenCalled();
    expect(closedSpy).toHaveBeenCalled();
    expect(component.showExitConfirm()).toBe(false);
  });

  it('dismiss() still dismisses directly when names differ', async () => {
    const subA = makeSubject(1, 'Alice');
    const subB = makeSubject(2, 'Bob');
    vi.spyOn(photoService, 'getSubjectPhotosWithFaces').mockResolvedValue([]);
    const dismissSpy = vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
    component.suggestion = makeSuggestion(subA, subB);

    await component.dismiss();

    expect(component.showExitConfirm()).toBe(false);
    expect(dismissSpy).toHaveBeenCalledWith(1);
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts -t "identical|keepSeparate|names differ"`
Expected: FAIL — `namesIdentical`, `showExitConfirm`, `keepSeparate` do not exist.

- [ ] **Step 3: Add the computed, signal, and guarded methods**

In `merge-review.component.ts`, import `computed`:

```ts
import { Component, Input, Output, EventEmitter, inject, signal, computed, ChangeDetectionStrategy, ViewChild, ElementRef, HostListener } from '@angular/core';
```

Add near the other signals:

```ts
  protected showExitConfirm = signal(false);

  protected namesIdentical = computed(() => {
    const a = this.subjectA()?.name?.trim().toLowerCase();
    const b = this.subjectB()?.name?.trim().toLowerCase();
    return !!a && !!b && a === b;
  });
```

Rename the existing `dismiss()` body to `doDismiss()` and the existing `close()` body to `doClose()`, then add guarded public entry points and `keepSeparate()`:

```ts
  async dismiss() {
    if (this.namesIdentical() && !this.submitting()) {
      this.showExitConfirm.set(true);
      return;
    }
    await this.doDismiss();
  }

  private async doDismiss() {
    if (!this._suggestion || this.submitting()) return;
    if (!this.canDismiss) {
      this.dismissed.emit();
      return;
    }
    this.submitting.set(true);
    try {
      await this.photoService.dismissMergeSuggestion(this._suggestion.id);
      this.dismissed.emit();
    } catch (e) {
      console.error('MergeReview: dismiss failed', e);
    } finally {
      this.submitting.set(false);
    }
  }

  close() {
    if (this.namesIdentical() && !this.submitting()) {
      this.showExitConfirm.set(true);
      return;
    }
    this.doClose();
  }

  private doClose() {
    if (!this._suggestion || this.submitting()) return;
    this.closed.emit();
  }

  /** Exit-guard "Keep separate": abandon the merge WITHOUT writing a cannot_link mark. */
  protected keepSeparate() {
    this.showExitConfirm.set(false);
    this.doClose();
  }

  protected confirmFromGuard() {
    this.showExitConfirm.set(false);
    void this.confirm();
  }
```

(Delete the old `dismiss()` and `close()` bodies you renamed — do not leave duplicates.)

- [ ] **Step 4: Add the nudge class and confirm strip to the template**

In `merge-review.component.html`, add a nudge ring to the Merge button by appending a conditional class binding:

```html
        <button
          class="px-4 py-2 rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
          [class.ring-2]="namesIdentical()"
          [class.ring-primary]="namesIdentical()"
          [class.animate-pulse]="namesIdentical()"
          (click)="confirm()"
          [disabled]="submitting()"
        >
          Merge as <span [class.opacity-50]="!mergeTarget?.target?.name">{{ mergeTarget?.target?.name || 'Unnamed' }}</span>
        </button>
```

Then add the confirm strip just inside `.modal-actions`, before the two existing buttons:

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
        }
```

(Leave the two existing footer buttons as-is after this block; the confirm strip sits alongside them with `mr-auto` pushing it left.)

- [ ] **Step 5: Run the spec to verify it passes**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS (guard tests plus the existing suite — note the existing "dismiss calls dismissMergeSuggestion" test uses Alice/Bob, so it stays on the direct path).

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.html src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): duplicate-name nudge and exit guard"
```

---

### Task 5: `confirmed` emits the surviving subject id

The modal must tell its parent which subject survived, so subject-detail can decide reload-vs-navigate.

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`
- Test: `src/app/components/merge-review/merge-review.component.spec.ts`

**Interfaces:**
- Produces: `confirmed: EventEmitter<number>` — emits `mergeTarget.target.id` on success.

- [ ] **Step 1: Update the existing confirm test to assert the emitted id**

In `merge-review.component.spec.ts`, find the test `confirm calls mergeSubjects with correct target/source then emits confirmed` and change its final assertion from `expect(confirmedSpy).toHaveBeenCalled();` to:

```ts
    expect(confirmedSpy).toHaveBeenCalledWith(1); // subA (id 1) is the named target
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts -t "confirm calls mergeSubjects"`
Expected: FAIL — `confirmed` currently emits `undefined`.

- [ ] **Step 3: Change the output type and emit the id**

In `merge-review.component.ts`, change the output declaration:

```ts
  @Output() confirmed = new EventEmitter<number>();
```

In `confirm()`, change `this.confirmed.emit();` to:

```ts
      this.confirmed.emit(target.target.id);
```

- [ ] **Step 4: Run the spec to verify it passes**

Run: `pnpm exec vitest run src/app/components/merge-review/merge-review.component.spec.ts`
Expected: PASS. (people-view binds `(confirmed)="onConfirmed()"` and simply ignores the new argument — no change needed there.)

- [ ] **Step 5: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "feat(merge-review): emit surviving subject id from confirmed"
```

---

### Task 6: Subject-detail uses the modal + dead-code cleanup

Replace subject-detail's inline similar-subjects Merge/Dismiss buttons with a Review action that opens `MergeReviewComponent`, and remove the now-dead methods.

**Files:**
- Modify: `src/app/components/subject-detail/subject-detail.component.ts`
- Modify: `src/app/components/subject-detail/subject-detail.component.html`
- Test: `src/app/components/subject-detail/subject-detail.component.spec.ts`

**Interfaces:**
- Consumes: `MergeReviewComponent` with `[suggestion]`, `(confirmed)="…($event)"` (surviving id from Task 5), `(dismissed)`, `(closed)`.
- Produces: `openReview(s: MergeSuggestion): void`, `onReviewConfirmed(survivingId: number): void`, `onReviewDismissed(): void`, `onReviewClosed(): void`, `reviewingSuggestion: WritableSignal<MergeSuggestion | null>`.

- [ ] **Step 1: Extend the test stub and write the failing tests**

In `subject-detail.component.spec.ts`, add two mocks to `SubjectDetailPhotoServiceStub` (so the embedded modal can construct):

```ts
  subjects = signal([]);
  getSubjectPhotosWithFaces = vi.fn().mockResolvedValue([]);
```

Add a new `describe` block at the end of the file:

```ts
describe('SubjectDetailComponent — similar-subjects review flow', () => {
  let stub: SubjectDetailPhotoServiceStub;

  beforeEach(() => {
    stub = new SubjectDetailPhotoServiceStub();
    stub.getSubjectDetail.mockResolvedValue(subjectDetail());
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'subject/:id', component: SubjectDetailComponent }]),
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        { provide: PhotoService, useValue: stub },
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    });
  });

  async function mount() {
    const harness = await RouterTestingHarness.create('/subject/1');
    harness.detectChanges();
    await harness.fixture.whenStable();
    harness.detectChanges();
    const cmp = harness.routeDebugElement!.componentInstance as SubjectDetailComponent;
    return { harness, cmp };
  }

  it('onReviewConfirmed reloads in place when the current subject survives', async () => {
    const { cmp } = await mount();
    stub.getSubjectDetail.mockClear();
    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate');

    (cmp as any).onReviewConfirmed(1); // survivor == current subject id

    expect(navigateSpy).not.toHaveBeenCalled();
    expect(stub.getSubjectDetail).toHaveBeenCalledWith(1);
  });

  it('onReviewConfirmed navigates when a different subject survives', async () => {
    const { cmp } = await mount();
    const router = TestBed.inject(Router);
    const navigateSpy = vi.spyOn(router, 'navigate').mockResolvedValue(true);

    (cmp as any).onReviewConfirmed(2); // survivor != current subject id

    expect(navigateSpy).toHaveBeenCalledWith(['/subject', 2]);
  });

  it('onReviewDismissed removes the reviewed suggestion from the list', async () => {
    const { cmp } = await mount();
    const suggestion = { id: 7, subject_a: { id: 1, name: 'Sofía', thumbnail_face_id: null, type: 'person', added_at: 0 }, subject_b: { id: 2, name: null, thumbnail_face_id: null, type: 'person', added_at: 0 }, score: 0.9 };
    (cmp as any).similarSubjects.set([suggestion]);
    (cmp as any).openReview(suggestion);

    (cmp as any).onReviewDismissed();

    expect((cmp as any).similarSubjects()).toEqual([]);
    expect((cmp as any).reviewingSuggestion()).toBeNull();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run src/app/components/subject-detail/subject-detail.component.spec.ts -t "review flow"`
Expected: FAIL — `onReviewConfirmed`/`openReview`/`reviewingSuggestion` do not exist.

- [ ] **Step 3: Update the component TypeScript**

In `subject-detail.component.ts`, add the import and register the component:

```ts
import { MergeReviewComponent } from '../merge-review/merge-review.component';
```

Add `MergeReviewComponent` to the `imports` array.

Add the signal near `similarSubjects`:

```ts
  protected reviewingSuggestion = signal<MergeSuggestion | null>(null);
```

Remove the now-dead `mergeSimilar()` and `dismissSimilar()` methods entirely. Add the review handlers:

```ts
  protected openReview(suggestion: MergeSuggestion) {
    this.reviewingSuggestion.set(suggestion);
  }

  protected onReviewConfirmed(survivingId: number) {
    const current = this.subjectId();
    this.reviewingSuggestion.set(null);
    if (current === null) return;
    if (survivingId === current) {
      void this.loadData(current);
    } else {
      void this.router.navigate(['/subject', survivingId]);
    }
  }

  protected onReviewDismissed() {
    const current = this.reviewingSuggestion();
    if (current) {
      this.similarSubjects.update((list) => list.filter((s) => s.id !== current.id));
    }
    this.reviewingSuggestion.set(null);
  }

  protected onReviewClosed() {
    this.reviewingSuggestion.set(null);
  }
```

- [ ] **Step 4: Update the template**

In `subject-detail.component.html`, inside the similar-subjects `@for`, replace the two inline buttons (the `Merge` button calling `mergeSimilar` and the `Dismiss` button calling `dismissSimilar`) with a single Review button:

```html
              <button
                class="px-2 py-1 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
                (click)="openReview(suggestion)"
              >
                Review
              </button>
```

Then add the modal instance just before the existing `<app-confirm-merge-dialog>` element:

```html
  <app-merge-review
    [suggestion]="reviewingSuggestion()"
    (confirmed)="onReviewConfirmed($event)"
    (dismissed)="onReviewDismissed()"
    (closed)="onReviewClosed()"
  />
```

- [ ] **Step 5: Run the spec to verify it passes**

Run: `pnpm exec vitest run src/app/components/subject-detail/subject-detail.component.spec.ts`
Expected: PASS (new review-flow tests plus the existing tagging/name-conflict tests).

- [ ] **Step 6: Run the full suite and the Angular build to confirm no regressions**

Run: `pnpm test`
Expected: PASS (whole Vitest suite).

Run: `pnpm build`
Expected: Angular build succeeds — confirms `mergeSimilar`/`dismissSimilar` removal left no dangling template references and no unused-import errors.

- [ ] **Step 7: Commit**

```bash
git add src/app/components/subject-detail/subject-detail.component.ts src/app/components/subject-detail/subject-detail.component.html src/app/components/subject-detail/subject-detail.component.spec.ts
git commit -m "feat(subject-detail): route similar-subject merges through MergeReviewComponent"
```

---

## Self-Review Notes

- **Spec coverage:** Feature 1 state model → Task 1; left-button relabel → Task 2; validation ladder (empty/Case 2/Case 3/Case 1) → Task 3; nudge + exit guard incl. "Keep separate" no-constraint → Task 4; `confirmed` emits surviving id → Task 5; Feature 2 (keep list, open modal, navigate-vs-reload, dead-code cleanup) → Task 6. Cleanup section (no file deletions) satisfied by Task 6 removing `mergeSimilar`/`dismissSimilar` only.
- **Types:** `subjectA`/`subjectB: WritableSignal<Subject | null>` and `get mergeTarget()` (Task 1) are consumed consistently in Tasks 3–4; `confirmed: EventEmitter<number>` (Task 5) consumed by Task 6's `onReviewConfirmed(survivingId: number)`.
- **Out of scope confirmed:** `ConfirmMergeDialogComponent` and `subject-person-card` untouched across all tasks.
