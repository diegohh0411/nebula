# Subject Merge Suggestions Design

A system for detecting and resolving duplicate subject clusters, addressing cases where HDBSCAN splits the same person (e.g., child vs. adult) into separate subjects.

## Problem

HDBSCAN clusters faces purely by embedding similarity. A person's face at age 5 and age 30 produces different embeddings, so the algorithm legitimately creates two separate clusters. The online incremental matcher sometimes bridges them (resulting in one cluster with mixed ages), but inconsistently — producing two subjects for the same person.

## Architecture

### Cross-Pair Linking Algorithm

After each `recluster_all()`, run a merge suggestion pass:

1. Load all subjects with their face embeddings
2. For each pair of subjects (A, B):
   - Compute pairwise cosine similarity between every face in A and every face in B
   - Count cross-pairs exceeding a threshold of **0.35** (below the main 0.4 clustering threshold, to catch weakened similarity across age)
   - If `cross_match_count >= 2` AND `cross_match_count / total_pairs >= 20%`, record a merge suggestion
3. Clear old suggestions and store new ones in `merge_suggestions` table

> **TODO(perf):** The cross-pair analysis should be throttled to run at most once every 12-24 hours rather than after every recluster batch. For now it runs every time since the dataset is small, but as the face count grows this will become expensive (O(n*m) per subject pair). Consider a `last_merge_scan_at` timestamp check in `recluster_all` or a dedicated periodic task.

### Name-Based Merge Detection

When a user names a subject with a name that already exists (exact match, case-insensitive, trimmed), the backend returns the conflicting subject's ID. The frontend shows a "Merge?" dialog. This catches cases the algorithm misses.

### How Merges Self-Reinforce

HDBSCAN ignores subject IDs during clustering — it only looks at raw embeddings. After a merge (reassign faces from B to A, delete B), the next recluster will:
1. Produce the same HDBSCAN clusters (same embedding geometry)
2. Both clusters now find subject A as the majority subject ID via the vote in `clustering.rs:63-68`
3. Everything stays merged — no re-splitting

## Data Model

### New Table: `merge_suggestions`

```sql
CREATE TABLE IF NOT EXISTS merge_suggestions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    cross_match_count INTEGER NOT NULL,
    total_pairs INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
```

### Merge Suggestion Response Model

```typescript
interface MergeSuggestion {
  id: number;
  subjectA: Subject;
  subjectB: Subject;
  crossMatchCount: number;
  totalPairs: number;
}
```

## Backend Changes

### New DB Functions (`db.rs`)

- `clear_merge_suggestions(pool)` — wipe all suggestions (before regenerating)
- `insert_merge_suggestion(pool, a, b, cross_matches, total_pairs)` — insert one suggestion
- `get_merge_suggestions(pool)` → `Vec<MergeSuggestion>` — join with subjects table to return full details
- `merge_subjects(pool, target_id, source_id)` — reassign all faces from source to target, delete source, remove all suggestions referencing either subject, auto-assign thumbnail if needed
- `dismiss_merge_suggestion(pool, id)` — delete the suggestion row
- `find_subject_by_name(pool, name, exclude_id)` → `Option<Subject>` — exact match, case-insensitive

### New Tauri Commands (`commands.rs`)

- `get_merge_suggestions` → returns `Vec<MergeSuggestion>`
- `merge_subjects(target_id: i64, source_id: i64)` → executes merge
- `dismiss_merge_suggestion(id: i64)` → removes suggestion
- Modify `name_subject` return type to include `duplicate_subject_id: Option<i64>`

### Merge Suggestion Engine (`clustering.rs`)

New function `find_merge_suggestions(pool)` called at the end of `recluster_all`:

```rust
pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    // TODO(perf): Throttle to once per 12-24 hours
    // 1. Load all subjects and their face embeddings
    // 2. For each pair, compute cross-pair similarities
    // 3. Apply thresholds (>= 2 matches, >= 20% of pairs)
    // 4. Clear old suggestions, insert new ones
}
```

## Frontend Changes

### People View (`people-view.component`)

Layout order:
1. Title bar with "People & Subjects" heading + Re-cluster button (unchanged)
2. **Possible Duplicates** section (shown only when suggestions exist)
   - Card/list showing each suggested pair with thumbnails, names, and match stats
   - "Merge" button → calls `merge_subjects`, reloads subjects and suggestions
   - "Dismiss" button → calls `dismiss_merge_suggestion`, removes from list
3. Subject grid (unchanged, but now sorted with named subjects first)

### Subject Detail (`subject-detail.component`)

- **Similar Subjects card** at the bottom of the page showing merge suggestions involving this subject
- **Name conflict dialog**: when saving a name that matches another subject, show "A subject named 'X' already exists. Merge them?" with Confirm/Cancel
  - On confirm: call `merge_subjects`, navigate to surviving subject
  - On cancel: save the name anyway (user chose to keep separate)

### PhotoService (`photo.service.ts`)

New methods:
- `getMergeSuggestions(): Promise<MergeSuggestion[]>`
- `mergeSubjects(targetId: number, sourceId: number): Promise<void>`
- `dismissMergeSuggestion(id: number): Promise<void>`

### Named-First Sorting

Change `list_all_subjects()` query in `db.rs` to:

```sql
ORDER BY CASE WHEN name IS NOT NULL THEN 0 ELSE 1 END, added_at DESC
```

## Thresholds

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Cross-pair similarity threshold | 0.35 | Below main 0.4 threshold to catch age-related drift |
| Minimum cross-match count | 2 | Avoid merging on a single coincidence |
| Minimum cross-match ratio | 20% | Ensure meaningful overlap, not just outliers |

These are initial values that can be tuned based on real-world results.

## Success Criteria

- Duplicate subjects (child/adult split) are detected and suggested for merge
- Users can merge with one click or dismiss false suggestions
- Naming a subject with an existing name triggers a merge prompt
- Named subjects appear first in the People view
- Merges persist across recluster runs (self-reinforcing via subject ID vote)
