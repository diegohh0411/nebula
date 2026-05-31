# Pipeline Stats Freshness & Granularity (TT-7 + TT-14) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Ensure the processing badge is always up-to-date and decrements smoothly (picture-by-picture) during inference.

**Architecture:** 
1.  **Backend:** Plumb the real-time EMA throughput (from TT-10) into a thread-safe `AtomicU32` in `AppState` so `get_processing_status` can return the live rate instead of `0.0`.
2.  **Frontend:** Implement a 1-second polling interval in `PhotoService` that runs only while `total_pending > 0`. This ensures the UI captures individual image completions recorded in the DB even between sparse backend events, solving the "jumps" in the count.

**Tech Stack:** Rust (Tauri, Atomic types), Angular (Signals, RxJS `timer`).

---

## File Map

| File | Change |
|---|---|
| `src-tauri/src/main.rs` | Add `throughput_ema: AtomicU32` (bit-casted f32) to `AppState` |
| `src-tauri/src/pipeline/mod.rs` | Update `AppState` throughput on every batch |
| `src-tauri/src/commands.rs` | Update `get_processing_status` to read from `AppState` |
| `src/app/services/photo.service.ts` | Add 1s polling logic while pipeline is active |

---

## Task 1: Add Throughput Storage to AppState

**Files:**
- Modify: `src-tauri/src/main.rs`

- [x] **Step 1: Add `throughput_ema` to `AppState`**

In `src-tauri/src/main.rs`, update the `AppState` struct to include an atomic for storing the latest throughput. We use `AtomicU32` to store a bit-casted `f32` since there is no native `AtomicF32`.

```rust
pub struct AppState {
    pub pool: SqlitePool,
    pub index: Arc<RwLock<VectorIndex>>,
    pub engine: Arc<VisionEngine>,
    pub manager: Arc<ModelManager>,
    pub throughput_ema: std::sync::atomic::AtomicU32, // Added: bits of f32
}
```

- [x] **Step 2: Initialize in `main()`**

Update the `AppState` initialization in the `main` function:

```rust
    let state = AppState {
        pool,
        index,
        engine,
        manager,
        throughput_ema: std::sync::atomic::AtomicU32::new(0.0f32.to_bits()),
    };
```

- [x] **Step 3: Verify compilation**

Run: `cd src-tauri && cargo check`
Expected: PASS

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(backend): add throughput_ema storage to AppState"
```

---

## Task 2: Update AppState from the Pipeline

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`

- [x] **Step 1: Update the atomic value in the loop**

In `src-tauri/src/pipeline/mod.rs`, find the `run_pipeline` function. Inside the main loop, where `throughput_ema` is computed (around line 350), update the shared state.

```rust
        let ema = match throughput_ema {
            None => inst_rate,
            Some(prev) => 0.3 * inst_rate + 0.7 * prev,
        };
        throughput_ema = Some(ema);

        // Added: Update shared AppState for the pull path (TT-7)
        let app_state: tauri::State<crate::AppState> = app.state();
        app_state.throughput_ema.store(ema.to_bits(), std::sync::atomic::Ordering::Relaxed);

        crate::embedder::emit_progress(&pool, &app, ema).await;
```

- [x] **Step 2: Verify compilation**

Run: `cd src-tauri && cargo check`
Expected: PASS

- [x] **Step 3: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "feat(pipeline): publish live throughput to AppState"
```

---

## Task 3: Return real throughput in `get_processing_status`

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [x] **Step 1: Read from `AppState` in the command**

In `src-tauri/src/commands.rs`, update `get_processing_status` to read the bit-casted float from the atomic.

```rust
#[tauri::command]
pub async fn get_processing_status(
    state: tauri::State<'_, AppState>,
) -> Result<crate::models::PipelineStatsPayload, String> {
    let ema_bits = state.throughput_ema.load(std::sync::atomic::Ordering::Relaxed);
    let images_per_sec = f32::from_bits(ema_bits);

    db::get_processing_counts(&state.pool).await
        .map(|s| crate::models::PipelineStatsPayload {
            total_pending: s.total_pending as u32,
            images_per_sec, // Now dynamic instead of 0.0
        })
        .map_err(map_err)
}
```

- [x] **Step 2: Verify backend**

Run: `cd src-tauri && cargo check`
Expected: PASS

- [x] **Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "fix(commands): return real throughput in get_processing_status"
```

---

## Task 4: Implement Frontend Polling for Smooth Decrements

**Files:**
- Modify: `src/app/services/photo.service.ts`

- [x] **Step 1: Add polling logic to `PhotoService`**

We want to poll the status every 1 second, but ONLY when `total_pending > 0`. We use RxJS `timer` and `switchMap` for this.

In `src/app/services/photo.service.ts`, add the following imports if missing:
```ts
import { timer, of, Subscription } from 'rxjs';
import { switchMap, filter, distinctUntilChanged } from 'rxjs/operators';
```

Add a private subscription tracker and the polling effect in the constructor:

```ts
  private pollingSub?: Subscription;

  constructor() {
    // ... existing event subscriptions ...

    // TT-7/TT-14: Freshness & Granularity Poll
    // Starts polling when pending > 0, stops when 0.
    effect(() => {
      const isProcessing = this.pipelineStats().total_pending > 0;
      
      if (isProcessing && !this.pollingSub) {
        this.pollingSub = timer(0, 1000).pipe(
          switchMap(() => this.refreshProcessingStatus())
        ).subscribe();
      } else if (!isProcessing && this.pollingSub) {
        this.pollingSub.unsubscribe();
        this.pollingSub = undefined;
      }
    });
  }
```

*Note: Ensure `refreshProcessingStatus` is updated to handle potential concurrent calls or just let it run since it's a simple GET.*

- [x] **Step 2: Verify frontend compilation**

Run: `npx tsc --noEmit`
Expected: PASS

- [x] **Step 3: Commit**

```bash
git add src/app/services/photo.service.ts
git commit -m "feat(ui): add 1s status polling while pipeline is active (TT-7, TT-14)"
```

---

## Task 5: Final Verification

- [x] **Step 1: Full build**

```bash
cd src-tauri && cargo build
cd .. && npm run build
```

- [x] **Step 2: Verification Check**

1.  Start processing a large folder.
2.  Observe the badge: it should update every second (TT-7).
3.  The count should decrement by ~1 at a time (TT-14) rather than jumping by 12.
4.  The `img/s` figure should be non-zero immediately.
5.  Stop processing: the polling should cease.

- [x] **Step 3: Final Commit**

```bash
git commit --allow-empty -m "docs: complete TT-7 and TT-14 combined implementation"
```
