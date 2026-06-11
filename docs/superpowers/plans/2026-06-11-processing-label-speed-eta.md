# Processing Label: Reliable Speed + ETA — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the processing badge's inference speed from disappearing during large imports, and add an estimated time to completion.

**Architecture:** Make `AppState.throughput_ema` the single source of truth for speed (written only by the pipeline loop); `emit_progress` reads it instead of taking a rate argument, so the folder scanner's progress heartbeats stop overwriting the real speed with `0.0`. Harden with an `effective_rate` helper (hold last-known when the window momentarily lacks samples) and a frontend hold-last-known guard. Derive ETA on the frontend from `total_pending / images_per_sec`.

**Tech Stack:** Rust (Tauri backend, `cargo test`), Angular + signals (TypeScript), Vitest (`vitest run`).

**Task reference:** Notion TT-64. Spec: `docs/superpowers/specs/2026-06-11-processing-label-speed-eta-design.md`.

---

## File Structure

- `src-tauri/src/pipeline/throughput.rs` — add pure `effective_rate` helper + tests (already owns throughput logic & a `#[cfg(test)]` module).
- `src-tauri/src/embedder.rs` — refactor `emit_progress` to read `throughput_ema` internally (drop the `images_per_sec` param).
- `src-tauri/src/pipeline/mod.rs` — use `effective_rate` at the publish site; reset `throughput_ema` to 0 when queues drain; update `emit_progress` call.
- `src-tauri/src/indexer.rs` — update the two `emit_progress` call sites (drop `0.0` argument).
- `src/app/models/models.ts` — add `formatEta` pure util.
- `src/app/models/models.spec.ts` — **create**, unit tests for `formatEta`.
- `src/app/services/photo.service.ts` — add `etaSeconds` computed; hold-last-known in the `pipelineStats$` subscription.
- `src/app/services/photo.service.spec.ts` — add tests for hold-last-known + `etaSeconds`.
- `src/app/components/search-bar/search-bar.component.ts` — expose `formatEta` to the template.
- `src/app/components/search-bar/search-bar.component.html` — render the ETA segment.

---

## Task 1: Backend — `effective_rate` helper

**Files:**
- Modify/Test: `src-tauri/src/pipeline/throughput.rs` (add fn + tests to existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block at the bottom of `src-tauri/src/pipeline/throughput.rs`:

```rust
    #[test]
    fn effective_rate_uses_raw_when_positive() {
        assert_eq!(super::effective_rate(12.5, 3.0), 12.5);
    }

    #[test]
    fn effective_rate_holds_prev_when_raw_is_zero() {
        // Window momentarily has <2 samples → raw 0; must hold the last value.
        assert_eq!(super::effective_rate(0.0, 9.0), 9.0);
    }

    #[test]
    fn effective_rate_returns_zero_when_both_zero() {
        assert_eq!(super::effective_rate(0.0, 0.0), 0.0);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p nebula effective_rate`
Expected: FAIL — `cannot find function 'effective_rate' in module 'super'` (compile error).

Note: if the package name differs, use `cargo test effective_rate` from `src-tauri`.

- [ ] **Step 3: Write the implementation**

Add this free function to `src-tauri/src/pipeline/throughput.rs` (above the `#[cfg(test)]` module, after the `impl ThroughputWindow` block):

```rust
/// Returns `raw` when it is a usable (> 0) rate, else falls back to the last
/// published value (`prev`). A transient "not enough samples" 0 from the sliding
/// window must never blank the displayed speed while processing is active.
pub fn effective_rate(raw: f32, prev: f32) -> f32 {
    if raw > 0.0 { raw } else { prev }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test effective_rate`
Expected: PASS (3 tests), plus the existing throughput tests still green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pipeline/throughput.rs
git commit -m "feat(TT-64): effective_rate helper holds last-known throughput"
```

---

## Task 2: Backend — `emit_progress` reads canonical speed; wire resilience + scanner fix

This task has no isolated unit test (it needs a live `AppHandle`); the verification is a clean compile + the existing suite staying green. Make all the edits, then build/test.

**Files:**
- Modify: `src-tauri/src/embedder.rs` (the `emit_progress` fn, ~line 47)
- Modify: `src-tauri/src/pipeline/mod.rs` (publish site ~391-398; empty-queue branch ~158-161)
- Modify: `src-tauri/src/indexer.rs:450,453`

- [ ] **Step 1: Refactor `emit_progress` to read `throughput_ema`**

Replace the existing `emit_progress` function in `src-tauri/src/embedder.rs` (currently `pub(crate) async fn emit_progress(pool: &SqlitePool, app: &AppHandle, images_per_sec: f32)`) with:

```rust
pub(crate) async fn emit_progress(pool: &SqlitePool, app: &AppHandle) {
    use tauri::Manager;
    let images_per_sec = {
        let state = app.state::<crate::AppState>();
        f32::from_bits(
            state
                .throughput_ema
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    };
    if let Ok(status) = db::get_processing_counts(pool).await {
        let _ = app.emit(
            "pipeline_stats",
            crate::models::PipelineStatsPayload {
                total_pending: status.total_pending as u32,
                images_per_sec,
            },
        );
    }
}
```

- [ ] **Step 2: Wire `effective_rate` at the pipeline publish site**

In `src-tauri/src/pipeline/mod.rs`, replace the block at ~lines 390-398:

```rust
        let now_secs = pipeline_start.elapsed().as_secs_f32();
        throughput_window.record(images_processed_this_iter, now_secs);
        let rate = throughput_window.rate(now_secs);

        // Update shared AppState for the pull path (TT-7)
        let app_state: tauri::State<crate::AppState> = app.state();
        app_state.throughput_ema.store(rate.to_bits(), std::sync::atomic::Ordering::Relaxed);

        crate::embedder::emit_progress(&pool, &app, rate).await;
```

with:

```rust
        let now_secs = pipeline_start.elapsed().as_secs_f32();
        throughput_window.record(images_processed_this_iter, now_secs);
        let raw_rate = throughput_window.rate(now_secs);

        // Single source of truth for speed (TT-7/TT-64): hold the last-known rate
        // when the window momentarily lacks samples, so the speed never blanks
        // mid-processing.
        let app_state: tauri::State<crate::AppState> = app.state();
        let prev = f32::from_bits(app_state.throughput_ema.load(std::sync::atomic::Ordering::Relaxed));
        let effective = throughput::effective_rate(raw_rate, prev);
        app_state.throughput_ema.store(effective.to_bits(), std::sync::atomic::Ordering::Relaxed);

        crate::embedder::emit_progress(&pool, &app).await;
```

- [ ] **Step 3: Reset the stored speed when the queues drain**

In `src-tauri/src/pipeline/mod.rs`, replace the empty-queue branch at ~lines 158-161:

```rust
        if sem_batch.is_empty() && sub_batch.is_empty() {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
```

with:

```rust
        if sem_batch.is_empty() && sub_batch.is_empty() {
            // Nothing pending: clear the held speed so a finished run does not
            // leak a stale rate into the next import (TT-64).
            let app_state: tauri::State<crate::AppState> = app.state();
            app_state
                .throughput_ema
                .store(0.0f32.to_bits(), std::sync::atomic::Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
```

- [ ] **Step 4: Stop the scanner from emitting a fake 0.0 speed**

In `src-tauri/src/indexer.rs`, change both call sites (currently at lines 450 and 453):

```rust
                crate::embedder::emit_progress(&self.pool, &self.app, 0.0).await;
```
and
```rust
        crate::embedder::emit_progress(&self.pool, &self.app, 0.0).await;
```

to (drop the `0.0` argument in both):

```rust
                crate::embedder::emit_progress(&self.pool, &self.app).await;
```
and
```rust
        crate::embedder::emit_progress(&self.pool, &self.app).await;
```

- [ ] **Step 5: Build and run the backend test suite**

Run: `cd src-tauri && cargo test`
Expected: Compiles cleanly (no remaining 3-arg `emit_progress` callers) and all tests pass, including Task 1's `effective_rate` tests.

If the compiler flags any other `emit_progress` caller, update it to the no-rate form (grep first: `grep -rn emit_progress src-tauri/src`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/embedder.rs src-tauri/src/pipeline/mod.rs src-tauri/src/indexer.rs
git commit -m "fix(TT-64): scanner no longer zeroes inference speed; speed has single source of truth"
```

---

## Task 3: Frontend — `formatEta` util

**Files:**
- Modify: `src/app/models/models.ts`
- Create/Test: `src/app/models/models.spec.ts`

- [ ] **Step 1: Write the failing tests**

Create `src/app/models/models.spec.ts` (Vitest globals are enabled in this project, matching the existing specs — no need to import `describe`/`it`/`expect`):

```ts
import { formatEta } from './models';

describe('formatEta', () => {
  it('formats sub-minute durations as seconds', () => {
    expect(formatEta(30)).toBe('~30s left');
  });

  it('formats durations under an hour as minutes', () => {
    expect(formatEta(12 * 60)).toBe('~12 min left');
  });

  it('formats multi-hour durations with minutes', () => {
    expect(formatEta(2 * 3600 + 10 * 60)).toBe('~2h 10m left');
  });

  it('omits minutes for exact hours', () => {
    expect(formatEta(2 * 3600)).toBe('~2h left');
  });

  it('returns empty string for zero, negative, or non-finite input', () => {
    expect(formatEta(0)).toBe('');
    expect(formatEta(-5)).toBe('');
    expect(formatEta(Infinity)).toBe('');
    expect(formatEta(NaN)).toBe('');
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/models/models.spec.ts`
Expected: FAIL — `formatEta` is not exported from `./models`.

- [ ] **Step 3: Write the implementation**

Add to `src/app/models/models.ts` (e.g. just below the `PipelineStats` interface):

```ts
/**
 * Human-readable estimated time to completion, compact adaptive unit:
 * "~45s left", "~12 min left", "~2h 10m left". Returns '' when unknown
 * (zero, negative, or non-finite), so callers can hide the segment.
 */
export function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '';
  if (seconds < 60) return `~${Math.round(seconds)}s left`;
  if (seconds < 3600) return `~${Math.round(seconds / 60)} min left`;
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return m > 0 ? `~${h}h ${m}m left` : `~${h}h left`;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/app/models/models.spec.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src/app/models/models.ts src/app/models/models.spec.ts
git commit -m "feat(TT-64): formatEta util for processing badge ETA"
```

---

## Task 4: Frontend — `etaSeconds` computed + hold-last-known guard

**Files:**
- Modify: `src/app/services/photo.service.ts` (constructor subscription ~121-123; add `etaSeconds` near other computeds ~115-118)
- Test: `src/app/services/photo.service.spec.ts` (add a new `describe` block)

- [ ] **Step 1: Write the failing tests**

Append a new block to `src/app/services/photo.service.spec.ts` (follow the existing file's TestBed + `TauriEventsService` mock pattern):

```ts
describe('PhotoService — processing speed resilience & ETA', () => {
  let service: PhotoService;
  let pipelineStats$: Subject<PipelineStats>;

  beforeEach(() => {
    pipelineStats$ = new Subject<PipelineStats>();
    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$,
            imageAdded$: new Subject(),
            imageUpdated$: new Subject(),
            imageRemoved$: new Subject(),
            modelDownloadProgress$: new Subject(),
          },
        },
      ],
    });
    service = TestBed.inject(PhotoService);
  });

  it('holds the last non-zero speed when a zero-speed stat arrives mid-processing', () => {
    pipelineStats$.next({ total_pending: 100, images_per_sec: 8 });
    expect(service.pipelineStats().images_per_sec).toBe(8);

    // Scanner heartbeat: refreshes the count but carries a 0 speed.
    pipelineStats$.next({ total_pending: 120, images_per_sec: 0 });
    expect(service.pipelineStats().images_per_sec).toBe(8); // held
    expect(service.pipelineStats().total_pending).toBe(120); // count still updates
  });

  it('clears the speed once processing finishes (pending 0)', () => {
    pipelineStats$.next({ total_pending: 100, images_per_sec: 8 });
    pipelineStats$.next({ total_pending: 0, images_per_sec: 0 });
    expect(service.pipelineStats().images_per_sec).toBe(0);
  });

  it('computes etaSeconds as remaining / speed', () => {
    pipelineStats$.next({ total_pending: 120, images_per_sec: 8 });
    expect(service.etaSeconds()).toBe(15);
  });

  it('returns 0 etaSeconds when speed is zero', () => {
    pipelineStats$.next({ total_pending: 0, images_per_sec: 0 });
    expect(service.etaSeconds()).toBe(0);
  });
});
```

Ensure the import line at the top of the spec includes `PipelineStats` (alongside the existing model imports): `import { ImageUpdatedEvent, PipelineStats } from '../models/models';`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/app/services/photo.service.spec.ts`
Expected: FAIL — `service.etaSeconds` is not a function, and the hold-last-known assertion fails (current code overwrites speed with 0).

- [ ] **Step 3: Implement the hold-last-known guard**

In `src/app/services/photo.service.ts`, replace the constructor subscription (currently ~lines 121-123):

```ts
    this.events.pipelineStats$.subscribe((e) => {
      this.pipelineStats.set(e);
    });
```

with:

```ts
    this.events.pipelineStats$.subscribe((e) => {
      // Hold-last-known speed: a 0 while work remains is a heartbeat without a
      // fresh sample, not a real stop — keep the prior speed (TT-64).
      const prev = this.pipelineStats();
      const images_per_sec =
        e.images_per_sec > 0 || e.total_pending === 0
          ? e.images_per_sec
          : prev.images_per_sec;
      this.pipelineStats.set({ ...e, images_per_sec });
    });
```

- [ ] **Step 4: Add the `etaSeconds` computed**

In `src/app/services/photo.service.ts`, add near the other `computed` signals (e.g. after `totalPhotoCount` ~line 118):

```ts
  /** Estimated seconds to drain the pending queue at the current speed. 0 when unknown. */
  readonly etaSeconds = computed<number>(() => {
    const s = this.pipelineStats();
    return s.images_per_sec > 0 ? s.total_pending / s.images_per_sec : 0;
  });
```

(`computed` is already imported in this file.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `npx vitest run src/app/services/photo.service.spec.ts`
Expected: PASS — the new 4 tests plus the existing suite.

- [ ] **Step 6: Commit**

```bash
git add src/app/services/photo.service.ts src/app/services/photo.service.spec.ts
git commit -m "feat(TT-64): hold-last-known speed + etaSeconds in PhotoService"
```

---

## Task 5: Frontend — render speed + ETA in the badge

No new unit test (pure template wiring); verify with a production build.

**Files:**
- Modify: `src/app/components/search-bar/search-bar.component.ts`
- Modify: `src/app/components/search-bar/search-bar.component.html:77-79`

- [ ] **Step 1: Expose `formatEta` to the template**

In `src/app/components/search-bar/search-bar.component.ts`, add the import and a protected member.

Add to the imports from models (or a new import line):

```ts
import { formatEta } from '../../models/models';
```

Inside the component class (next to the other `protected` members, ~line 30):

```ts
  protected readonly formatEta = formatEta;
```

- [ ] **Step 2: Render the ETA segment**

In `src/app/components/search-bar/search-bar.component.html`, replace the speed block (currently lines 77-79):

```html
        @if (photos.pipelineStats().images_per_sec >= 0.1) {
          · {{ photos.pipelineStats().images_per_sec | number:'1.0-1' }} img/s
        }
```

with:

```html
        @if (photos.pipelineStats().images_per_sec >= 0.1) {
          · {{ photos.pipelineStats().images_per_sec | number:'1.0-1' }} img/s
          @if (formatEta(photos.etaSeconds())) {
            · {{ formatEta(photos.etaSeconds()) }}
          }
        }
```

- [ ] **Step 3: Verify the build compiles**

Run: `npx ng build --configuration development`
Expected: Build succeeds (template type-checks `formatEta` and `photos.etaSeconds`).

- [ ] **Step 4: Run the full frontend suite**

Run: `npx vitest run`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/app/components/search-bar/search-bar.component.ts src/app/components/search-bar/search-bar.component.html
git commit -m "feat(TT-64): show inference speed + ETA in processing badge"
```

---

## Final Verification (manual)

- [ ] Run the app, add a folder with 1000+ images, and confirm during discovery the badge keeps showing `Processing N images · X img/s · ~Y left` — the speed no longer vanishes.
- [ ] Confirm the ETA shrinks toward zero as the queue drains and the badge clears when finished.

Reference: spec acceptance criteria in `docs/superpowers/specs/2026-06-11-processing-label-speed-eta-design.md`.
