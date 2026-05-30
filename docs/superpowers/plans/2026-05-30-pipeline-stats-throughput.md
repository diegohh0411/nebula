# Pipeline Stats & Throughput Badge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `processing_progress` Tauri event end-to-end with a unified `pipeline_stats` event that carries `total_pending` and `images_per_sec`, and redesign the search-bar badge for non-technical users.

**Architecture:** A `VecDeque`-based rolling-window tracker in `run_pipeline` computes images/sec over the last 5 seconds and passes the rate to `emit_progress`, which now emits `pipeline_stats`. The frontend collapses per-type pending counts into a single total and drives a 3-state badge machine (active → completing → idle) in the search-bar component.

**Tech Stack:** Rust/Tauri (backend), Angular 17+ signals + effects (frontend), `std::collections::VecDeque`, `std::time::Instant`.

---

## File Map

| File | Change |
|---|---|
| `src-tauri/src/models/entities.rs` | Remove `ProcessingProgressPayload`; add `PipelineStatsPayload` |
| `src-tauri/src/embedder.rs` | Update `emit_progress` signature + event name |
| `src-tauri/src/indexer.rs` | Update 2 call sites to pass `0.0` |
| `src-tauri/src/pipeline/mod.rs` | Add rolling-window tracker; pass rate to `emit_progress` |
| `src-tauri/src/commands.rs` | Update `get_processing_status` return type |
| `src/app/models/models.ts` | Remove `ProcessingStatus`/`ProcessingProgressEvent`; add `PipelineStats` |
| `src/app/services/tauri-events.service.ts` | Replace `processingProgress$` with `pipelineStats$` |
| `src/app/services/photo.service.ts` | Replace `processingStatus` signal with `pipelineStats` |
| `src/app/components/search-bar/search-bar.component.ts` | Add 3-state badge machine using `effect` |
| `src/app/components/search-bar/search-bar.component.html` | New badge template |

---

## Task 1: Add `PipelineStatsPayload` to Rust models

**Files:**
- Modify: `src-tauri/src/models/entities.rs`

- [ ] **Step 1: Replace `ProcessingProgressPayload` with `PipelineStatsPayload`**

In `src-tauri/src/models/entities.rs`, find and remove the `ProcessingProgressPayload` struct (around line 41–46) and replace it with:

```rust
#[derive(Debug, Serialize, Clone)]
pub struct PipelineStatsPayload {
    pub total_pending: u32,
    pub images_per_sec: f32,
}
```

The old struct to remove:
```rust
#[derive(Debug, Serialize, Clone)]
pub struct ProcessingProgressPayload {
    pub semantic_pending: i64,
    pub subject_pending: i64,
    pub done: i64,
}
```

- [ ] **Step 2: Verify the file compiles in isolation**

```bash
cd src-tauri && cargo check 2>&1 | grep "models/entities"
```

Expected: no errors mentioning `entities.rs` (other files will fail until Tasks 2–3 are done).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/models/entities.rs
git commit -m "feat: add PipelineStatsPayload, remove ProcessingProgressPayload"
```

---

## Task 2: Update `emit_progress` and all call sites

**Files:**
- Modify: `src-tauri/src/embedder.rs`
- Modify: `src-tauri/src/indexer.rs`

- [ ] **Step 1: Update `emit_progress` in `embedder.rs`**

Replace the entire `emit_progress` function (currently at the bottom of `embedder.rs`):

```rust
pub(crate) async fn emit_progress(pool: &SqlitePool, app: &AppHandle, images_per_sec: f32) {
    if let Ok(status) = db::get_processing_counts(pool).await {
        let total_pending = (status.semantic_pending + status.subject_pending) as u32;
        let _ = app.emit(
            "pipeline_stats",
            crate::models::PipelineStatsPayload {
                total_pending,
                images_per_sec,
            },
        );
    }
}
```

Also remove the now-unused import of `ProcessingProgressPayload` from this file if present (check the import block at the top — it imports `ProcessingProgressPayload` via `use crate::{db, models::ProcessingProgressPayload}`; replace with `use crate::{db, models::PipelineStatsPayload}`).

- [ ] **Step 2: Update 2 call sites in `indexer.rs`**

Line 444 and 447 in `src-tauri/src/indexer.rs` both call `crate::embedder::emit_progress(&self.pool, &self.app)`. Update both to:

```rust
crate::embedder::emit_progress(&self.pool, &self.app, 0.0).await;
```

- [ ] **Step 3: Verify the backend compiles (excluding pipeline/mod.rs)**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error"
```

Expected: only errors in `pipeline/mod.rs` (still uses the old call signature). All others resolved.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embedder.rs src-tauri/src/indexer.rs
git commit -m "feat: update emit_progress to emit pipeline_stats event"
```

---

## Task 3: Update `get_processing_status` command

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Update the command return type**

In `src-tauri/src/commands.rs`, find the `get_processing_status` command (around line 198). Replace it with:

```rust
#[tauri::command]
pub async fn get_processing_status(
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::PipelineStatsPayload, String> {
    db::get_processing_counts(&state.pool).await
        .map(|s| crate::models::PipelineStatsPayload {
            total_pending: (s.semantic_pending + s.subject_pending) as u32,
            images_per_sec: 0.0,
        })
        .map_err(map_err)
}
```

Also remove `ProcessingStatus` from the import at the top of `commands.rs` (line 7 currently reads `models::{ProcessingStatus, FolderWithCount, ...}`). Remove `ProcessingStatus` from that list.

- [ ] **Step 2: Verify**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error"
```

Expected: only errors in `pipeline/mod.rs`.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat: update get_processing_status to return PipelineStatsPayload"
```

---

## Task 4: Add rolling-window tracker in the pipeline

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`

- [ ] **Step 1: Add imports at the top of `pipeline/mod.rs`**

At the top of `src-tauri/src/pipeline/mod.rs`, the existing imports include `use std::time::Duration;`. Extend them:

```rust
use std::collections::VecDeque;
use std::time::{Duration, Instant};
```

- [ ] **Step 2: Add the tracker variable before the pipeline loop**

In `run_pipeline`, just before the `loop {` line, add:

```rust
let mut throughput_window: VecDeque<(Instant, usize)> = VecDeque::new();
```

- [ ] **Step 3: Record batch completions and compute rate**

In `run_pipeline`, just before the existing `crate::embedder::emit_progress(&pool, &app).await;` call (currently at the very bottom of the loop body), insert the rolling-window update:

```rust
// Rolling-window throughput: track how many images completed in the last 5 s.
let batch_images = {
    // Count images that were actually processed this iteration (decoded successfully).
    // We already have `decoded` at this point — but it was consumed. Use the pending
    // variable that tracks face work, or simply count all dispatched work items.
    // The simplest correct count: total images that reached Phase B (all entries in
    // `pending` that were built). Since `pending` is consumed by the loop above,
    // capture the count before the loop using a local variable (see Step 4).
    images_processed_this_iter
};
let now = Instant::now();
throughput_window.push_back((now, batch_images));
throughput_window.retain(|(t, _)| t.elapsed() <= Duration::from_secs(5));
let sum_images: usize = throughput_window.iter().map(|(_, n)| n).sum();
let window_span = throughput_window
    .front()
    .map(|(t, _)| t.elapsed())
    .unwrap_or(Duration::from_millis(1))
    .max(Duration::from_millis(1));
let images_per_sec = sum_images as f32 / window_span.as_secs_f32();
```

- [ ] **Step 4: Add `images_processed_this_iter` counter**

Before the `// Phase A` comment in the loop, add:

```rust
let images_processed_this_iter = decoded.len();
```

This captures the count of images that successfully decoded and will be processed in Phase B.

- [ ] **Step 5: Update the `emit_progress` call**

Change the existing call at the bottom of the loop from:

```rust
crate::embedder::emit_progress(&pool, &app).await;
```

to:

```rust
crate::embedder::emit_progress(&pool, &app, images_per_sec).await;
```

- [ ] **Step 6: Verify the full backend compiles**

```bash
cd src-tauri && cargo check 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "feat: add rolling-window throughput tracker to pipeline"
```

---

## Task 5: Update frontend models and services

**Files:**
- Modify: `src/app/models/models.ts`
- Modify: `src/app/services/tauri-events.service.ts`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Update `models.ts`**

In `src/app/models/models.ts`:

1. Remove the `ProcessingStatus` interface (currently lines ~34–38):
```ts
// DELETE THIS:
export interface ProcessingStatus {
  semantic_pending: number;
  subject_pending: number;
  done: number;
}
```

2. Remove the `ProcessingProgressEvent` interface (currently lines ~40–44):
```ts
// DELETE THIS:
export interface ProcessingProgressEvent {
  semantic_pending: number;
  subject_pending: number;
  done: number;
}
```

3. Add the new interface in their place:
```ts
export interface PipelineStats {
  total_pending: number;
  images_per_sec: number;
}
```

4. Remove the `getProcessingStage` function — it reads `.semantic_analysis_done` and `.subject_analysis_done` from image objects, which are unrelated to the event, so keep it. **Do not remove it.**

- [ ] **Step 2: Update `TauriEventsService`**

In `src/app/services/tauri-events.service.ts`:

Replace the import and the `processingProgress$` subject + listener with `pipelineStats$`:

```ts
import { Injectable, OnDestroy } from '@angular/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { Subject } from 'rxjs';
import {
  PipelineStats,
  ImageAddedEvent,
  ImageRemovedEvent,
  ImageUpdatedEvent,
  ModelDownloadEvent,
} from '../models/models';

@Injectable({ providedIn: 'root' })
export class TauriEventsService implements OnDestroy {
  readonly pipelineStats$ = new Subject<PipelineStats>();
  readonly imageAdded$ = new Subject<ImageAddedEvent>();
  readonly imageUpdated$ = new Subject<ImageUpdatedEvent>();
  readonly imageRemoved$ = new Subject<ImageRemovedEvent>();
  readonly modelDownloadProgress$ = new Subject<ModelDownloadEvent>();

  private unlisteners: UnlistenFn[] = [];

  constructor() {
    this.setupListeners();
  }

  private async setupListeners(): Promise<void> {
    this.unlisteners.push(
      await listen<PipelineStats>('pipeline_stats', (e) =>
        this.pipelineStats$.next(e.payload)
      ),
      await listen<ImageAddedEvent>('image_added', (e) =>
        this.imageAdded$.next(e.payload)
      ),
      await listen<ImageUpdatedEvent>('image_updated', (e) =>
        this.imageUpdated$.next(e.payload)
      ),
      await listen<ImageRemovedEvent>('image_removed', (e) =>
        this.imageRemoved$.next(e.payload)
      ),
      await listen<ModelDownloadEvent>('model_download_progress', (e) =>
        this.modelDownloadProgress$.next(e.payload)
      )
    );
  }

  ngOnDestroy(): void {
    this.unlisteners.forEach((fn) => fn());
  }
}
```

- [ ] **Step 3: Update `PhotoService`**

In `src/app/services/photo.service.ts`:

1. Replace the `ProcessingStatus` import with `PipelineStats`:
```ts
import {
  DayGroup,
  PipelineStats,       // ← replaces ProcessingStatus
  Folder,
  Image,
  SearchResult,
  VirtualRow,
  Subject,
  Face,
  MergeSuggestion,
  NameSubjectResult,
  SubjectDetail,
} from '../models/models';
```

2. Replace the `processingStatus` signal declaration (~line 31):
```ts
// OLD:
readonly processingStatus = signal<ProcessingStatus>({ semantic_pending: 0, subject_pending: 0, done: 0 });
// NEW:
readonly pipelineStats = signal<PipelineStats>({ total_pending: 0, images_per_sec: 0 });
```

3. In the constructor, replace the `processingProgress$` subscription (~line 116):
```ts
// OLD:
this.events.processingProgress$.subscribe((e) => {
  this.processingStatus.set(e);
});
// NEW:
this.events.pipelineStats$.subscribe((e) => {
  this.pipelineStats.set(e);
});
```

4. Replace `refreshProcessingStatus` (~line 200):
```ts
async refreshProcessingStatus(): Promise<void> {
  const stats = await invoke<PipelineStats>('get_processing_status');
  this.pipelineStats.set(stats);
}
```

- [ ] **Step 4: Verify TypeScript compiles**

```bash
cd /home/user/nebula && npx tsc --noEmit 2>&1 | head -40
```

Expected: no errors related to `processingStatus`, `ProcessingStatus`, or `processingProgress$`.

- [ ] **Step 5: Commit**

```bash
git add src/app/models/models.ts \
        src/app/services/tauri-events.service.ts \
        src/app/services/photo.service.ts
git commit -m "feat: replace processingStatus with pipelineStats in frontend services"
```

---

## Task 6: Redesign the search-bar badge

**Files:**
- Modify: `src/app/components/search-bar/search-bar.component.ts`
- Modify: `src/app/components/search-bar/search-bar.component.html`

- [ ] **Step 1: Update `search-bar.component.ts`**

Replace the full file content with:

```ts
import {
  Component,
  ChangeDetectionStrategy,
  inject,
  signal,
  effect,
  OnDestroy,
} from '@angular/core';
import { DecimalPipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { PhotoService } from '../../services/photo.service';

type BadgeState = 'active' | 'completing' | 'idle';

@Component({
  selector: 'app-search-bar',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, DecimalPipe],
  templateUrl: './search-bar.component.html',
  styleUrl: './search-bar.component.css',
})
export class SearchBarComponent implements OnDestroy {
  protected photos = inject(PhotoService);
  protected query = signal('');
  protected isDragOver = signal(false);
  protected badgeState = signal<BadgeState>('idle');

  private completingTimer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    effect(() => {
      const stats = this.photos.pipelineStats();
      if (stats.total_pending > 0) {
        if (this.completingTimer !== null) {
          clearTimeout(this.completingTimer);
          this.completingTimer = null;
        }
        this.badgeState.set('active');
      } else if (this.badgeState() === 'active') {
        this.badgeState.set('completing');
        this.completingTimer = setTimeout(() => {
          this.badgeState.set('idle');
          this.completingTimer = null;
        }, 2500);
      }
    });
  }

  ngOnDestroy(): void {
    if (this.completingTimer !== null) {
      clearTimeout(this.completingTimer);
    }
  }

  protected onSearch(): void {
    void this.photos.searchByText(this.query());
  }

  protected onClear(): void {
    this.query.set('');
    this.photos.clearSearch();
  }

  protected onDragOver(event: DragEvent): void {
    event.preventDefault();
    this.isDragOver.set(true);
  }

  protected onDragLeave(event: DragEvent): void {
    this.isDragOver.set(false);
  }

  protected onDrop(event: DragEvent): void {
    event.preventDefault();
    this.isDragOver.set(false);
    const file = event.dataTransfer?.files[0];
    if (!file || !file.type.startsWith('image/')) return;
    const reader = new FileReader();
    reader.onload = () => {
      const base64 = (reader.result as string).split(',')[1];
      const objectUrl = URL.createObjectURL(file);
      void this.photos.searchByExternalImage(base64, file.type, objectUrl);
    };
    reader.readAsDataURL(file);
  }

  protected onPaste(event: ClipboardEvent): void {
    const items = event.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        const file = item.getAsFile();
        if (!file) continue;
        const reader = new FileReader();
        reader.onload = () => {
          const base64 = (reader.result as string).split(',')[1];
          const objectUrl = URL.createObjectURL(file);
          void this.photos.searchByExternalImage(base64, file.type, objectUrl);
        };
        reader.readAsDataURL(file);
        break;
      }
    }
  }

  protected clearImageSearch(): void {
    this.query.set('');
    this.photos.clearSearch();
  }
}
```

- [ ] **Step 2: Update `search-bar.component.html`**

Replace the processing status badge section. The old block (lines 54–65) reads:

```html
  @if (photos.processingStatus().semantic_pending > 0 || photos.processingStatus().subject_pending > 0) {
    <span class="embed-badge" title="Photos still being processed">
      <span class="embed-badge-dot"></span>
      @if (...) { ... }
    </span>
  }
```

Replace that entire `@if` block with:

```html
  @if (badgeState() !== 'idle') {
    <span class="embed-badge" [title]="badgeState() === 'active' ? 'Photos still being processed' : 'All photos are up to date'">
      @if (badgeState() === 'active') {
        <span class="embed-badge-dot"></span>
        Processing {{ photos.pipelineStats().total_pending }} images
        @if (photos.pipelineStats().images_per_sec >= 0.5) {
          · {{ photos.pipelineStats().images_per_sec | number:'1.0-0' }} img/s
        }
      } @else {
        Library up to date
      }
    </span>
  }
```

Leave the rest of the template (app-name, search input, search error) unchanged.

- [ ] **Step 3: Verify TypeScript compiles**

```bash
cd /home/user/nebula && npx tsc --noEmit 2>&1 | head -40
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/app/components/search-bar/search-bar.component.ts \
        src/app/components/search-bar/search-bar.component.html
git commit -m "feat: redesign processing badge with throughput and up-to-date state"
```

---

## Task 7: Final wiring check and push

- [ ] **Step 1: Full backend build**

```bash
cd /home/user/nebula/src-tauri && cargo build 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 2: Full frontend build**

```bash
cd /home/user/nebula && npx tsc --noEmit 2>&1
```

Expected: no errors.

- [ ] **Step 3: Verify no remaining references to old API**

```bash
grep -rn "processing_progress\|processingProgress\|ProcessingProgressPayload\|ProcessingStatus\|processingStatus\b" \
  src-tauri/src/ src/app/ 2>/dev/null | grep -v ".md"
```

Expected: no matches.

- [ ] **Step 4: Push**

```bash
git push -u origin claude/sweet-dirac-Fm0QR
```
