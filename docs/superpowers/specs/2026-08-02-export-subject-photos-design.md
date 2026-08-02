# Export subject photos (Copy all) — design

Date: 2026-08-02
Status: implemented

## Problem

Users can browse every photo of a person (e.g. Cass) on the subject detail page,
but cannot get those originals onto an arbitrary folder on the native filesystem.
The motivating workflow is: export Cass’s full-resolution photos → open the
folder → upload to Google Photos (outside Nebula) → share.

Nebula stays local-first: no Google Photos API, OAuth, or cloud upload in this
feature.

## Goals (v1)

- One-shot **Copy all…** from a subject’s detail page.
- Copy **full-resolution original files** (not Nebula previews/thumbnails).
- Destination is a user-chosen directory via the native folder dialog.
- Flat export: all files land directly in that directory (no mirrored tree).
- After a successful copy, **open the destination** in the OS file manager.
- Progress feedback while copying large sets.
- Survive name collisions and missing sources without losing the rest of the batch.

## Non-goals (v1)

- Multi-select / cherry-pick subset of photos.
- Respecting subject-detail grid filters (date range, etc.) — export is always
  the **entire person**.
- Direct Google Photos (or any cloud) upload.
- Move (only copy).
- Adding the export folder as a Nebula library folder.
- Generic “export any selection from gallery” (can reuse the copy helper later).

## Product decisions

| Topic | Decision |
|-------|----------|
| Workflow depth | Copy + open destination (user handles Google Photos) |
| Selection model | One-shot Copy all (no multi-select UI) |
| Export set | Entire subject; ignore UI filters |
| File kind | Originals only |
| Layout | Flat folder |
| Name collisions | Parent-folder prefix (`Vacation_IMG_0001.jpg`) |
| Architecture | Subject-scoped command + small FS copy helper |

## User flow

1. Open `/subject/:id` (e.g. Cass).
2. Click **Copy all…** in the page header (shown only when `photo_count > 0`).
3. Native directory picker (`@tauri-apps/plugin-dialog`, `directory: true`).
   Cancel → no backend call.
4. Backend copies every original belonging to the subject into the chosen dir.
   UI shows progress: “Copying N of M…” and disables the button.
5. On success: open destination with `@tauri-apps/plugin-opener`, show an
   inline status line (`Copied 128 photos`, plus skip counts if any).
6. On hard failure: show inline error status, re-enable button, do not open
   the folder.

## Backend

### Placement

Vertical-slice rules apply:

- **`people` slice** owns the command and subject path resolution (it already
  has `list_images_for_subject` / `get_subject_photos`).
- **Copy/naming FS logic** lives in `library/export.rs` so it stays reusable and
  does not bloat `people` with filesystem policy. `people` calls into the public
  helper API; it does not open files itself beyond orchestration.

### Command

```rust
// people/commands.rs
#[tauri::command]
pub async fn export_subject_photos(
    subject_id: i64,
    dest_dir: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<ExportSubjectResult, String>
```

Registered at its definition site in `app/mod.rs` (no re-export-only registration).

### Result model

```rust
// people/models.rs (or shared export models if preferred)
pub struct ExportSubjectResult {
    pub dest_dir: String,
    pub copied: u32,
    pub skipped_missing: u32,
    pub skipped_errors: u32,
}
```

Serde-serializable for the frontend.

### Algorithm

1. Validate subject exists (same style as other subject commands).
2. `list_images_for_subject` → ordered list of absolute source `path`s
   (`deleted_at IS NULL` only; same set as subject photos today).
3. Validate `dest_dir` is an existing directory and is writable (probe create or
   metadata + write check). Hard-fail otherwise.
4. For each source path (index `i` of `total`):
   - If source missing or not a file → `skipped_missing++`, continue.
   - Compute destination file name (see Naming).
   - `tokio::fs::copy(src, dest)`. On error → `skipped_errors++`, continue.
   - Else `copied++`.
   - Emit progress event (below).
5. Return `ExportSubjectResult`. Opening the folder is **frontend-only** so the
   backend stays headless-testable.

### Naming

```
source:  /media/Photos/2024/Vacation/IMG_0001.jpg
parent:  Vacation          // immediate parent directory name
dest:    {dest_dir}/Vacation_IMG_0001.jpg
```

Rules:

- `parent` = file’s parent directory’s file name (last component).
- If parent is empty, `.`, or effectively root → use **basename only**.
- Sanitize only what is required for the platform (strip path separators from
  parent if any slip in). Do not aggressively rewrite user filenames.
- If the computed dest path **already exists** (re-export or residual collision
  after prefix): append ` (2)`, ` (3)`, … before the extension
  (`Vacation_IMG_0001 (2).jpg`). Never overwrite.

### Progress event

```text
event: "export_subject_progress"
payload: { current: u32, total: u32 }  // current = 1-based completed attempts
```

Emitted after each file attempt (copy, skip, or error) so the bar advances
even when skipping. Frontend ignores late events after the invoke resolves.

### Errors (hard)

- Unknown `subject_id`
- `dest_dir` missing, not a directory, or not writable
- Empty/invalid `dest_dir` string

These return `Err(String)` and abort before/without completing the batch.
If a hard failure occurs mid-batch (rare; e.g. dest becomes unwritable), stop
and return `Err`; partial files already written may remain on disk. The UI shows
the error string only (no folder open).

### Errors (soft)

- Missing/deleted source file → skip, count `skipped_missing`
- Per-file I/O error → skip, count `skipped_errors`

Empty subject (`photo_count == 0`): button hidden in the UI. If the command is
invoked anyway, return `Ok` with `copied: 0` (and zeros for skip counts).

## Frontend

### Entry point

`SubjectDetailComponent` header: **Copy all…** button (Lucide `copy` icon +
label). Adjacent to the existing ⋮ menu. Hidden when `detail()?.photo_count`
is 0 or null. Register `Copy` in `app-icons.ts` (template names must be listed
there or lucide-angular blanks the view).

There is no global toast system today. Use a local `exportStatus` signal on
the subject page: success and error messages render as a short line under the
header (or next to the progress bar) and clear on the next export attempt or
after a few seconds.

### Orchestration

```text
onCopyAll:
  dest = await dialog.open({ directory: true, multiple: false })
  if !dest: return
  exporting = true; progress = { current: 0, total: 0 }; exportStatus = null
  listen('export_subject_progress', update progress)
  try:
    result = await photos.exportSubjectPhotos(subjectId, dest)
    exportStatus = success message with copied / skips
    await open(result.dest_dir)   // @tauri-apps/plugin-opener
  catch e:
    exportStatus = error message
  finally:
    unlisten; exporting = false
```

### PhotoService

- `exportSubjectPhotos(subjectId: number, destDir: string): Promise<ExportSubjectResult>`
- Models: `ExportSubjectResult`, `ExportSubjectProgress` in `models.ts`.

### Progress UI

Slim bar below the header while `exporting` is true. Label:
`Copying {current} of {total}…`. Button disabled while exporting. No cancel
mid-copy in v1 (YAGNI; can add later).

### Dependencies (already present)

- `@tauri-apps/plugin-dialog` — folder picker (used by sidebar add-folder)
- `@tauri-apps/plugin-opener` — open destination (registered in app + capabilities;
  first product use)

## Testing

### Rust

- Naming unit tests: parent prefix, root/empty parent, extension preserved,
  collision → `(2)` / `(3)`.
- Integration with temp dirs: N files copy; missing source skipped; unwritable
  dest errors; unknown subject errors; progress monotonic.
- Prefer pure helper tests without full Tauri runtime where possible; command
  test can use existing app/db test fixtures if available.

### Frontend

- Subject detail: button visibility by `photo_count`.
- Orchestration: cancel dialog does not invoke; success path calls open;
  error path does not open (mock dialog/invoke/opener).

## Future extensions (out of scope)

- Multi-select export from subject/gallery reusing the FS helper with
  `image_ids[]`.
- Optional “respect filters” mode once multi-select exists.
- Post-export “Reveal in folder” without re-copy.
- Manifest CSV (source path → dest name) for audit.

## Implementation sketch (order)

1. Library copy/naming helper + unit tests.
2. `export_subject_photos` command + progress emit + registration.
3. Frontend models + PhotoService method.
4. Subject detail button, progress, dialog, opener, toasts.
5. Component specs for visibility / orchestration happy path.
