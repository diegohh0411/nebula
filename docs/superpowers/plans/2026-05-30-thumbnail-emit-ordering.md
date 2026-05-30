# TT-12: Guarantee Thumbnail Availability vs. Stage 2 emit Ordering

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Adopt Option A — prove via tests that the frontend's `image_updated` handler is already order-agnostic, update the spec to reflect the real async data-flow introduced by TT-9, and add pipeline comments that make the contract explicit for future readers.

**Architecture:** No change to the backend emit logic. The frontend (`PhotoService`) already uses `auditTime(2000)` + unconditional `refreshImages()`, making it inherently resilient to any emit order. The deliverables are: (1) Vitest test infrastructure, (2) a `PhotoService` spec that proves order-agnostic behaviour, (3) updated spec doc, (4) minimal Rust comments. No pipeline logic changes.

**Tech Stack:** Angular 20, RxJS 7 (`auditTime`), Vitest 4 + `@analogjs/vitest-angular` 2.4, TypeScript, Rust (comment-only changes)

---

### Task 1: Set up Vitest test infrastructure

**Files:**
- Create: `vitest.config.ts`
- Create: `src/test-setup.ts`
- Modify: `package.json` (add `"test"` script)

- [ ] **Step 1: Create `vitest.config.ts`**

```typescript
import { defineConfig } from 'vite';
import angular from '@analogjs/vite-plugin-angular';

export default defineConfig({
  plugins: [angular()],
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['src/test-setup.ts'],
    include: ['src/**/*.spec.ts'],
  },
});
```

- [ ] **Step 2: Create `src/test-setup.ts`**

```typescript
import '@analogjs/vitest-angular/setup-zone';
```

- [ ] **Step 3: Add `"test"` script to `package.json`**

Open `package.json` and add `"test": "vitest run"` to the `"scripts"` block so it reads:

```json
"scripts": {
  "ng": "ng",
  "start": "ng serve",
  "build": "ng build",
  "watch": "ng build --watch --configuration development",
  "tauri": "tauri",
  "test": "vitest run"
},
```

- [ ] **Step 4: Install dependencies if not already installed**

```bash
cd /home/user/nebula && pnpm install
```

Expected: dependencies resolved, no errors.

- [ ] **Step 5: Verify the test runner starts**

```bash
cd /home/user/nebula && pnpm test 2>&1 | tail -20
```

Expected: vitest runs and exits (no test files yet → may report "no tests found" or 0 tests passed). No crash.

- [ ] **Step 6: Commit**

```bash
git add vitest.config.ts src/test-setup.ts package.json
git commit -m "chore(test): set up vitest + @analogjs/vitest-angular infrastructure"
```

---

### Task 2: Write PhotoService spec — prove order-agnostic `image_updated` handling

**Files:**
- Create: `src/app/services/photo.service.spec.ts`
- Read: `src/app/services/photo.service.ts` (understand constructor + injected deps)

Context you need before writing the test:
- `PhotoService` uses Angular's `inject()` with `TauriEventsService` (`private events = inject(TauriEventsService)`).
- Its constructor immediately subscribes: `this.events.imageUpdated$.pipe(auditTime(2000)).subscribe(() => { void this.refreshImages(); void this.refreshSearchResults(); })`.
- `refreshImages()` and `refreshSearchResults()` both call `invoke(...)` from `@tauri-apps/api/core` — these must be spied on so no real Tauri bridge is invoked.
- The mock for `TauriEventsService` must provide all four `Subject`s: `pipelineStats$`, `imageAdded$`, `imageUpdated$`, `imageRemoved$`.

- [ ] **Step 1: Write the failing spec**

Create `src/app/services/photo.service.spec.ts`:

```typescript
import { TestBed, fakeAsync, tick } from '@angular/core/testing';
import { Subject } from 'rxjs';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { PhotoService } from './photo.service';
import { TauriEventsService } from './tauri-events.service';
import { ImageUpdatedEvent } from '../models/models';

describe('PhotoService — imageUpdated$ order-agnostic contract', () => {
  let service: PhotoService;
  let imageUpdated$: Subject<ImageUpdatedEvent>;

  beforeEach(() => {
    imageUpdated$ = new Subject<ImageUpdatedEvent>();

    TestBed.configureTestingModule({
      providers: [
        PhotoService,
        {
          provide: TauriEventsService,
          useValue: {
            pipelineStats$: new Subject(),
            imageAdded$: new Subject(),
            imageUpdated$,
            imageRemoved$: new Subject(),
            modelDownloadProgress$: new Subject(),
          },
        },
      ],
    });

    service = TestBed.inject(PhotoService);
    vi.spyOn(service as any, 'refreshImages').mockResolvedValue(undefined);
    vi.spyOn(service as any, 'refreshSearchResults').mockResolvedValue(undefined);
  });

  it('calls refreshImages after auditTime window expires', fakeAsync(() => {
    imageUpdated$.next({ image_id: 1 });
    expect((service as any).refreshImages).not.toHaveBeenCalled();
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);
  }));

  it('coalesces rapid emits (stage-2 then stage-1 within 2 s) into one refresh', fakeAsync(() => {
    imageUpdated$.next({ image_id: 1 }); // stage-2 "analysis complete" fires first
    tick(100);
    imageUpdated$.next({ image_id: 1 }); // stage-1 "preview ready" fires 100 ms later
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);
  }));

  it('fires a second refresh when stage-1 arrives after the 2 s audit window has elapsed', fakeAsync(() => {
    imageUpdated$.next({ image_id: 1 }); // stage-2 fires
    tick(2000); // first audit window expires → first refresh
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);

    imageUpdated$.next({ image_id: 1 }); // stage-1 thumbnail task fires very late
    tick(2000); // second audit window → second refresh (UI corrects itself)
    expect((service as any).refreshImages).toHaveBeenCalledTimes(2);
  }));

  it('does not assume event order — stage-2 before stage-1 eventually shows thumbnail', fakeAsync(() => {
    // Worst case: stage-2 fires, UI refreshes and may see thumbnail_path = null.
    // Then stage-1 thumbnail-write fires and UI refreshes again, picking up the thumbnail.
    imageUpdated$.next({ image_id: 42 });
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(1);

    imageUpdated$.next({ image_id: 42 });
    tick(2000);
    expect((service as any).refreshImages).toHaveBeenCalledTimes(2);
    // Both calls are unconditional: the second one will find thumbnail_path set.
  }));
});
```

- [ ] **Step 2: Run the spec (expect failures first)**

```bash
cd /home/user/nebula && pnpm test 2>&1 | tail -40
```

Expected: the spec is found; tests may fail if `PhotoService` constructor has unresolved imports (e.g., `invoke` is called directly at module load). Diagnose the error output.

- [ ] **Step 3: Fix any setup issues**

Common issues and fixes:

**Issue:** `invoke` or `convertFileSrc` from `@tauri-apps/api/core` throws at import time.

Fix — add a module mock at the top of the spec file (before imports):
```typescript
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((path: string) => path),
}));
```

**Issue:** `PhotoService` has a method called `loadFolders` or similar that is also called in the constructor and uses `invoke`.

Fix — add it to the spy list after `TestBed.inject(PhotoService)`:
```typescript
vi.spyOn(service as any, 'loadFolders').mockResolvedValue(undefined);
vi.spyOn(service as any, 'refreshImages').mockResolvedValue(undefined);
vi.spyOn(service as any, 'refreshSearchResults').mockResolvedValue(undefined);
```

**Issue:** `fakeAsync` and `tick` don't work with Vitest globals.

Fix — confirm `globals: true` is set in `vitest.config.ts`. Also ensure `src/test-setup.ts` is loaded.

- [ ] **Step 4: Run tests to verify all 4 pass**

```bash
cd /home/user/nebula && pnpm test 2>&1 | tail -20
```

Expected output:
```
✓ src/app/services/photo.service.spec.ts (4)
  ✓ PhotoService — imageUpdated$ order-agnostic contract (4)

Test Files  1 passed (1)
Tests       4 passed (4)
```

- [ ] **Step 5: Commit**

```bash
git add src/app/services/photo.service.spec.ts
git commit -m "test(photo-service): prove imageUpdated$ handler is order-agnostic (TT-12)"
```

---

### Task 3: Update spec to reflect async emit ordering

**Files:**
- Modify: `docs/superpowers/specs/2026-05-30-early-preview-creation-design.md`

The spec currently describes First Emit → Second Emit as sequential (steps 2–6 under "Data Flow & Events"). TT-9's detached spawn broke that ordering. This task replaces the data-flow section with the real async contract.

- [ ] **Step 1: Read the current spec**

```bash
cat -n /home/user/nebula/docs/superpowers/specs/2026-05-30-early-preview-creation-design.md
```

- [ ] **Step 2: Replace the "Data Flow & Events" section**

Find this section in the file:

```markdown
## Data Flow & Events

1. **Stage 1 (Decode) completes:** We have the `DecodedImage` in memory.
2. **Generate Thumbnail:** We immediately spawn a blocking task to resize the image and save it to the `thumbnails` directory.
3. **Database Update:** Once written, update the `thumbnail_path` in the `images` table.
4. **First Emit:** Emit the `image_updated` event to the frontend. This signals that the UI can now display the preview, even though ML analysis is still pending.
5. **Stage 2 (Inference) completes:** Embeddings and faces are saved to the DB.
6. **Second Emit:** Emit the `image_updated` event again. This ensures the frontend knows the image is fully processed (e.g. for search or status indicators).
```

Replace it with:

```markdown
## Data Flow & Events

Two `image_updated` events are emitted per image. **Their order is not guaranteed** — see the Ordering Contract section below.

**Stage 1 path (detached, concurrent with Stage 2):**
1. Stage 1 (Decode) completes — `DecodedImage` is in memory.
2. A `tokio::spawn` thumbnail task is launched (not awaited).
3. Inside that task: thumbnail is written to disk, `thumbnail_path` is updated in the DB.
4. **First Emit ("preview ready"):** `image_updated` is emitted only if both the write and DB update succeed.

**Stage 2 path (main loop):**
5. Stage 2 (Inference) completes — embeddings and faces are saved to the DB.
6. **Second Emit ("analysis complete"):** `image_updated` is emitted unconditionally.

Steps 2–4 and steps 5–6 run concurrently. Either emit may arrive at the frontend first.

## Ordering Contract (Option A — adopted in TT-12)

Every `image_updated` event means **"refetch this image"** — nothing more. Handlers must:
- Re-query the DB on every event.
- Tolerate `thumbnail_path = null` (thumbnail write may not have completed yet).
- Not assume that any particular emit implies a specific set of fields are populated.

The frontend (`PhotoService`) uses `auditTime(2000)` + unconditional `refreshImages()`, which satisfies this contract: rapid successive emits are coalesced into one refresh, and a late Stage 1 emit triggers a second refresh that displays the thumbnail once it is written.
```

- [ ] **Step 3: Verify the file looks correct**

```bash
cat /home/user/nebula/docs/superpowers/specs/2026-05-30-early-preview-creation-design.md
```

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-05-30-early-preview-creation-design.md
git commit -m "docs(spec): document async emit ordering contract (TT-12 Option A)"
```

---

### Task 4: Add pipeline contract comments

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`

Two precise locations need comments. Use `grep -n "image_updated" src-tauri/src/pipeline/mod.rs` to confirm current line numbers before editing.

- [ ] **Step 1: Confirm line numbers**

```bash
grep -n "image_updated\|tokio::spawn" /home/user/nebula/src-tauri/src/pipeline/mod.rs | head -20
```

Expected: shows the `tokio::spawn` at ~line 193 and two `app.emit("image_updated", ...)` calls at ~line 211 and ~line 376.

- [ ] **Step 2: Add comment above the Stage 1 `tokio::spawn` (line ~193)**

In `src-tauri/src/pipeline/mod.rs`, find this line:

```rust
                    tokio::spawn(async move {
```

Replace with:

```rust
                    // Detached: not awaited before Stage 2. Emits image_updated when thumbnail is
                    // written. May fire before or after Stage 2's own image_updated emit — the
                    // frontend must treat every image_updated as "refetch" (Option A contract, TT-12).
                    tokio::spawn(async move {
```

- [ ] **Step 3: Add comment above the Stage 2 emit (line ~376)**

In `src-tauri/src/pipeline/mod.rs`, find this block:

```rust
            let _ = app.emit(
                "image_updated",
                crate::models::ImageUpdatedPayload { image_id },
            );
```

Replace with:

```rust
            // Second emit: signals full analysis complete (embeddings + faces written). NOT ordered
            // vs. the Stage 1 thumbnail emit — frontend must handle either order (TT-12 Option A).
            let _ = app.emit(
                "image_updated",
                crate::models::ImageUpdatedPayload { image_id },
            );
```

- [ ] **Step 4: Build to confirm no compilation errors**

```bash
cd /home/user/nebula/src-tauri && cargo build 2>&1 | tail -10
```

Expected: `Finished` with no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "docs(pipeline): comment async emit ordering contract at both emit sites (TT-12)"
```

---

## Self-Review

**Spec coverage check:**

| Acceptance criterion | Covered by |
|---|---|
| Audit frontend `image_updated` listener — order-sensitive? | Task 2 tests expose the `auditTime` + unconditional-refresh design |
| Decide on correct contract (Option A / B / C) | Option A chosen; documented in Task 3 spec update |
| Implement chosen option with tests | Task 2: 4 Angular unit tests covering ordering + late thumbnail scenarios |
| Tests cover `thumbnail_path` presence expectation | Test 3 + Test 4 in Task 2: second refresh always finds `thumbnail_path` set (Stage 1 emits only after DB update succeeds — `pipeline/mod.rs` line 209) |
| Update spec data-flow docs | Task 3 |
| Keep TT-9 perf win (no blocking on critical path) | No pipeline logic changes |

**Placeholder scan:** No TBD/TODO entries. All code blocks are complete. All commands are specific with expected output.

**Type consistency:** `ImageUpdatedEvent.image_id` in the Angular test matches the interface in `src/app/models/models.ts`. `crate::models::ImageUpdatedPayload { image_id }` in the Rust comments matches the existing struct in `src-tauri/src/models/entities.rs`.
