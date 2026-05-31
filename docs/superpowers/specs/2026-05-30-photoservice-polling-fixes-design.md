# PhotoService Polling and Efficiency Fixes

The `PhotoService` uses a polling mechanism to refresh processing status when there are pending items in the pipeline. The current implementation has several code quality and efficiency issues that need to be addressed.

## Problem Description

1.  **Fragile Polling Stream**: The `pollingSub` observable uses `switchMap` to call `refreshProcessingStatus()`, which returns a `Promise`. If this IPC call fails, the entire stream terminates, and polling stops until `isProcessing` toggles from `false` to `true` again.
2.  **Inefficient Effect Dependencies**: The `effect` that manages polling reads the entire `pipelineStats()` signal. This causes the effect to re-run whenever `images_per_sec` changes, even if `total_pending` remains above zero. This is unnecessary because `images_per_sec` is updated by the poll itself (and via event stream), leading to redundant effect executions.
3.  **Code Bloat**: Several RxJS operators and functions (`of`, `filter`, `distinctUntilChanged`) are imported but not used in the file.

## Proposed Changes

### 1. Robust Polling with Error Handling

Wrap the `refreshProcessingStatus()` call in `from()` (from `rxjs`) and use `catchError` to handle failures. This ensures that a single failed IPC call doesn't kill the polling timer.

```typescript
this.pollingSub = timer(0, 1000).pipe(
  switchMap(() => from(this.refreshProcessingStatus()).pipe(
    catchError(err => {
      console.error('Failed to poll processing status:', err);
      return EMPTY;
    })
  ))
).subscribe();
```

### 2. Optimized Effect Dependencies

Use a `computed` signal to derive the `isProcessing` state. This ensures the `effect` only runs when the boolean state actually changes, ignoring updates to other properties of `pipelineStats`.

```typescript
const isProcessing = computed(() => this.pipelineStats().total_pending > 0);

effect(() => {
  const active = isProcessing();
  // ... rest of the logic
});
```

### 3. Cleanup Unused Imports

Remove `of`, `filter`, and `distinctUntilChanged` from the imports at the top of the file. Add `from`, `EMPTY`, and `catchError` as needed for the new implementation.

## Verification Plan

### Automated Tests
- Run `npx tsc --project tsconfig.app.json --noEmit` to ensure no type errors or missing imports.

### Manual Verification
- Observe the application during a large import.
- Check console for polling errors if they occur (simulated or real).
- Verify that polling correctly starts when `total_pending > 0` and stops when `total_pending === 0`.
