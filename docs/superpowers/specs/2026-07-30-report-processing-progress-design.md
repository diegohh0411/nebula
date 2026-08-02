# Report processing-progress bar — design

Date: 2026-07-30
Status: approved (interactive session; user requested immediate implementation)

## Problem

A report's coverage is only trustworthy once the pipeline has fully processed the
images in its source folders. Today there is no way to see how far along that is —
the user prioritizes the report's folders and then stares at the global queue
counter. The report detail page needs a progress bar showing the percentage of
the report's images that are fully processed.

## Definition of "fully processed"

`images.semantic_analysis_done = 1 AND images.subject_analysis_done = 1 AND
deleted_at IS NULL` — the same predicate `pipeline::queue::get_processing_counts`
uses globally, scoped to the report's folders.

## Backend (`reports` slice)

- `reports/models.rs`: `ProcessingProgress { total: i64, done: i64 }`
  (serde-serializable).
- `reports/repo.rs`: `get_folders_processing_progress(pool, folder_ids) ->
  Result<ProcessingProgress>` — one query over `images` with
  `folder_id IN (…) AND deleted_at IS NULL`; `done` counts rows where both
  analysis flags are 1. Empty `folder_ids` returns `{0, 0}`.
- `reports/commands.rs`: `get_report_processing_progress(reportId) ->
  ProcessingProgress` — resolves the report's `folder_ids` (404s like
  `prioritize_report_processing`), delegates to the repo fn. Registered in
  `app/mod.rs` at its definition site.

## Frontend (Angular)

- `models.ts`: `ProcessingProgress { total: number; done: number }`.
- `photo.service.ts`: `getReportProcessingProgress(reportId): Promise<ProcessingProgress>`.
- `report-detail.component.ts`: `progress` signal fetched on init; an `effect`
  on `photos.pipelineStats()` re-fetches while `total_pending > 0`, so the bar
  advances live off the existing `pipeline_stats` event and stops refreshing
  when the pipeline goes idle.
- Template: slim progress bar in the summary block near the folder names /
  Prioritize button, labelled "N of M images processed (P%)". Full bar at 100%
  with the same label — no special empty/complete states beyond styling.
  Hidden while `progress` is null or `total` is 0.

Rejected alternative: a new folder-scoped progress event emitted from the
pipeline loop — more plumbing for no visible gain on one page; the existing
stats event already ticks at a sensible cadence.

## Testing

- Repo test: multi-folder counts, deleted images excluded, partial-flag images
  not counted as done, empty folder list.
- Component spec: percentage rendering; re-fetch triggered by a stats tick and
  suppressed when idle.
