# PhotoService Polling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve robustness and efficiency of `PhotoService` polling by adding error handling and optimizing Angular Signal dependencies.

**Architecture:** 
1. Derive polling state using a `computed` signal to minimize `effect` re-runs.
2. Wrap polling IPC calls in an RxJS stream with `catchError` to prevent subscription termination.
3. Clean up unused RxJS imports.

**Tech Stack:** Angular Signals, RxJS, Tauri IPC.

---

### Task 1: Update Imports

**Files:**
- Modify: `src/app/services/photo.service.ts`

- [x] **Step 1: Clean up and add necessary RxJS imports**

Modify the imports to remove `of`, `filter`, `distinctUntilChanged` and add `from`, `EMPTY`, `catchError`.

```typescript
import { timer, from, EMPTY, Subscription } from 'rxjs';
import { auditTime, switchMap, catchError } from 'rxjs/operators';
```

### Task 2: Optimize and Robustify Polling

**Files:**
- Modify: `src/app/services/photo.service.ts`

- [x] **Step 1: Update the polling logic in constructor**

```typescript
    // TT-7/TT-14: Freshness & Granularity Poll
    // Starts polling when pending > 0, stops when 0.
    const isProcessing = computed(() => this.pipelineStats().total_pending > 0);

    effect(() => {
      const active = isProcessing();

      if (active && !this.pollingSub) {
        this.pollingSub = timer(0, 1000).pipe(
          switchMap(() => from(this.refreshProcessingStatus()).pipe(
            catchError(err => {
              console.error('Failed to poll processing status:', err);
              return EMPTY;
            })
          ))
        ).subscribe();
      } else if (!active && this.pollingSub) {
        this.pollingSub.unsubscribe();
        this.pollingSub = undefined;
      }
    });
```

### Task 3: Verification

- [x] **Step 1: Run Type Check**

Run: `npx tsc --project tsconfig.app.json --noEmit`
Expected: Success (No output or exit code 0)

- [x] **Step 2: Commit Changes**

```bash
git add src/app/services/photo.service.ts
git commit -m "refactor(ui): improve polling robustness and efficiency in PhotoService"
```
