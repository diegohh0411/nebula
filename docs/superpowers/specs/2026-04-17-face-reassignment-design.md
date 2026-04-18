# Face Subject Reassignment in Lightbox — Design Spec

**Date:** 2026-04-17

## Problem

Nebula's face clustering sometimes assigns faces to the wrong subject (e.g., confusing siblings with similar faces). Users need a way to correct these errors while viewing a photo in the lightbox.

## Research Summary

- **Google Photos**: Per-photo face label editing in info panel — remove wrong label, add correct one
- **Immich**: Simple `reassignFace()` — just moves the face, no learning from corrections
- **No production photo manager learns from corrections**. The academic approach (constrained clustering with must-link/cannot-link constraints) requires fundamental algorithm changes
- **No Rust crate** supports constrained/semi-supervised clustering out of the box (checked `hdbscan`, `linfa-clustering`, `scirs2-cluster`, `petal-clustering`)

## Scope

1. Lightbox face reassignment UI (extend existing `FaceAssignPopoverComponent`)
2. Protect manual assignments during recluster (add `is_manual` flag)
3. Store corrections data for future learning

**Deferred**: Subject detail page corrections, learning from corrections

## Approach

Extend the existing `FaceAssignPopoverComponent` to handle both assigned and unassigned faces. When a face already has a subject, the popover shows the current subject and options to reassign or remove.

---

## Backend Changes

### Schema

**Add `is_manual` column to `faces` table** (new migration):
```sql
ALTER TABLE faces ADD COLUMN is_manual INTEGER NOT NULL DEFAULT 0;
```

**New `face_corrections` table**:
```sql
CREATE TABLE IF NOT EXISTS face_corrections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    face_id INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    old_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    new_subject_id INTEGER REFERENCES subjects(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_corrections_face ON face_corrections(face_id);
```

### New DB functions

- `unassign_face(pool, face_id)` — Sets `subject_id = NULL, is_manual = 1`
- `reassign_face(pool, face_id, new_subject_id)` — Updates `subject_id`, sets `is_manual = 1`, records correction in `face_corrections`
- `record_face_correction(pool, face_id, old_subject_id, new_subject_id)` — Inserts row into `face_corrections`

### Modified DB functions

- `assign_face_to_subject` — Also set `is_manual = 1`
- `create_subject_for_face` — Also set `is_manual = 1`

### New Tauri command

- `unassign_face(face_id)` — Calls `db::unassign_face`

### Modified Tauri command

- `assign_face_to_subject` — Also calls `record_face_correction` if face had a previous subject

### Protect manual assignments during recluster

In `recluster_all` (`clustering.rs`):
1. Before clustering, load `is_manual` status for each face
2. After HDBSCAN produces cluster assignments, for each face:
   - If `is_manual = 1` and `subject_id IS NOT NULL` → keep the manual subject assignment, ignore cluster result
   - If `is_manual = 1` and `subject_id IS NULL` → keep unassigned (don't let clustering auto-assign)
   - If `is_manual = 0` → use cluster result as before

---

## Frontend Changes

### FaceAssignPopover extension

The existing `FaceAssignPopoverComponent` gets two modes:

**Assign mode** (current behavior, for unassigned faces):
- Trigger: "Tap to add" text
- Popover: Search/create subject command palette

**Reassign mode** (new, for assigned faces):
- Trigger: Subject name + pencil icon
- Popover contains:
  - Header: "Currently: **{subject name}**"
  - "Remove from subject" button (calls `unassign_face`)
  - Divider
  - Subject search/create (same command palette)

New inputs/outputs:
- `currentSubjectId: input<number | null>(null)` — When set, enters reassign mode
- `reassigned: output<{ face: Face; newSubject: Subject | null }>()` — Emitted on reassign or remove

### Lightbox face overlay click behavior

Currently: click on face overlay → `navigateToSubject(subject_id)` (closes lightbox, navigates to subject page)

New: click on assigned face overlay → opens the reassign popover on that face. The popover provides a "View" link to navigate to the subject.

### Sidebar People section

Each assigned face in the sidebar gets a small pencil icon next to the name:
- Clicking the name → navigates to subject (unchanged)
- Clicking the pencil icon → opens the FaceAssignPopover in reassign mode

### Lightbox state management

New method on `LightboxComponent`:
- `onFaceReassigned(event: { face: Face; newSubject: Subject | null })` — Updates the face's `subject_id` in the local `faces` signal. If `newSubject` is null (removed), face moves to `unassignedFaces`. If `newSubject` is a different subject, face stays in `assignedFaces` with updated subject.

---

## Edge Cases

1. **Face crop thumbnail**: If the removed/reassigned face was the subject's thumbnail face (`thumbnail_face_id`), the subject needs a new thumbnail. After any correction, call `auto_assign_missing_thumbnails` for affected subjects.

2. **Last face in subject**: If removing/reassigning the last face from a subject, that subject becomes orphaned. The existing `delete_subjects_with_no_faces` handles this during recluster. For immediate cleanup after a correction, the backend checks and deletes orphaned subjects.

3. **Correction to same subject**: If user picks the same subject the face is already assigned to, no-op.

4. **Concurrent recluster**: If recluster runs while user is correcting, the `is_manual` flag protects the user's corrections.

---

## Files to Modify

**Backend (Rust)**:
- `src-tauri/src/db.rs` — Schema migration, new functions, modified functions
- `src-tauri/src/commands.rs` — New `unassign_face` command, modify `assign_face_to_subject`
- `src-tauri/src/clustering.rs` — Protect manual assignments in `recluster_all`
- `src-tauri/src/models.rs` — Add `is_manual` to Face model

**Frontend (Angular)**:
- `src/app/components/face-assign-popover/face-assign-popover.component.ts` — Add reassign mode
- `src/app/components/face-assign-popover/face-assign-popover.component.html` — Reassign mode template
- `src/app/components/lightbox/lightbox.component.ts` — New handlers, face overlay interaction change
- `src/app/components/lightbox/lightbox.component.html` — Face overlay popover integration, sidebar pencil icons
- `src/app/services/photo.service.ts` — Add `unassignFace` method
