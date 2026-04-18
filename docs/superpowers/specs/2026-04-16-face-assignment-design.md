# Face Assignment — "Faces to Add" Feature

**Date:** 2026-04-16  
**Branch:** feature/face-labeling  
**Status:** Approved

## Overview

When a photo contains faces that the clustering algorithm couldn't assign to any subject (`subject_id IS NULL`), users currently have no way to assign them. This feature surfaces unassigned faces in the lightbox sidebar and provides an inline assignment flow — similar to Google Photos' "Faces to add" link.

---

## User Flow

1. User opens the lightbox for a photo.
2. The sidebar's **People** section shows all assigned subjects as usual.
3. If the photo has unassigned faces, a small link — **"There are some faces to add"** — appears below the assigned subjects list.
4. Clicking the link expands a **"Faces to add"** subsection showing each unassigned face crop with a **"Tap to add"** affordance. The link disappears while the section is open.
5. Clicking **"▴ hide"** in the subsection header collapses it and restores the link.
6. Tapping an unassigned face opens an **inline assignment popover** anchored to that row.
7. Inside the popover:
   - A fuzzy search input filters existing subjects by name.
   - When the input is empty, **"Create new subject"** appears at the top of the list.
   - When the input has text, existing subjects are filtered and **"Create subject `<typed-name>`"** appears at the bottom.
   - Unnamed subjects are shown in italics.
8. Selecting an existing subject calls `assign_face_to_subject`. Selecting "Create" calls `create_subject_for_face`.
9. On success, the face is removed from "Faces to add" and the subject appears in the People section — both updates are optimistic (no re-fetch).
10. When the last unassigned face is assigned, the "Faces to add" subsection and the link both disappear.

---

## Architecture

### Backend — 2 new Tauri commands (`commands.rs`)

**`assign_face_to_subject(face_id: i64, subject_id: i64) -> Result<()>`**
- SQL: `UPDATE faces SET subject_id = ? WHERE id = ?`

**`create_subject_for_face(face_id: i64, name: Option<String>) -> Result<Subject>`**
- SQL: `INSERT INTO subjects (name, type, added_at) VALUES (?, 'person', ?)`, then `UPDATE faces SET subject_id = <new_id> WHERE id = ?`
- Returns the newly created `Subject` row.

No new query is needed for loading unassigned faces — `list_faces_for_image` already returns all faces including those with `subject_id: null`. The frontend filters client-side.

### Frontend — Angular / Tauri

#### `PhotoService` — 2 new methods

```typescript
assignFaceToSubject(faceId: number, subjectId: number): Promise<void>
createSubjectForFace(faceId: number, name?: string): Promise<Subject>
```

Both are thin wrappers over `invoke()`.

After `createSubjectForFace` resolves, the new subject is pushed into the `subjects()` signal so it is immediately available across the app.

#### `LightboxComponent` — changes

- Add computed: `unassignedFaces = faces().filter(f => f.subject_id === null)`
- Add local state: `showFacesToAdd = false`
- Render "There are some faces to add" link when `unassignedFaces().length > 0 && !showFacesToAdd`
- Render "Faces to add" subsection when `showFacesToAdd`
- On assignment event from popover: remove the face from `unassignedFaces` and add the subject to the local subjects list (optimistic)

#### `FaceAssignPopoverComponent` — new standalone component

**Location:** `src/app/components/face-assign-popover/`

**Inputs:**
- `faceId: number`

**Outputs:**
- `assigned: EventEmitter<{ face: Face; subject: Subject }>`

**Spartan components used:**
- `brn-popover` + `hlm-popover-content` — popover container anchored to the trigger
- `hlm-command` + `hlm-command-input` + `hlm-command-list` + `hlm-command-item` — searchable list

**Lucide icons used:**
- `lucide-search` — inside the command input
- `lucide-plus` — on the "Create subject" item
- Subject thumbnail (face crop) — on each existing subject item

**Behavior:**
- Subject list sourced from `photoService.subjects()` signal — no extra fetch
- Fuzzy filter: client-side `name.toLowerCase().includes(query.toLowerCase())` (simple contains match is sufficient; upgrade to fuse.js only if needed)
- When `query === ''`: "Create new subject" item renders at the top (creates subject with `name = null`)
- When `query !== ''`: matching subjects shown, "Create subject `<query>`" item at the bottom
- Popover closes after any selection
- Emits `assigned` with the face and the subject (existing or newly created)

---

## Spartan Components to Generate

Run before implementation:

```bash
npx spartan add popover
npx spartan add command
```

---

## Data Flow Diagram

```
User taps "Tap to add"
        │
        ▼
FaceAssignPopoverComponent opens
        │
   ┌────┴────────────────────┐
   │ select existing subject  │ select "Create"
   ▼                          ▼
assignFaceToSubject()     createSubjectForFace()
        │                          │
        └──────────┬───────────────┘
                   ▼
          emit (assigned) event
                   │
                   ▼
        LightboxComponent
        - remove face from unassignedFaces
        - add subject to People list
        - if unassignedFaces empty → hide section + link
```

---

## Out of Scope

- Bulk assignment (selecting multiple unassigned faces at once)
- Unassigned faces view outside the lightbox (e.g. People view banner)
- Re-running clustering from this flow
- Keyboard navigation beyond what `hlm-command` provides by default
