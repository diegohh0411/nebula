# Multi-folder reports + queue ETA reduction — design

Date: 2026-07-30
Status: approved (interactive session; PR-only workflow, Notion process explicitly skipped by user)

## Problem

1. Saved reports target exactly one folder (`saved_reports.folder_id`). The user needs a
   coverage report over ~300 images spread across 5 folders.
2. The inference queue (~1000 images) is processed strictly by `scheduled_at` with no
   prioritization, and the pipeline loop carries avoidable per-batch overhead. The report
   is needed hours from now; the ETA must drop.

Scope decisions from the session: both workstreams; **safe throughput fixes only** (no
parallel face workers, no GPU/DirectML placement tonight); prioritization is triggered by
a button on the report detail page.

## 1. Multi-source-folder reports

Chosen approach: a `saved_report_folders` junction table mirroring `saved_report_tags`.
(Rejected: CSV column — unqueryable, no FK cascade; nullable `folder_id` + optional
junction — two sources of truth.)

- **Migration 7**:
  1. `CREATE TABLE saved_report_folders (report_id REFERENCES saved_reports(id) ON DELETE CASCADE, folder_id REFERENCES folders(id) ON DELETE CASCADE, PRIMARY KEY (report_id, folder_id))`
  2. Backfill: `INSERT INTO saved_report_folders SELECT id, folder_id FROM saved_reports`
  3. Rebuild `saved_reports` without `folder_id` (table-rebuild pattern of migration 5).
  `BASE_SCHEMA` updated to match.
- **Models**: `SavedReport.folder_id: i64` → `folder_ids: Vec<i64>`.
- **Repo** (`reports/repo.rs`):
  - `create_saved_report(pool, name, folder_ids, tag_ids)` — dedupe, reject empty
    `folder_ids` (same style as name validation).
  - `folders_by_report_id` helper cloned from `tags_by_report_id`; `list`/`get` hydrate it.
  - `get_folder_coverage(pool, folder_ids, tag_ids)`: `WHERE i.folder_id IN (…)`.
    Frequency = COUNT(DISTINCT image) across the union of the folders.
- **Commands**: `get_folder_coverage(folderIds: number[], …)`,
  `create_saved_report(name, folderIds, tagIds)`.

## 2. Queue prioritization

- **Migration 8**: `ALTER TABLE embedding_queue ADD COLUMN priority INTEGER NOT NULL DEFAULT 0`
  (+ `BASE_SCHEMA`).
- `get_queue_batch`: `ORDER BY priority DESC, scheduled_at ASC`.
- `queue::prioritize_folders(pool, folder_ids) -> u64`: one statement,
  `UPDATE embedding_queue SET priority = (SELECT COALESCE(MAX(priority),0)+1 FROM embedding_queue) WHERE image_id IN (SELECT id FROM images WHERE folder_id IN (…))`;
  returns affected-row count. Max+1 ⇒ the latest prioritization wins.
- Command `prioritize_report_processing(reportId) -> u64`: resolve the report's folders,
  call `prioritize_folders`.

## 3. Safe throughput fixes (`pipeline/mod.rs` loop)

1. **Amortize index-snapshot saves.** Currently the whole vector index is written to
   `nebula.idx` every batch. New policy: save at most once per 60 s while processing, and
   once when the loop goes idle. Precondition (verify at implementation time): the
   startup path can rebuild/absorb rows whose embeddings are in SQLite but missing from
   the snapshot. If it cannot self-heal, keep per-batch saves and drop this item.
2. **Amortize clustering relabel.** Keep `update_edges_incremental` per batch. Run
   `relabel_from_edges` + `upgrade_thumbnails_and_emit` at most once per 5 batches or
   30 s (whichever comes first), and always flush before entering the idle branch, so the
   idle sweep and the People view never see stale labels for long. Extract the timing
   policy into a small testable struct (pattern: `ThroughputWindow`).
3. **Prefetch next batch.** After Phase A dispatches batch N to the actors, spawn a task
   that pulls batch N+1 from the queue **excluding batch N's in-flight queue ids**
   (`WHERE id NOT IN (…)`, ≤ 24 ids) and decodes it, overlapping with Phase B awaits.
   Next iteration consumes the prefetched batch. Empty prefetch falls through to the
   existing idle branch. One batch of priority-staleness is accepted.

Explicitly out of scope tonight: multiple face actors, GPU placement, batch-size tuning.

## 4. UI (Angular)

- `models.ts`: `SavedReport.folder_ids: number[]`.
- **Reports creation form**: folder multi-select styled like the existing tag
  multi-select; ≥ 1 folder required. Cards show "N folders" when > 1.
- **Report detail**: show all source folder names; **"Prioritize processing"** button →
  `prioritize_report_processing`, confirm with "Moved N images to the front of the queue".
- `photo.service.ts`: update signatures, add `prioritizeReportProcessing(reportId)`.

## Testing

- Reports repo: multi-folder coverage union/frequency, CRUD round-trip, empty-folder
  rejection, migration backfill (fresh pool runs full migration chain).
- Queue: priority ordering in `get_queue_batch`, `prioritize_folders` bump + max+1
  semantics, exclusion list in the prefetch pull.
- Pipeline: timing-policy struct unit tests; existing suite stays green.

## Expected impact

Prioritization: report becomes available after ~300 images instead of ~1000 (~70 %
sooner). Loop fixes remove per-batch snapshot writes, whole-graph relabels, and decode
stalls from the critical path — the three largest non-inference costs.
