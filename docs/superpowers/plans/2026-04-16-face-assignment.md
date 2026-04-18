# Face Assignment — "Faces to Add" Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface unassigned faces in the lightbox sidebar with an inline Spartan combobox popover for assigning them to existing or new subjects.

**Architecture:** Two new Rust DB functions (`assign_face_to_subject`, `create_subject_for_face`) exposed as Tauri commands. A new `FaceAssignPopoverComponent` uses `brn-popover` + `hlm-command` for fuzzy subject search. `LightboxComponent` filters faces client-side and adds a collapsible "Faces to add" section that renders the popover per face, updating optimistically on assignment.

**Tech Stack:** Rust/SQLite (sqlx), Tauri commands, Angular 20 signals, Spartan.ng (`@spartan-ng/brain/popover`, `@spartan-ng/brain/command`, `@spartan-ng/helm/popover`, `@spartan-ng/helm/command`), Tailwind CSS, lucide-angular.

---

## File Map

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `src-tauri/src/db.rs` | Add `assign_face_to_subject`, `create_subject_for_face` |
| Modify | `src-tauri/src/commands.rs` | Add 2 Tauri commands |
| Modify | `src-tauri/src/lib.rs` | Register the 2 new commands in invoke_handler |
| Modify | `src/app/libs/ui/command/src/lib/hlm-command-input.ts` | Replace @ng-icons with lucide-angular |
| Modify | `src/app/app.config.ts` | Add Plus icon to lucide registry |
| Modify | `src/app/services/photo.service.ts` | Add `assignFaceToSubject`, `createSubjectForFace` |
| Create | `src/app/components/face-assign-popover/face-assign-popover.component.ts` | Popover component logic |
| Create | `src/app/components/face-assign-popover/face-assign-popover.component.html` | Popover template |
| Create | `src/app/components/face-assign-popover/face-assign-popover.component.css` | Popover styles |
| Modify | `src/app/components/lightbox/lightbox.component.ts` | Add unassigned face signals + handler |
| Modify | `src/app/components/lightbox/lightbox.component.html` | Add "Faces to add" sidebar section |

---

## Task 1: DB — `assign_face_to_subject`

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add the function at the bottom of db.rs, before the closing of the file**

```rust
pub async fn assign_face_to_subject(pool: &SqlitePool, face_id: i64, subject_id: i64) -> Result<()> {
    sqlx::query("UPDATE faces SET subject_id = ? WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 2: Build to verify no compile errors**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```
Expected: no `^error` lines.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add assign_face_to_subject db function"
```

---

## Task 2: DB — `create_subject_for_face`

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Add the function directly after `assign_face_to_subject`**

```rust
pub async fn create_subject_for_face(pool: &SqlitePool, face_id: i64, name: Option<&str>) -> Result<Subject> {
    let subject_id = insert_subject(pool, name, "person").await?;
    sqlx::query("UPDATE faces SET subject_id = ? WHERE id = ?")
        .bind(subject_id)
        .bind(face_id)
        .execute(pool)
        .await?;
    let row = sqlx::query(
        "SELECT id, name, thumbnail_face_id, type, added_at FROM subjects WHERE id = ?"
    )
    .bind(subject_id)
    .fetch_one(pool)
    .await?;
    Ok(Subject {
        id: row.get("id"),
        name: row.get("name"),
        thumbnail_face_id: row.get("thumbnail_face_id"),
        subject_type: row.get("type"),
        added_at: row.get("added_at"),
    })
}
```

- [ ] **Step 2: Build to verify**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```
Expected: no `^error` lines.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat: add create_subject_for_face db function"
```

---

## Task 3: Tauri commands + registration

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add both commands at the bottom of `commands.rs`, before the closing of the file**

```rust
#[tauri::command]
pub async fn assign_face_to_subject(
    face_id: i64,
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::assign_face_to_subject(&state.pool, face_id, subject_id)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn create_subject_for_face(
    face_id: i64,
    name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::Subject, String> {
    db::create_subject_for_face(&state.pool, face_id, name.as_deref())
        .await
        .map_err(map_err)
}
```

- [ ] **Step 2: Register both commands in `src-tauri/src/lib.rs`**

Find the `invoke_handler` block (ends at `commands::dismiss_merge_suggestion`) and add both new commands:

```rust
        // add these two lines after commands::dismiss_merge_suggestion,
        commands::assign_face_to_subject,
        commands::create_subject_for_face,
```

- [ ] **Step 3: Build to verify**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error" | head -20
```
Expected: no `^error` lines.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: add assign_face_to_subject and create_subject_for_face Tauri commands"
```

---

## Task 4: Fix hlm-command-input + register Plus icon

**Files:**
- Modify: `src/app/libs/ui/command/src/lib/hlm-command-input.ts`
- Modify: `src/app/app.config.ts`

The generated `hlm-command-input.ts` imports `@ng-icons/core` and `@ng-icons/lucide` which are not installed. Replace with `lucide-angular`. Also remove the `HlmInputGroupImports` dependency (not scaffolded).

- [ ] **Step 1: Rewrite `src/app/libs/ui/command/src/lib/hlm-command-input.ts`**

```typescript
import { ChangeDetectionStrategy, Component, input } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { BrnCommandInput } from '@spartan-ng/brain/command';
import { classes } from '@spartan-ng/helm/utils';

@Component({
	selector: 'hlm-command-input',
	imports: [LucideAngularModule, BrnCommandInput],
	changeDetection: ChangeDetectionStrategy.OnPush,
	template: `
		<div class="flex items-center border-b px-3 gap-2">
			<lucide-icon name="search" [size]="16" class="shrink-0 opacity-50" />
			<input
				brnCommandInput
				data-slot="command-input"
				class="flex h-10 w-full bg-transparent py-3 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50"
				[id]="id()"
				[placeholder]="placeholder()"
			/>
		</div>
	`,
})
export class HlmCommandInput {
	public readonly id = input<string | undefined>();
	public readonly placeholder = input<string>('Search...');

	constructor() {
		classes(() => '');
	}
}
```

- [ ] **Step 2: Add `Plus` to the lucide registry in `src/app/app.config.ts`**

The current import line is:
```typescript
import { LucideAngularModule, Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical } from 'lucide-angular';
```

Change it to:
```typescript
import { LucideAngularModule, Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus } from 'lucide-angular';
```

And update the `.pick()` call to include `Plus`:
```typescript
importProvidersFrom(LucideAngularModule.pick({ Search, Info, X, ChevronLeft, ChevronRight, ArrowLeft, Pencil, Star, EllipsisVertical, Plus })),
```

- [ ] **Step 3: Build to verify**

```bash
pnpm ng build 2>&1 | grep -E "^Error|TS[0-9]+" | head -20
```
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/app/libs/ui/command/src/lib/hlm-command-input.ts src/app/app.config.ts
git commit -m "fix: replace @ng-icons with lucide-angular in hlm-command-input, add Plus icon"
```

---

## Task 5: PhotoService — two new methods

**Files:**
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Add `assignFaceToSubject` and `createSubjectForFace` to `PhotoService`**

Add the two methods in the `// ---- Commands ----` section, after `dismissMergeSuggestion`:

```typescript
async assignFaceToSubject(faceId: number, subjectId: number): Promise<void> {
  await invoke('assign_face_to_subject', { faceId, subjectId });
}

async createSubjectForFace(faceId: number, name?: string): Promise<Subject> {
  const subject = await invoke<Subject>('create_subject_for_face', {
    faceId,
    name: name ?? null,
  });
  this.subjects.update(subjects => [...subjects, subject]);
  return subject;
}
```

- [ ] **Step 2: Build to verify**

```bash
pnpm ng build 2>&1 | grep -E "^Error|TS[0-9]+" | head -20
```
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/app/services/photo.service.ts
git commit -m "feat: add assignFaceToSubject and createSubjectForFace to PhotoService"
```

---

## Task 6: FaceAssignPopoverComponent

**Files:**
- Create: `src/app/components/face-assign-popover/face-assign-popover.component.ts`
- Create: `src/app/components/face-assign-popover/face-assign-popover.component.html`
- Create: `src/app/components/face-assign-popover/face-assign-popover.component.css`

- [ ] **Step 1: Create `face-assign-popover.component.ts`**

```typescript
import {
  Component,
  ChangeDetectionStrategy,
  input,
  output,
  inject,
  signal,
  computed,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { LucideAngularModule } from 'lucide-angular';
import { BrnPopoverImports } from '@spartan-ng/brain/popover';
import { BrnCommandImports } from '@spartan-ng/brain/command';
import { HlmPopoverImports } from '@spartan-ng/helm/popover';
import { HlmCommandImports } from '@spartan-ng/helm/command';
import { PhotoService } from '../../services/photo.service';
import { Face, Subject } from '../../models/models';

@Component({
  selector: 'app-face-assign-popover',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CommonModule,
    LucideAngularModule,
    BrnPopoverImports,
    BrnCommandImports,
    HlmPopoverImports,
    HlmCommandImports,
  ],
  templateUrl: './face-assign-popover.component.html',
  styleUrl: './face-assign-popover.component.css',
})
export class FaceAssignPopoverComponent {
  readonly face = input.required<Face>();
  readonly assigned = output<{ face: Face; subject: Subject }>();

  protected photos = inject(PhotoService);
  protected query = signal('');
  protected isOpen = signal(false);

  protected filteredSubjects = computed(() => {
    const q = this.query().toLowerCase().trim();
    if (!q) return this.photos.subjects();
    return this.photos.subjects().filter(s =>
      s.name?.toLowerCase().includes(q) ?? false
    );
  });

  protected faceCropUrl = signal<string | null>(null);

  async open() {
    this.isOpen.set(true);
    this.query.set('');
    const url = await this.photos.getFaceCrop(this.face().id);
    this.faceCropUrl.set(this.photos.thumbnailUrl(url));
  }

  close() {
    this.isOpen.set(false);
    this.query.set('');
  }

  async selectSubject(subject: Subject) {
    await this.photos.assignFaceToSubject(this.face().id, subject.id);
    this.assigned.emit({ face: this.face(), subject });
    this.close();
  }

  async createSubject() {
    const name = this.query().trim() || undefined;
    const subject = await this.photos.createSubjectForFace(this.face().id, name);
    this.assigned.emit({ face: this.face(), subject });
    this.close();
  }

  subjectThumbUrl(subject: Subject): string | null {
    return subject.thumbnail_face_id
      ? this.photos.thumbnailUrl(null) // placeholder — thumbnail loaded lazily
      : null;
  }
}
```

- [ ] **Step 2: Create `face-assign-popover.component.html`**

```html
<div brnPopover>
  <button
    type="button"
    brnPopoverTrigger
    (click)="open()"
    class="text-xs text-muted-foreground underline cursor-pointer hover:text-foreground transition-colors"
  >
    Tap to add
  </button>

  <ng-template brnPopoverContent>
    <div hlmPopoverContent class="w-60 p-0 overflow-hidden">
      <div
        brnCommand
        [search]="query()"
        (searchChange)="query.set($event)"
      >
        <hlm-command-input placeholder="Search subjects..." />
        <hlm-command-list>

          @if (query() === '') {
            <button
              brnCommandItem
              value="__create__"
              (selected)="createSubject()"
              class="flex items-center gap-2 px-2 py-1.5 text-sm w-full text-left cursor-pointer hover:bg-accent rounded-sm"
            >
              <lucide-icon name="plus" [size]="14" class="shrink-0" />
              <span>Create new subject</span>
            </button>
            <div class="h-px bg-border mx-1 my-1"></div>
          }

          @for (subject of filteredSubjects(); track subject.id) {
            <button
              brnCommandItem
              [value]="subject.name ?? ''"
              (selected)="selectSubject(subject)"
              class="flex items-center gap-2 px-2 py-1.5 text-sm w-full text-left cursor-pointer hover:bg-accent rounded-sm"
            >
              <div class="w-6 h-6 rounded-full bg-muted shrink-0 overflow-hidden">
              </div>
              <span
                [class.italic]="!subject.name"
                [class.text-muted-foreground]="!subject.name"
              >
                {{ subject.name ?? 'Unnamed subject' }}
              </span>
            </button>
          }

          @if (query() !== '') {
            <div class="h-px bg-border mx-1 my-1"></div>
            <button
              brnCommandItem
              value="__create__"
              (selected)="createSubject()"
              class="flex items-center gap-2 px-2 py-1.5 text-sm w-full text-left cursor-pointer hover:bg-accent rounded-sm"
            >
              <lucide-icon name="plus" [size]="14" class="shrink-0" />
              <span>Create subject "{{ query() }}"</span>
            </button>
          }

          <hlm-command-empty>No subjects found</hlm-command-empty>
        </hlm-command-list>
      </div>
    </div>
  </ng-template>
</div>
```

- [ ] **Step 3: Create `face-assign-popover.component.css`** (empty — all styling via Tailwind)

```css
```

- [ ] **Step 4: Build to verify**

```bash
pnpm ng build 2>&1 | grep -E "^Error|TS[0-9]+" | head -20
```
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/face-assign-popover/
git commit -m "feat: add FaceAssignPopoverComponent with brn-popover and hlm-command"
```

---

## Task 7: LightboxComponent — wire up "Faces to add"

**Files:**
- Modify: `src/app/components/lightbox/lightbox.component.ts`
- Modify: `src/app/components/lightbox/lightbox.component.html`

- [ ] **Step 1: Update `lightbox.component.ts`**

Add the import for `FaceAssignPopoverComponent` and `Subject` to the existing imports at the top:

```typescript
import { Image, SearchResult, Face, Subject } from '../../models/models';
import { FaceAssignPopoverComponent } from '../face-assign-popover/face-assign-popover.component';
```

Add `FaceAssignPopoverComponent` to the `imports` array in `@Component`:

```typescript
imports: [CommonModule, LucideAngularModule, FaceAssignPopoverComponent],
```

Add these signals and the handler inside the class body, after the existing `faces` and `activeFaceId` signals:

```typescript
showFacesToAdd = signal(false);

protected assignedFaces = computed(() => this.faces().filter(f => f.subject_id !== null));
protected unassignedFaces = computed(() => this.faces().filter(f => f.subject_id === null));

// Local list of subjects shown in the sidebar for this photo (separate from global signal)
protected localSubjects = signal<Subject[]>([]);
```

Update `ngOnChanges` to also populate `localSubjects` from the current subjects signal and reset `showFacesToAdd`:

```typescript
ngOnChanges() {
  if (this.image) {
    const id = 'id' in this.image ? this.image.id : this.image.image_id;
    this.photos.loadFacesForImage(id).then(f => this.faces.set(f));
    if (this.photos.subjects().length === 0) {
      void this.photos.loadSubjects();
    }
    this.localSubjects.set(this.photos.subjects());
    this.showFacesToAdd.set(false);
  } else {
    this.faces.set([]);
    this.localSubjects.set([]);
  }
  this.imgLayout.set(null);
}
```

Add the assignment handler method:

```typescript
onFaceAssigned(event: { face: Face; subject: Subject }) {
  // Remove from faces list by updating subject_id (optimistic)
  this.faces.update(faces =>
    faces.map(f => f.id === event.face.id ? { ...f, subject_id: event.subject.id } : f)
  );
  // Add subject to local list if not already present
  this.localSubjects.update(subjects => {
    if (subjects.find(s => s.id === event.subject.id)) return subjects;
    return [...subjects, event.subject];
  });
  // Auto-collapse if no unassigned faces remain
  if (this.unassignedFaces().length === 0) {
    this.showFacesToAdd.set(false);
  }
}
```

- [ ] **Step 2: Update the People section in `lightbox.component.html`**

Replace the entire `<div class="meta-section">` block (the People section) with:

```html
<div class="meta-section">
  <h3>People</h3>
  @if (assignedFaces().length === 0 && unassignedFaces().length === 0) {
    <div class="value">No people detected</div>
  } @else {
    <div class="people-list">
      @for (face of assignedFaces(); track face.id) {
        <div
          class="person-item clickable"
          [class.active]="activeFaceId() === face.id"
          (mouseenter)="setActiveFace(face.id)"
          (mouseleave)="setActiveFace(null)"
          (click)="navigateToSubject(face.subject_id)">
          <div class="person-avatar">👤</div>
          <div class="person-name">{{ getSubjectName(face.subject_id) }}</div>
        </div>
      }
    </div>

    @if (unassignedFaces().length > 0) {
      @if (!showFacesToAdd()) {
        <button
          type="button"
          class="mt-2 text-xs text-muted-foreground underline cursor-pointer hover:text-foreground transition-colors bg-transparent border-none p-0"
          (click)="showFacesToAdd.set(true)"
        >
          There are some faces to add
        </button>
      } @else {
        <div class="mt-3 pt-3 border-t border-border">
          <div class="flex items-center justify-between mb-2">
            <span class="text-xs uppercase tracking-wide text-muted-foreground">Faces to add</span>
            <button
              type="button"
              class="text-xs text-muted-foreground cursor-pointer hover:text-foreground transition-colors bg-transparent border-none p-0"
              (click)="showFacesToAdd.set(false)"
            >
              ▴ hide
            </button>
          </div>
          <div class="flex flex-col gap-1">
            @for (face of unassignedFaces(); track face.id) {
              <div class="flex items-center gap-2">
                <div class="w-8 h-8 rounded-full bg-muted border border-dashed border-muted-foreground shrink-0 overflow-hidden">
                </div>
                <app-face-assign-popover
                  [face]="face"
                  (assigned)="onFaceAssigned($event)"
                />
              </div>
            }
          </div>
        </div>
      }
    }
  }
</div>
```

- [ ] **Step 3: Build to verify**

```bash
pnpm ng build 2>&1 | grep -E "^Error|TS[0-9]+" | head -20
```
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/lightbox/
git commit -m "feat: add Faces to add section to lightbox sidebar"
```

---

## Task 8: Manual smoke test + final commit

- [ ] **Step 1: Start the app**

```bash
pnpm tauri dev
```

- [ ] **Step 2: Open a photo in the lightbox that has detected faces**

Open the sidebar (ℹ️ button). Verify:
- Assigned faces appear as before in the People section
- If a face has `subject_id = null`, "There are some faces to add" link appears below

- [ ] **Step 3: Expand the Faces to add section**

Click the link. Verify:
- The link disappears
- "Faces to add" section appears with "▴ hide" toggle
- Each unassigned face row shows a "Tap to add" link

- [ ] **Step 4: Test the assignment popover — existing subject**

Click "Tap to add" on an unassigned face. Verify:
- Popover opens with search input and subject list
- "Create new subject" appears at the top when input is empty
- Typing filters subjects by name
- "Create subject `<typed>`" appears at the bottom when typing
- Selecting an existing subject closes the popover, face moves to People section

- [ ] **Step 5: Test the assignment popover — new subject**

Click "Tap to add", leave the input empty, click "Create new subject". Verify:
- New unnamed subject appears in People section
- If this was the last unassigned face, the "Faces to add" section disappears

- [ ] **Step 6: Final commit if any fixups were made during smoke test**

```bash
git add -p
git commit -m "fix: smoke test fixups for face assignment flow"
```
