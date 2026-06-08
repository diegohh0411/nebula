# Inline "Add a name" on People Grid Cards — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Google-Photos-style hover-reveal inline input to unnamed People grid cards so users can name clusters without leaving the page.

**Architecture:** New editing state lives entirely in `PeopleViewComponent` (three signals: `editingSubjectId`, `editingName`, `namingConflict`). `MergeReviewComponent` gets a single `canDismiss` input so it can be reused for naming conflicts (where "Not the same person" dismissal via API doesn't apply). No new components are added.

**Tech Stack:** Angular 20, signals, `@ViewChildren`, Vitest + `@analogjs/vitest-angular`, Tailwind CSS.

---

### Files Changed

| File | Change |
|---|---|
| `src/app/components/merge-review/merge-review.component.ts` | Add `@Input() canDismiss = true` |
| `src/app/components/merge-review/merge-review.component.html` | Dismiss button label conditional |
| `src/app/components/merge-review/merge-review.component.spec.ts` | One new test for `canDismiss=false` |
| `src/app/components/people-view/people-view.component.ts` | New signals + 6 methods |
| `src/app/components/people-view/people-view.component.html` | Hover-reveal input affordance + second merge-review outlet |
| `src/app/components/people-view/people-view.component.spec.ts` | New file, 4 tests |

---

### Task 1: Extend `MergeReviewComponent` with `canDismiss`

**Files:**
- Modify: `src/app/components/merge-review/merge-review.component.ts`
- Modify: `src/app/components/merge-review/merge-review.component.html`
- Modify: `src/app/components/merge-review/merge-review.component.spec.ts`

- [ ] **Step 1: Write the failing test**

Add this test to the existing `describe` block in `src/app/components/merge-review/merge-review.component.spec.ts`:

```typescript
it('with canDismiss=false, dismiss() emits dismissed without calling dismissMergeSuggestion', async () => {
  const subA = makeSubject(1, 'Alice');
  const subB = makeSubject(2, null);
  vi.spyOn(photoService, 'getSubjectPhotos').mockResolvedValue([]);
  vi.spyOn(photoService, 'dismissMergeSuggestion').mockResolvedValue(undefined);
  const dismissedSpy = vi.fn();
  component.dismissed.subscribe(dismissedSpy);

  component.canDismiss = false;
  component.suggestion = makeSuggestion(subA, subB);
  await component.dismiss();

  expect(photoService.dismissMergeSuggestion).not.toHaveBeenCalled();
  expect(dismissedSpy).toHaveBeenCalled();
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
pnpm test --run 2>&1 | tail -20
```

Expected: FAIL — `Property 'canDismiss' does not exist` or `dismissMergeSuggestion was called`.

- [ ] **Step 3: Add `canDismiss` input to the component class**

In `src/app/components/merge-review/merge-review.component.ts`, add the input after the existing `@Output()` declarations and replace the `dismiss()` method:

```typescript
@Input() canDismiss = true;
```

Replace the existing `dismiss()` method (lines ~107-118) with:

```typescript
async dismiss() {
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
```

- [ ] **Step 4: Update the dismiss button label in the template**

In `src/app/components/merge-review/merge-review.component.html`, replace the dismiss button (lines ~42-49):

```html
<button
  class="px-4 py-2 rounded-md border border-border hover:bg-muted transition-colors text-muted-foreground"
  cdkFocusInitial
  (click)="dismiss()"
  [disabled]="submitting()"
>
  {{ canDismiss ? 'Dismiss' : 'Cancel' }}
</button>
```

- [ ] **Step 5: Run all tests to verify they pass**

```bash
pnpm test --run 2>&1 | tail -10
```

Expected: all 11 tests pass (10 existing + 1 new).

- [ ] **Step 6: Commit**

```bash
git add src/app/components/merge-review/merge-review.component.ts \
        src/app/components/merge-review/merge-review.component.html \
        src/app/components/merge-review/merge-review.component.spec.ts
git commit -m "$(cat <<'EOF'
feat(merge-review): add canDismiss input for naming-conflict reuse

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: People grid — inline editing state and template

**Files:**
- Create: `src/app/components/people-view/people-view.component.spec.ts`
- Modify: `src/app/components/people-view/people-view.component.ts`
- Modify: `src/app/components/people-view/people-view.component.html`

- [ ] **Step 1: Create the spec file with three failing tests**

Create `src/app/components/people-view/people-view.component.spec.ts`:

```typescript
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { By } from '@angular/platform-browser';
import { provideRouter } from '@angular/router';
import { Subject as RxSubject } from 'rxjs';
import { PeopleViewComponent } from './people-view.component';
import { PhotoService } from '../../services/photo.service';
import { TauriEventsService } from '../../services/tauri-events.service';
import { Subject } from '../../models/models';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

const mockTauriEvents = {
  pipelineStats$: new RxSubject(),
  imageAdded$: new RxSubject(),
  imageUpdated$: new RxSubject(),
  imageRemoved$: new RxSubject(),
  modelDownloadProgress$: new RxSubject(),
};

const makeSubject = (id: number, name: string | null): Subject => ({
  id, name, thumbnail_face_id: null, type: 'person', added_at: 0,
});

describe('PeopleViewComponent — inline naming', () => {
  let component: PeopleViewComponent;
  let fixture: ComponentFixture<PeopleViewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [PeopleViewComponent],
      providers: [
        PhotoService,
        provideRouter([]),
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(PeopleViewComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);

    vi.spyOn(photoService, 'loadSubjects').mockResolvedValue(undefined);
    vi.spyOn(photoService, 'getMergeSuggestions').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('');
  });

  it('calls nameSubject with id and trimmed name when Enter is pressed', async () => {
    photoService.subjects.set([makeSubject(1, null)]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    fixture.detectChanges();

    const hint = fixture.debugElement.query(By.css('[data-testid="add-name-hint"]'));
    hint.triggerEventHandler('click', new MouseEvent('click'));
    fixture.detectChanges();

    component.editingName.set('  Alice  ');
    fixture.detectChanges();

    const input = fixture.debugElement.query(By.css('[data-testid="name-input"]'));
    input.triggerEventHandler('keydown', new KeyboardEvent('keydown', { key: 'Enter' }));
    await fixture.whenStable();

    expect(photoService.nameSubject).toHaveBeenCalledWith(1, 'Alice');
  });

  it('shows name on card immediately before nameSubject resolves (optimistic update)', async () => {
    photoService.subjects.set([makeSubject(1, null)]);
    let resolve!: (v: { duplicate_subject_id: null }) => void;
    vi.spyOn(photoService, 'nameSubject').mockReturnValue(
      new Promise(r => { resolve = r; })
    );
    fixture.detectChanges();

    const hint = fixture.debugElement.query(By.css('[data-testid="add-name-hint"]'));
    hint.triggerEventHandler('click', new MouseEvent('click'));
    fixture.detectChanges();

    component.editingName.set('Alice');
    const input = fixture.debugElement.query(By.css('[data-testid="name-input"]'));
    input.triggerEventHandler('keydown', new KeyboardEvent('keydown', { key: 'Enter' }));
    fixture.detectChanges();

    // Name is visible before service resolves
    expect(photoService.subjects()[0].name).toBe('Alice');
    resolve({ duplicate_subject_id: null });
  });

  it('clicking the card link does not enter editing mode', () => {
    photoService.subjects.set([makeSubject(1, null)]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: null });
    fixture.detectChanges();

    const cardLink = fixture.debugElement.query(By.css('a[routerLink]'));
    cardLink.triggerEventHandler('click', new MouseEvent('click', { bubbles: true }));
    fixture.detectChanges();

    expect(component.editingSubjectId()).toBeNull();
  });
});
```

- [ ] **Step 2: Run tests to confirm all three fail**

```bash
pnpm test --run 2>&1 | tail -20
```

Expected: 3 new tests fail (component doesn't have `editingSubjectId`, `editingName`, or the template elements yet).

- [ ] **Step 3: Update `people-view.component.ts` with new signals and methods**

Replace the full content of `src/app/components/people-view/people-view.component.ts`:

```typescript
import { Component, inject, OnInit, signal, ViewChildren, QueryList, ElementRef } from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';
import { MergeSuggestion, Subject } from '../../models/models';
import { RouterLink } from '@angular/router';
import { MergeReviewComponent } from '../merge-review/merge-review.component';

@Component({
  selector: 'app-people-view',
  standalone: true,
  imports: [CommonModule, RouterLink, MergeReviewComponent],
  templateUrl: './people-view.component.html',
  styleUrl: './people-view.component.css'
})
export class PeopleViewComponent implements OnInit {
  protected photoService = inject(PhotoService);
  protected faceCropUrls = signal<Record<number, string>>({});
  protected mergeSuggestions = signal<MergeSuggestion[]>([]);
  protected suggestionCropUrls = signal<Record<number, string>>({});
  protected reviewingSuggestion = signal<MergeSuggestion | null>(null);

  editingSubjectId = signal<number | null>(null);
  editingName = signal<string>('');
  protected namingConflict = signal<MergeSuggestion | null>(null);

  @ViewChildren('nameInput') private nameInputRefs!: QueryList<ElementRef<HTMLInputElement>>;

  async ngOnInit() {
    await this.photoService.loadSubjects();
    void this.loadMergeSuggestions();
    void this.loadThumbnails();
  }

  private async loadMergeSuggestions() {
    try {
      const suggestions = await this.photoService.getMergeSuggestions(3);
      this.mergeSuggestions.set(suggestions);
      void this.loadSuggestionCrops(suggestions);
    } catch (e) {
      console.error('Failed to load merge suggestions', e);
    }
  }

  private async loadSuggestionCrops(suggestions: MergeSuggestion[]) {
    const ids = new Set<number>();
    for (const s of suggestions) {
      if (s.subject_a.thumbnail_face_id) ids.add(s.subject_a.thumbnail_face_id);
      if (s.subject_b.thumbnail_face_id) ids.add(s.subject_b.thumbnail_face_id);
    }
    const urls: Record<number, string> = {};
    await Promise.all(
      [...ids].map(async (faceId) => {
        try {
          const path = await this.photoService.getFaceCrop(faceId);
          const url = this.photoService.thumbnailUrl(path);
          if (url) urls[faceId] = url;
        } catch {}
      })
    );
    this.suggestionCropUrls.set(urls);
  }

  private async loadThumbnails() {
    const subjects = this.photoService.subjects();
    const urls: Record<number, string> = {};

    await Promise.all(subjects.map(async (s) => {
      if (s.thumbnail_face_id) {
        try {
          const path = await this.photoService.getFaceCrop(s.thumbnail_face_id);
          const url = this.photoService.thumbnailUrl(path);
          if (url) urls[s.id] = url;
        } catch (e) {
          console.error(`Failed to load thumbnail for subject ${s.id}`, e);
        }
      }
    }));

    this.faceCropUrls.set(urls);
  }

  protected openReview(suggestion: MergeSuggestion) {
    this.reviewingSuggestion.set(suggestion);
  }

  async onConfirmed() {
    this.reviewingSuggestion.set(null);
    await Promise.all([this.loadThumbnails(), this.loadMergeSuggestions()]);
  }

  async onDismissed() {
    const current = this.reviewingSuggestion();
    if (current) {
      this.mergeSuggestions.update((list) => list.filter((s) => s.id !== current.id));
    }
    this.reviewingSuggestion.set(null);
  }

  onClosed() {
    this.reviewingSuggestion.set(null);
  }

  protected getThumbUrl(subject: Subject): string | null {
    if (!subject.thumbnail_face_id) return null;
    return this.suggestionCropUrls()[subject.thumbnail_face_id] ?? this.faceCropUrls()[subject.id] ?? null;
  }

  protected startEditing(subject: Subject, event: Event): void {
    event.stopPropagation();
    this.editingSubjectId.set(subject.id);
    this.editingName.set('');
  }

  protected async commitName(subject: Subject): Promise<void> {
    if (this.editingSubjectId() !== subject.id) return;
    const name = this.editingName().trim();
    if (!name) { this.cancelEditing(); return; }

    this.photoService.subjects.update(subjects =>
      subjects.map(s => s.id === subject.id ? { ...s, name } : s)
    );
    this.editingSubjectId.set(null);
    this.editingName.set('');

    const result = await this.photoService.nameSubject(subject.id, name);

    if (result.duplicate_subject_id) {
      const duplicate = this.photoService.subjects().find(s => s.id === result.duplicate_subject_id);
      if (duplicate) {
        const current = this.photoService.subjects().find(s => s.id === subject.id) ?? { ...subject, name };
        this.namingConflict.set({ id: -1, subject_a: duplicate, subject_b: current, score: 1.0 });
      }
    }
  }

  protected cancelEditing(): void {
    this.editingSubjectId.set(null);
    this.editingName.set('');
  }

  protected onKeydown(event: KeyboardEvent, subject: Subject): void {
    if (event.key === 'Enter') {
      void this.commitName(subject);
    } else if (event.key === 'Escape') {
      this.cancelEditing();
    } else if (event.key === 'Tab') {
      event.preventDefault();
      const subjects = this.photoService.subjects();
      const idx = subjects.findIndex(s => s.id === subject.id);
      const nextUnnamed = subjects.slice(idx + 1).find(s => !s.name) ?? null;
      void this.commitName(subject);
      if (nextUnnamed) {
        this.editingSubjectId.set(nextUnnamed.id);
        this.editingName.set('');
        setTimeout(() => this.nameInputRefs.first?.nativeElement.focus(), 0);
      }
    }
  }

  protected onConflictConfirmed(): void {
    this.namingConflict.set(null);
    void Promise.all([this.loadThumbnails(), this.loadMergeSuggestions()]);
  }

  protected onConflictDismissed(): void {
    this.namingConflict.set(null);
  }
}
```

- [ ] **Step 4: Update `people-view.component.html` with hover-reveal input and conflict outlet**

Replace the full content of `src/app/components/people-view/people-view.component.html`:

```html
<div class="people-container p-8">
  <h1 class="text-3xl font-bold mb-8">People</h1>

  @if (mergeSuggestions().length > 0) {
    <div class="mb-8 p-4 rounded-lg border border-accent/30 bg-accent/5">
      <h2 class="text-lg font-semibold mb-3">Possible Duplicates</h2>
      <div class="flex flex-col gap-3">
        @for (suggestion of mergeSuggestions(); track suggestion.id) {
          <div class="flex items-center gap-4 p-3 rounded-md bg-background border border-border">
            <div class="flex items-center gap-3 flex-1 min-w-0">
              <div class="flex items-center gap-2">
                <div class="w-12 h-12 rounded-full overflow-hidden border border-border bg-muted flex items-center justify-center shrink-0">
                  @if (getThumbUrl(suggestion.subject_a)) {
                    <img [src]="getThumbUrl(suggestion.subject_a)" alt="" class="w-full h-full object-cover" />
                  } @else {
                    <span class="text-lg text-muted-foreground">👤</span>
                  }
                </div>
                <div class="w-12 h-12 rounded-full overflow-hidden border border-border bg-muted flex items-center justify-center shrink-0">
                  @if (getThumbUrl(suggestion.subject_b)) {
                    <img [src]="getThumbUrl(suggestion.subject_b)" alt="" class="w-full h-full object-cover" />
                  } @else {
                    <span class="text-lg text-muted-foreground">👤</span>
                  }
                </div>
              </div>
              <div class="min-w-0">
                <div class="font-medium truncate">
                  <span [class.opacity-50]="!suggestion.subject_a.name">{{ suggestion.subject_a.name || 'Unnamed' }}</span>
                  &amp;
                  <span [class.opacity-50]="!suggestion.subject_b.name">{{ suggestion.subject_b.name || 'Unnamed' }}</span>
                </div>
                <div class="text-xs text-muted-foreground">
                  {{ suggestion.score | percent }} match
                </div>
              </div>
            </div>
            <div class="shrink-0">
              <button
                class="px-3 py-1.5 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
                (click)="openReview(suggestion)"
              >
                Review
              </button>
            </div>
          </div>
        }
      </div>
    </div>
  }

  <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 gap-6">
    @for (subject of photoService.subjects(); track subject.id) {
      <a
        class="group cursor-pointer flex flex-col items-center gap-3 transition-transform hover:scale-105"
        [routerLink]="['/subject', subject.id]"
      >
        <div class="w-32 h-32 rounded-full overflow-hidden border-2 border-border group-hover:border-accent bg-muted flex items-center justify-center transition-colors">
          @if (faceCropUrls()[subject.id]) {
            <img [src]="faceCropUrls()[subject.id]" alt="Face Crop" class="w-full h-full object-cover" />
          } @else {
            <span class="text-4xl text-muted-foreground">👤</span>
          }
        </div>

        <div class="text-center h-6 flex items-center justify-center">
          @if (subject.name) {
            <span class="font-medium block">{{ subject.name }}</span>
          } @else if (editingSubjectId() === subject.id) {
            <input
              #nameInput
              data-testid="name-input"
              class="w-28 text-center text-sm border border-border rounded px-2 py-0.5 focus:outline-none focus:border-accent bg-background"
              [value]="editingName()"
              (input)="editingName.set($any($event.target).value)"
              (keydown)="onKeydown($event, subject)"
              (blur)="commitName(subject)"
              (click)="$event.stopPropagation()"
              autofocus
            />
          } @else {
            <span
              data-testid="add-name-hint"
              class="text-xs text-accent opacity-0 group-hover:opacity-100 cursor-pointer transition-opacity select-none"
              (click)="startEditing(subject, $event)"
            >+ Add a name</span>
          }
        </div>
      </a>
    } @empty {
      <div class="col-span-full py-20 text-center text-muted-foreground">
        <p class="text-lg mb-2">No subjects discovered yet.</p>
        <p class="text-sm">Start adding photos with faces to see people here!</p>
      </div>
    }
  </div>
</div>

<!-- Merge suggestion review modal -->
<app-merge-review
  [suggestion]="reviewingSuggestion()"
  (confirmed)="onConfirmed()"
  (dismissed)="onDismissed()"
  (closed)="onClosed()"
/>

<!-- Naming conflict merge modal (no dismiss-to-API) -->
<app-merge-review
  [suggestion]="namingConflict()"
  [canDismiss]="false"
  (confirmed)="onConflictConfirmed()"
  (dismissed)="onConflictDismissed()"
  (closed)="onConflictDismissed()"
/>
```

- [ ] **Step 5: Run all tests**

```bash
pnpm test --run 2>&1 | tail -15
```

Expected: all 14 tests pass (11 from Task 1 + 3 new).

- [ ] **Step 6: Commit**

```bash
git add src/app/components/people-view/people-view.component.ts \
        src/app/components/people-view/people-view.component.html \
        src/app/components/people-view/people-view.component.spec.ts
git commit -m "$(cat <<'EOF'
feat(people-view): inline hover-reveal Add a name input on unnamed cards

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Name conflict — write and verify test

**Files:**
- Modify: `src/app/components/people-view/people-view.component.spec.ts`

The conflict-handling code was already added in Task 2's TS. This task adds and verifies the test that covers it.

- [ ] **Step 1: Add conflict test to the spec file**

In `src/app/components/people-view/people-view.component.spec.ts`, add a second `describe` block after the first:

```typescript
describe('PeopleViewComponent — name conflict', () => {
  let component: PeopleViewComponent;
  let fixture: ComponentFixture<PeopleViewComponent>;
  let photoService: PhotoService;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [PeopleViewComponent],
      providers: [
        PhotoService,
        provideRouter([]),
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    }).compileComponents();

    fixture = TestBed.createComponent(PeopleViewComponent);
    component = fixture.componentInstance;
    photoService = TestBed.inject(PhotoService);

    vi.spyOn(photoService, 'loadSubjects').mockResolvedValue(undefined);
    vi.spyOn(photoService, 'getMergeSuggestions').mockResolvedValue([]);
    vi.spyOn(photoService, 'getFaceCrop').mockResolvedValue('');
  });

  it('sets namingConflict with synthetic suggestion when duplicate_subject_id returned', async () => {
    const current = makeSubject(1, null);
    const duplicate = makeSubject(2, 'Emma');
    photoService.subjects.set([current, duplicate]);
    vi.spyOn(photoService, 'nameSubject').mockResolvedValue({ duplicate_subject_id: 2 });

    fixture.detectChanges();
    component.editingSubjectId.set(1);
    component.editingName.set('Emma');
    await component['commitName'](current);

    const conflict = component['namingConflict']();
    expect(conflict).not.toBeNull();
    expect(conflict!.id).toBe(-1);
    expect(conflict!.subject_a.id).toBe(2);
    expect(conflict!.subject_b.id).toBe(1);
  });
});
```

- [ ] **Step 2: Run tests**

```bash
pnpm test --run 2>&1 | tail -15
```

Expected: all 15 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/app/components/people-view/people-view.component.spec.ts
git commit -m "$(cat <<'EOF'
test(people-view): cover name conflict routing to merge modal

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Build check and TypeScript compile

- [ ] **Step 1: Run the Angular build to catch any type errors**

```bash
pnpm exec ng build --configuration development 2>&1 | tail -20
```

Expected: Build succeeds with no errors. If there are type errors, fix them before proceeding.

- [ ] **Step 2: Run full test suite one final time**

```bash
pnpm test --run 2>&1 | tail -10
```

Expected: all 15 tests pass, 0 failures.

- [ ] **Step 3: Commit if any build-fix changes were made, then push**

If build fixes were needed in Step 1:

```bash
git add -p
git commit -m "$(cat <<'EOF'
fix(people-view): resolve TypeScript compile errors from inline naming feature

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

Then push the branch and open a PR:

```bash
git push -u origin worktree-tt-26-inline-add-name-people-grid
gh pr create \
  --title "feat: Inline \"Add a name\" on People grid cards (TT-26)" \
  --body "$(cat <<'EOF'
## Summary
- Adds hover-reveal inline input to unnamed People grid cards (Google Photos style)
- Submitting via Enter/blur calls `nameSubject` with optimistic update for instant feedback
- Name conflicts open the existing `MergeReviewComponent` with `canDismiss=false` so the user can verify faces before merging
- Tab moves focus to the next unnamed card for rapid sequential naming
- Detail-page naming flow is unchanged

## Test plan
- [ ] Open the app, navigate to People grid — unnamed cards show no label by default
- [ ] Hover an unnamed card — "+ Add a name" hint appears
- [ ] Click hint — inline input appears, focused
- [ ] Type a name and press Enter — card immediately shows name, input disappears
- [ ] Type a name that already exists on another cluster — merge review modal opens with Cancel (not Dismiss)
- [ ] Cancel the conflict → modal closes, both clusters retain the typed name
- [ ] Confirm the conflict → clusters merge, grid reloads
- [ ] Press Tab after naming — focus moves to next unnamed card's input
- [ ] Press Escape — input disappears, card returns to hover-reveal state
- [ ] Clicking the card face/body (not the hint) still navigates to subject detail
- [ ] Detail-page name editing still works as before
- [ ] `pnpm test --run` passes all 15 tests

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review Notes

**Spec coverage check:**
- ✅ Hover-reveal affordance → Task 2 template
- ✅ `stopPropagation` on hint/input → `startEditing` + `(click)="$event.stopPropagation()"` on input
- ✅ Submit via Enter/blur → `onKeydown` + `(blur)="commitName(subject)"`
- ✅ Optimistic update → `photoService.subjects.update(...)` before `await nameSubject`
- ✅ Name conflict → `namingConflict` signal + second `app-merge-review` outlet with `canDismiss=false`
- ✅ Keyboard flow / Tab to next → `onKeydown` Tab case + `@ViewChildren` focus
- ✅ No regression on detail page — nothing in detail component touched
- ✅ `canDismiss` test → Task 1
- ✅ Inline submit test → Task 2
- ✅ Optimistic update test → Task 2
- ✅ Card click navigates test → Task 2
- ✅ Conflict routing test → Task 3

**Double-commit guard:** `commitName` checks `if (this.editingSubjectId() !== subject.id) return` at the top, preventing the blur event (fired when Angular destroys the input) from re-triggering after Enter already committed.
