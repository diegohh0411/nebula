# Export Subject Photos (Copy all) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users one-shot copy every original photo of a subject (e.g. Cass) into a chosen folder on disk, then open that folder for external Google Photos upload.

**Architecture:** Pure FS helper in `library/export.rs` (naming + copy batch). `people` owns `export_subject_photos` command: validate subject, load paths via existing `list_images_for_subject`, call helper, emit `export_subject_progress`. Angular subject-detail header drives dialog → invoke → progress UI → `openPath` via plugin-opener.

**Tech Stack:** Rust/Tauri 2, sqlx, tokio::fs, Angular 20 signals, `@tauri-apps/plugin-dialog`, `@tauri-apps/plugin-opener`, Vitest.

**Spec:** `docs/superpowers/specs/2026-08-02-export-subject-photos-design.md`

## File map

| File | Responsibility |
|------|----------------|
| Create `src-tauri/src/library/export.rs` | Dest naming, unique path, `copy_paths_to_dir`, unit tests |
| Modify `src-tauri/src/library/mod.rs` | `pub mod export;` |
| Modify `src-tauri/src/models/entities.rs` | `ExportSubjectResult`, `ExportSubjectProgress` |
| Modify `src-tauri/src/people/commands.rs` | `export_subject_photos` command |
| Modify `src-tauri/src/app/mod.rs` | Register command in `generate_handler!` |
| Modify `src/app/models/models.ts` | TS interfaces |
| Modify `src/app/services/photo.service.ts` | `exportSubjectPhotos` |
| Modify `src/app/app-icons.ts` | Register Lucide `Copy` |
| Modify `src/app/components/subject-detail/*` | Button, progress, status, orchestration |
| Modify `src/app/components/subject-detail/subject-detail.component.spec.ts` | Visibility + orchestration tests |
| Possibly modify `src-tauri/capabilities/default.json` | Broaden opener path scope if open fails |

## Global constraints

- Originals only (image `path`), not previews/thumbnails.
- Entire subject set; ignore client-side filters.
- Flat dest; parent-folder prefix; never overwrite (use ` (N)` suffix).
- Soft-skip missing sources; hard-fail unknown subject / bad dest.
- Open folder is **frontend-only** after successful invoke.
- No global toast system — inline status on subject detail.
- Cargo tests from `src-tauri`: `cargo test --lib export::` or full `cargo test --lib`.
- Frontend: `pnpm exec vitest run <path>`.
- Register command at definition site in `app/mod.rs` (not via re-export alone).
- Tauri invoke args are camelCase from TS (`subjectId`, `destDir`); Rust params use snake_case.

---

### Task 1: Naming + unique dest path (`library/export.rs`)

**Files:**
- Create: `src-tauri/src/library/export.rs`
- Modify: `src-tauri/src/library/mod.rs`

- [ ] **Step 1: Add module stub and failing tests**

Add to `src-tauri/src/library/mod.rs`:

```rust
pub mod export;
```

Create `src-tauri/src/library/export.rs`:

```rust
//! Copy library originals into a user-chosen export directory.

use std::path::{Path, PathBuf};

/// Build the preferred destination file name: `{parent}_{basename}`.
/// Falls back to basename when parent is empty, `.`, or a root component.
pub fn preferred_dest_name(source: &Path) -> String {
    todo!("preferred_dest_name")
}

/// If `dest_dir/file_name` is free, return it; else insert ` (2)`, ` (3)`, …
/// before the extension until free. Never returns an existing path.
pub fn unique_dest_path(dest_dir: &Path, file_name: &str) -> PathBuf {
    todo!("unique_dest_path")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn preferred_name_uses_parent_prefix() {
        let p = PathBuf::from("/media/Photos/2024/Vacation/IMG_0001.jpg");
        assert_eq!(preferred_dest_name(&p), "Vacation_IMG_0001.jpg");
    }

    #[test]
    fn preferred_name_phone_prefix() {
        let p = PathBuf::from("/media/Phone/DSC_99.JPG");
        assert_eq!(preferred_dest_name(&p), "Phone_DSC_99.JPG");
    }

    #[test]
    fn preferred_name_root_falls_back_to_basename() {
        // Unix root parent is "/" — file_name of parent is None / empty
        let p = PathBuf::from("/IMG_0001.jpg");
        assert_eq!(preferred_dest_name(&p), "IMG_0001.jpg");
    }

    #[test]
    fn preferred_name_strips_separators_from_parent() {
        // Defensive: if a weird path component slipped in, no path seps in output name
        let p = PathBuf::from("/tmp/normal/photo.jpg");
        let name = preferred_dest_name(&p);
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
        assert_eq!(name, "normal_photo.jpg");
    }

    #[test]
    fn unique_path_no_collision() {
        let dir = tempfile::tempdir().unwrap();
        let out = unique_dest_path(dir.path(), "Vacation_IMG.jpg");
        assert_eq!(out, dir.path().join("Vacation_IMG.jpg"));
    }

    #[test]
    fn unique_path_suffixes_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Vacation_IMG.jpg"), b"a").unwrap();
        fs::write(dir.path().join("Vacation_IMG (2).jpg"), b"b").unwrap();
        let out = unique_dest_path(dir.path(), "Vacation_IMG.jpg");
        assert_eq!(out, dir.path().join("Vacation_IMG (3).jpg"));
    }

    #[test]
    fn unique_path_preserves_multi_dot_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Vacation_photo.raw.jpg"), b"a").unwrap();
        let out = unique_dest_path(dir.path(), "Vacation_photo.raw.jpg");
        assert_eq!(out, dir.path().join("Vacation_photo.raw (2).jpg"));
    }
}
```

If `tempfile` is not already a dev-dependency, add it under `[dev-dependencies]` in `src-tauri/Cargo.toml`:

```toml
tempfile = "3"
```

(Check first with `rg 'tempfile' src-tauri/Cargo.toml` — add only if missing.)

- [ ] **Step 2: Run tests — expect fail**

```bash
cd src-tauri && cargo test --lib library::export::tests -- --nocapture
```

Expected: FAIL (todo! panic or link errors).

- [ ] **Step 3: Implement naming helpers**

Replace todos in `export.rs` with:

```rust
use std::path::{Path, PathBuf};

/// Build the preferred destination file name: `{parent}_{basename}`.
/// Falls back to basename when parent is empty, `.`, or a filesystem root.
pub fn preferred_dest_name(source: &Path) -> String {
    let basename = source
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("export.bin")
        .to_string();

    let parent_name = source
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != ".");

    match parent_name {
        Some(parent) => {
            // Strip any accidental path separators from parent for safety.
            let safe_parent: String = parent
                .chars()
                .filter(|c| *c != '/' && *c != '\\')
                .collect();
            if safe_parent.is_empty() {
                basename
            } else {
                format!("{safe_parent}_{basename}")
            }
        }
        None => basename,
    }
}

/// If `dest_dir/file_name` is free, return it; else insert ` (2)`, ` (3)`, …
/// before the extension until free. Never returns an existing path.
pub fn unique_dest_path(dest_dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dest_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let ext = path.extension().and_then(|s| s.to_str());

    let mut n = 2u32;
    loop {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dest_dir.join(&name);
        if !candidate.exists() {
            return candidate;
        }
        n = n.saturating_add(1);
        if n > 100_000 {
            // Pathological; still return a unique-ish name
            return dest_dir.join(format!("{stem} ({n}).tmp"));
        }
    }
}
```

- [ ] **Step 4: Run tests — expect pass**

```bash
cd src-tauri && cargo test --lib library::export::tests -- --nocapture
```

Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/library/export.rs src-tauri/src/library/mod.rs src-tauri/Cargo.toml
git commit -m "feat(library): export naming helpers for subject photo copy"
```

---

### Task 2: Batch copy helper + dest validation

**Files:**
- Modify: `src-tauri/src/library/export.rs`

- [ ] **Step 1: Write failing tests for copy batch**

Append to `tests` module in `export.rs`:

```rust
    use super::{copy_paths_to_dir, validate_dest_dir, CopyFilesResult};

    #[tokio::test]
    async fn copy_paths_copies_with_parent_prefix() {
        let src_root = tempfile::tempdir().unwrap();
        let vacation = src_root.path().join("Vacation");
        fs::create_dir_all(&vacation).unwrap();
        let a = vacation.join("IMG_01.jpg");
        let b = vacation.join("IMG_02.jpg");
        fs::write(&a, b"one").unwrap();
        fs::write(&b, b"two").unwrap();

        let dest = tempfile::tempdir().unwrap();
        let mut progress = Vec::new();
        let result = copy_paths_to_dir(
            &[a.clone(), b.clone()],
            dest.path(),
            |cur, total| progress.push((cur, total)),
        )
        .await
        .unwrap();

        assert_eq!(
            result,
            CopyFilesResult {
                copied: 2,
                skipped_missing: 0,
                skipped_errors: 0,
            }
        );
        assert!(dest.path().join("Vacation_IMG_01.jpg").exists());
        assert!(dest.path().join("Vacation_IMG_02.jpg").exists());
        assert_eq!(progress, vec![(1, 2), (2, 2)]);
    }

    #[tokio::test]
    async fn copy_paths_skips_missing_source() {
        let src_root = tempfile::tempdir().unwrap();
        let phone = src_root.path().join("Phone");
        fs::create_dir_all(&phone).unwrap();
        let good = phone.join("ok.jpg");
        fs::write(&good, b"ok").unwrap();
        let missing = phone.join("gone.jpg");

        let dest = tempfile::tempdir().unwrap();
        let result = copy_paths_to_dir(&[good, missing], dest.path(), |_, _| {})
            .await
            .unwrap();

        assert_eq!(result.copied, 1);
        assert_eq!(result.skipped_missing, 1);
        assert_eq!(result.skipped_errors, 0);
        assert!(dest.path().join("Phone_ok.jpg").exists());
    }

    #[tokio::test]
    async fn copy_paths_never_overwrites_existing_dest() {
        let src_root = tempfile::tempdir().unwrap();
        let folder = src_root.path().join("Trip");
        fs::create_dir_all(&folder).unwrap();
        let src = folder.join("shot.jpg");
        fs::write(&src, b"new").unwrap();

        let dest = tempfile::tempdir().unwrap();
        fs::write(dest.path().join("Trip_shot.jpg"), b"old").unwrap();

        let result = copy_paths_to_dir(&[src], dest.path(), |_, _| {})
            .await
            .unwrap();
        assert_eq!(result.copied, 1);
        assert_eq!(fs::read(dest.path().join("Trip_shot.jpg")).unwrap(), b"old");
        assert_eq!(
            fs::read(dest.path().join("Trip_shot (2).jpg")).unwrap(),
            b"new"
        );
    }

    #[test]
    fn validate_dest_rejects_missing_path() {
        let p = PathBuf::from("/no/such/export/dir/hopefully");
        let err = validate_dest_dir(&p).unwrap_err();
        assert!(err.to_lowercase().contains("not") || err.to_lowercase().contains("exist"));
    }

    #[test]
    fn validate_dest_accepts_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        validate_dest_dir(dir.path()).unwrap();
    }

    #[test]
    fn validate_dest_rejects_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        fs::write(&file, b"x").unwrap();
        assert!(validate_dest_dir(&file).is_err());
    }
```

- [ ] **Step 2: Run tests — expect fail**

```bash
cd src-tauri && cargo test --lib library::export:: -- --nocapture
```

Expected: FAIL — `copy_paths_to_dir` / `CopyFilesResult` / `validate_dest_dir` missing.

- [ ] **Step 3: Implement copy + validation**

Add to `export.rs` (above tests):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFilesResult {
    pub copied: u32,
    pub skipped_missing: u32,
    pub skipped_errors: u32,
}

/// Ensure `dest_dir` exists, is a directory, and is writable.
pub fn validate_dest_dir(dest_dir: &Path) -> Result<(), String> {
    if dest_dir.as_os_str().is_empty() {
        return Err("Destination path is empty".into());
    }
    let meta = std::fs::metadata(dest_dir).map_err(|e| {
        format!("Destination does not exist or is inaccessible: {e}")
    })?;
    if !meta.is_dir() {
        return Err("Destination is not a directory".into());
    }
    // Writable probe: create + remove a temp file.
    let probe = dest_dir.join(".nebula_export_write_probe");
    std::fs::write(&probe, b"").map_err(|e| {
        format!("Destination is not writable: {e}")
    })?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Copy each source path into `dest_dir` using parent-prefix naming.
/// Soft-skips missing sources and per-file I/O errors.
/// Calls `on_progress(current, total)` after each attempt (`current` is 1-based).
pub async fn copy_paths_to_dir<F>(
    sources: &[PathBuf],
    dest_dir: &Path,
    mut on_progress: F,
) -> Result<CopyFilesResult, String>
where
    F: FnMut(u32, u32),
{
    validate_dest_dir(dest_dir)?;

    let total = sources.len() as u32;
    let mut copied = 0u32;
    let mut skipped_missing = 0u32;
    let mut skipped_errors = 0u32;

    for (idx, src) in sources.iter().enumerate() {
        let current = (idx as u32).saturating_add(1);

        let meta = match tokio::fs::metadata(src).await {
            Ok(m) if m.is_file() => m,
            Ok(_) => {
                skipped_missing = skipped_missing.saturating_add(1);
                on_progress(current, total);
                continue;
            }
            Err(_) => {
                skipped_missing = skipped_missing.saturating_add(1);
                on_progress(current, total);
                continue;
            }
        };
        let _ = meta;

        let name = preferred_dest_name(src);
        let dest = unique_dest_path(dest_dir, &name);

        match tokio::fs::copy(src, &dest).await {
            Ok(_) => copied = copied.saturating_add(1),
            Err(_) => skipped_errors = skipped_errors.saturating_add(1),
        }
        on_progress(current, total);
    }

    Ok(CopyFilesResult {
        copied,
        skipped_missing,
        skipped_errors,
    })
}
```

- [ ] **Step 4: Run tests — expect pass**

```bash
cd src-tauri && cargo test --lib library::export:: -- --nocapture
```

Expected: all PASS. Fix clippy if needed (`cargo clippy -p nebula --lib -- -D warnings` only if CI requires; prefer clean code).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/library/export.rs
git commit -m "feat(library): copy_paths_to_dir with soft skips and dest validation"
```

---

### Task 3: `export_subject_photos` command + models + registration

**Files:**
- Modify: `src-tauri/src/models/entities.rs`
- Modify: `src-tauri/src/people/commands.rs`
- Modify: `src-tauri/src/app/mod.rs`

- [ ] **Step 1: Add result / progress structs**

In `src-tauri/src/models/entities.rs`, near other subject-related structs (`SubjectDetail`):

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExportSubjectResult {
    pub dest_dir: String,
    pub copied: u32,
    pub skipped_missing: u32,
    pub skipped_errors: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExportSubjectProgress {
    pub current: u32,
    pub total: u32,
}
```

Ensure `Serialize`/`Deserialize` imports already exist at top of file (they do via other structs).

- [ ] **Step 2: Implement command**

Append to `src-tauri/src/people/commands.rs`:

```rust
#[tauri::command]
pub async fn export_subject_photos(
    subject_id: i64,
    dest_dir: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::ExportSubjectResult, String> {
    use crate::library::export::{copy_paths_to_dir, CopyFilesResult};
    use crate::models::ExportSubjectProgress;
    use std::path::PathBuf;
    use tauri::Emitter;

    // Validate subject exists (same message as get_subject_detail).
    let _detail = repo::get_subject_detail_with_counts(&state.pool, subject_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| "Subject not found".to_string())?;

    let images = repo::list_images_for_subject(&state.pool, subject_id)
        .await
        .map_err(map_err)?;

    let sources: Vec<PathBuf> = images.into_iter().map(|img| PathBuf::from(img.path)).collect();
    let dest = PathBuf::from(&dest_dir);

    let CopyFilesResult {
        copied,
        skipped_missing,
        skipped_errors,
    } = copy_paths_to_dir(&sources, &dest, |current, total| {
        let _ = app.emit(
            "export_subject_progress",
            ExportSubjectProgress { current, total },
        );
    })
    .await?;

    Ok(crate::models::ExportSubjectResult {
        dest_dir,
        copied,
        skipped_missing,
        skipped_errors,
    })
}
```

Check how other commands import `Emitter` — pipeline uses `app.emit`. If `AppHandle` needs `use tauri::Emitter`, keep it. If the project already has a prelude, match local style.

- [ ] **Step 3: Register command**

In `src-tauri/src/app/mod.rs`, inside `generate_handler![...]`, after `get_subject_photos_with_faces` (or near other people commands):

```rust
crate::people::commands::export_subject_photos,
```

- [ ] **Step 4: Compile check**

```bash
cd src-tauri && cargo test --lib library::export:: -- --nocapture && cargo check
```

Expected: success. Fix any unused import / Emitter trait issues.

Optional integration-style test (if easy with existing pool helpers): subject not found returns error string. Only add if a small test fixture pattern already exists in `people` tests; otherwise skip — unit coverage on the helper is enough for v1.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/models/entities.rs src-tauri/src/people/commands.rs src-tauri/src/app/mod.rs
git commit -m "feat(people): export_subject_photos command with progress events"
```

---

### Task 4: Frontend models + PhotoService

**Files:**
- Modify: `src/app/models/models.ts`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Add TS interfaces**

In `src/app/models/models.ts`:

```ts
export interface ExportSubjectResult {
  dest_dir: string;
  copied: number;
  skipped_missing: number;
  skipped_errors: number;
}

export interface ExportSubjectProgress {
  current: number;
  total: number;
}
```

- [ ] **Step 2: Add PhotoService method**

Import the new types in `photo.service.ts` and add near other subject methods:

```ts
async exportSubjectPhotos(
  subjectId: number,
  destDir: string,
): Promise<ExportSubjectResult> {
  return await invoke<ExportSubjectResult>('export_subject_photos', {
    subjectId,
    destDir,
  });
}
```

- [ ] **Step 3: Commit**

```bash
git add src/app/models/models.ts src/app/services/photo.service.ts
git commit -m "feat(frontend): exportSubjectPhotos service + models"
```

---

### Task 5: Subject detail UI (Copy all… + progress + open)

**Files:**
- Modify: `src/app/app-icons.ts`
- Modify: `src/app/components/subject-detail/subject-detail.component.ts`
- Modify: `src/app/components/subject-detail/subject-detail.component.html`
- Possibly: `src/app/components/subject-detail/subject-detail.component.css` (only if needed)

- [ ] **Step 1: Register Lucide Copy icon**

In `src/app/app-icons.ts`, add `Copy` to the lucide import list and to `APP_ICONS`:

```ts
import {
  // ...existing...
  Copy,
} from 'lucide-angular';

export const APP_ICONS = {
  // ...existing...
  Copy,
};
```

- [ ] **Step 2: Component logic**

In `subject-detail.component.ts`, add imports:

```ts
import { open } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { ExportSubjectProgress } from '../../models/models';
```

Add signals + methods on the component class:

```ts
protected exporting = signal(false);
protected exportProgress = signal<ExportSubjectProgress | null>(null);
protected exportStatus = signal<string | null>(null);

protected async onCopyAll(): Promise<void> {
  if (this.exporting()) return;
  const id = this.subjectId();
  if (id === null) return;

  const selected = await open({ directory: true, multiple: false });
  if (!selected || typeof selected !== 'string') return;

  this.exporting.set(true);
  this.exportProgress.set({ current: 0, total: 0 });
  this.exportStatus.set(null);

  let unlisten: UnlistenFn | undefined;
  try {
    unlisten = await listen<ExportSubjectProgress>('export_subject_progress', (e) => {
      this.exportProgress.set(e.payload);
    });

    const result = await this.photos.exportSubjectPhotos(id, selected);

    const parts = [`Copied ${result.copied} photo${result.copied === 1 ? '' : 's'}`];
    if (result.skipped_missing > 0) {
      parts.push(`${result.skipped_missing} missing`);
    }
    if (result.skipped_errors > 0) {
      parts.push(`${result.skipped_errors} failed`);
    }
    this.exportStatus.set(parts.join(' · '));

    try {
      await openPath(result.dest_dir);
    } catch (openErr) {
      console.error('Failed to open export folder', openErr);
      this.exportStatus.set(
        `${parts.join(' · ')} (could not open folder)`,
      );
    }
  } catch (e) {
    console.error('Export failed', e);
    const msg = e instanceof Error ? e.message : String(e);
    this.exportStatus.set(msg || 'Export failed');
  } finally {
    if (unlisten) unlisten();
    this.exporting.set(false);
    this.exportProgress.set(null);
  }
}
```

- [ ] **Step 3: Template**

In the header of `subject-detail.component.html`, in the right-side actions area (before the ⋮ menu `div.relative`), add the Copy all button. Replace the trailing actions block structure with:

```html
    <div class="flex items-center gap-2">
      @if ((detail()?.photo_count ?? 0) > 0) {
        <button
          type="button"
          (click)="onCopyAll()"
          [disabled]="exporting()"
          class="flex items-center gap-2 px-3 py-1.5 text-sm rounded-md border border-border hover:bg-muted transition-colors text-foreground disabled:opacity-50 disabled:pointer-events-none"
          title="Copy all originals to a folder"
        >
          <lucide-icon name="copy" size="16"></lucide-icon>
          {{ exporting() ? 'Copying…' : 'Copy all…' }}
        </button>
      }

      <div class="relative">
        <button (click)="toggleMenu()" class="p-2 hover:bg-muted rounded-md transition-colors text-muted-foreground hover:text-foreground">
          <lucide-icon name="ellipsis-vertical" size="20"></lucide-icon>
        </button>

        @if (isMenuOpen()) {
          <div class="absolute right-0 mt-2 w-56 rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-lg z-20 animate-in fade-in zoom-in-95 duration-100">
            <button
              class="w-full flex items-center gap-2 px-2 py-1.5 text-sm rounded-sm hover:bg-accent hover:text-accent-foreground transition-colors"
              [routerLink]="['/subject', subjectId(), 'face-picker']"
              (click)="closeMenu()"
            >
              <lucide-icon name="star" size="14"></lucide-icon>
              Choose Representative Face
            </button>
          </div>
          <div class="fixed inset-0 z-10" (click)="closeMenu()"></div>
        }
      </div>
    </div>
```

(Ensure you remove the old standalone `div.relative` menu so it is not duplicated.)

Immediately **after** `</header>`, add progress + status:

```html
  @if (exporting() && exportProgress(); as prog) {
    <div class="px-6 py-2 border-b border-border bg-muted/40">
      <div class="text-xs text-muted-foreground mb-1">
        Copying {{ prog.current }} of {{ prog.total || '…' }}…
      </div>
      <div class="h-1.5 rounded-full bg-border overflow-hidden">
        <div
          class="h-full bg-primary transition-all"
          [style.width.%]="prog.total > 0 ? (prog.current / prog.total) * 100 : 0"
        ></div>
      </div>
    </div>
  } @else if (exportStatus()) {
    <div class="px-6 py-2 border-b border-border text-sm text-muted-foreground">
      {{ exportStatus() }}
    </div>
  }
```

- [ ] **Step 4: Opener capability (if needed)**

If runtime open fails with a permission/scope error, extend `src-tauri/capabilities/default.json` permissions to allow opening arbitrary user paths. Prefer the plugin’s documented path allow pattern, e.g.:

```json
{
  "identifier": "opener:allow-open-path",
  "allow": [{ "path": "**" }]
}
```

Only add if `opener:default` is insufficient (verify in `pnpm tauri dev`).

- [ ] **Step 5: Manual smoke (optional during agent work)**

`pnpm tauri dev` → open a subject with photos → Copy all… → pick temp dir → confirm files with `Parent_basename.ext` naming → folder opens → status line shows counts.

- [ ] **Step 6: Commit**

```bash
git add src/app/app-icons.ts \
  src/app/components/subject-detail/subject-detail.component.ts \
  src/app/components/subject-detail/subject-detail.component.html \
  src-tauri/capabilities/default.json
git commit -m "feat(ui): Copy all subject photos to folder and open destination"
```

---

### Task 6: Component tests

**Files:**
- Modify: `src/app/components/subject-detail/subject-detail.component.spec.ts`

- [ ] **Step 1: Extend stub + write tests**

Update `SubjectDetailPhotoServiceStub` with:

```ts
exportSubjectPhotos = vi.fn();
```

Add mocks at top of the file (after existing `@tauri-apps/api/core` mock):

```ts
const openDialogMock = vi.fn();
const openPathMock = vi.fn();
const listenMock = vi.fn().mockResolvedValue(() => {});

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => openDialogMock(...args),
}));

vi.mock('@tauri-apps/plugin-opener', () => ({
  openPath: (...args: unknown[]) => openPathMock(...args),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));
```

Add a describe block (mirror existing harness setup):

```ts
describe('SubjectDetailComponent — export', () => {
  let stub: SubjectDetailPhotoServiceStub;
  let harness: RouterTestingHarness;

  beforeEach(async () => {
    stub = new SubjectDetailPhotoServiceStub();
    stub.getSubjectDetail.mockResolvedValue(subjectDetail());
    openDialogMock.mockReset();
    openPathMock.mockReset();
    listenMock.mockReset().mockResolvedValue(() => {});
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'subject/:id', component: SubjectDetailComponent }]),
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        { provide: PhotoService, useValue: stub },
        { provide: TauriEventsService, useValue: mockTauriEvents },
      ],
    });
    harness = await RouterTestingHarness.create();
  });

  it('hides Copy all when photo_count is 0', async () => {
    stub.getSubjectDetail.mockResolvedValue(subjectDetail());
    const fixture = await harness.navigateByUrl('/subject/1', SubjectDetailComponent);
    await fixture.whenStable();
    fixture.detectChanges();
    const btn = fixture.nativeElement.querySelector('button[title="Copy all originals to a folder"]');
    expect(btn).toBeNull();
  });

  it('shows Copy all when photo_count > 0', async () => {
    stub.getSubjectDetail.mockResolvedValue({
      ...subjectDetail(),
      photo_count: 3,
    });
    const fixture = await harness.navigateByUrl('/subject/1', SubjectDetailComponent);
    await fixture.whenStable();
    fixture.detectChanges();
    const btn = fixture.nativeElement.querySelector('button[title="Copy all originals to a folder"]');
    expect(btn).toBeTruthy();
    expect(btn.textContent).toContain('Copy all');
  });

  it('does not invoke export when dialog is cancelled', async () => {
    stub.getSubjectDetail.mockResolvedValue({ ...subjectDetail(), photo_count: 2 });
    openDialogMock.mockResolvedValue(null);
    const fixture = await harness.navigateByUrl('/subject/1', SubjectDetailComponent);
    await fixture.whenStable();
    fixture.detectChanges();
    const cmp = fixture.componentInstance as SubjectDetailComponent;
    await cmp.onCopyAll();
    expect(stub.exportSubjectPhotos).not.toHaveBeenCalled();
    expect(openPathMock).not.toHaveBeenCalled();
  });

  it('exports, shows status, and opens destination on success', async () => {
    stub.getSubjectDetail.mockResolvedValue({ ...subjectDetail(), photo_count: 2 });
    openDialogMock.mockResolvedValue('/tmp/cass-export');
    stub.exportSubjectPhotos.mockResolvedValue({
      dest_dir: '/tmp/cass-export',
      copied: 2,
      skipped_missing: 0,
      skipped_errors: 0,
    });
    openPathMock.mockResolvedValue(undefined);

    const fixture = await harness.navigateByUrl('/subject/1', SubjectDetailComponent);
    await fixture.whenStable();
    fixture.detectChanges();
    const cmp = fixture.componentInstance as SubjectDetailComponent;
    await cmp.onCopyAll();
    fixture.detectChanges();

    expect(stub.exportSubjectPhotos).toHaveBeenCalledWith(1, '/tmp/cass-export');
    expect(openPathMock).toHaveBeenCalledWith('/tmp/cass-export');
    expect(cmp.exportStatus()).toContain('Copied 2');
  });

  it('does not open folder when export fails', async () => {
    stub.getSubjectDetail.mockResolvedValue({ ...subjectDetail(), photo_count: 2 });
    openDialogMock.mockResolvedValue('/tmp/cass-export');
    stub.exportSubjectPhotos.mockRejectedValue(new Error('Subject not found'));

    const fixture = await harness.navigateByUrl('/subject/1', SubjectDetailComponent);
    await fixture.whenStable();
    fixture.detectChanges();
    const cmp = fixture.componentInstance as SubjectDetailComponent;
    await cmp.onCopyAll();
    fixture.detectChanges();

    expect(openPathMock).not.toHaveBeenCalled();
    expect(cmp.exportStatus()).toMatch(/Subject not found|Export failed/i);
  });
});
```

Adjust harness patterns to match existing tests in the same file if `navigateByUrl` usage differs — read neighboring describes and stay consistent.

- [ ] **Step 2: Run tests**

```bash
pnpm exec vitest run src/app/components/subject-detail/subject-detail.component.spec.ts
```

Expected: PASS. Fix selector/stub/export visibility as needed.

Also run app-icons check if present:

```bash
pnpm exec vitest run src/app/app-icons.spec.ts
```

Expected: PASS (`copy` registered).

- [ ] **Step 3: Commit**

```bash
git add src/app/components/subject-detail/subject-detail.component.spec.ts
git commit -m "test(ui): subject detail Copy all visibility and export orchestration"
```

---

### Task 7: Final verification

- [ ] **Step 1: Rust tests + fmt**

```bash
cd src-tauri && cargo fmt --all && cargo test --lib library::export:: -- --nocapture
```

Expected: PASS.

- [ ] **Step 2: Frontend unit tests for touched specs**

```bash
pnpm exec vitest run src/app/components/subject-detail/subject-detail.component.spec.ts src/app/app-icons.spec.ts
```

Expected: PASS.

- [ ] **Step 3: Optional full clippy (matches pre-push)**

```bash
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

Fix any issues introduced by this feature only.

- [ ] **Step 4: Mark plan complete**

If anything drifted from the spec during implementation, update the design doc’s Status line to `implemented` in a docs commit — optional.

---

## Spec coverage checklist

| Spec requirement | Task |
|------------------|------|
| Copy all on subject detail | 5 |
| Full-res originals | 2–3 (`img.path`) |
| Native directory dialog | 5 |
| Flat export | 1–2 |
| Parent-folder prefix naming | 1 |
| Collision ` (N)` suffix, never overwrite | 1–2 |
| Open destination after success | 5 |
| Progress events + UI | 3, 5 |
| Soft-skip missing | 2 |
| Hard-fail bad dest / unknown subject | 2–3 |
| Inline status (no toast system) | 5 |
| Backend unit tests | 1–2 |
| Frontend orchestration tests | 6 |
| Non-goals (multi-select, Google API, filters) | not implemented ✓ |

## Placeholder / consistency review

- Types aligned: `ExportSubjectResult` / `ExportSubjectProgress` in Rust + TS; event name `export_subject_progress`.
- Service method `exportSubjectPhotos(subjectId, destDir)` matches invoke payload.
- Helper API: `preferred_dest_name`, `unique_dest_path`, `validate_dest_dir`, `copy_paths_to_dir`, `CopyFilesResult`.
- No TBD/TODO left in plan steps.
