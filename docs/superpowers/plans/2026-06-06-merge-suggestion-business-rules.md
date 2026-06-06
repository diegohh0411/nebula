# Merge-Suggestion Business Rules Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce two business rules on merge suggestions: (1) never generate unnamed↔unnamed pairs, (2) cap the People page at the top 3 suggestions per load.

**Architecture:** Rule 1 is enforced at generation time in `find_merge_suggestions` by fetching a `HashMap<i64, bool>` of named flags and skipping pairs where both are unnamed. Rule 2 is enforced at read time by adding `limit: Option<i64>` to `get_merge_suggestions` (db + command), so the People view passes `3` while the subject-detail view passes `None` (unlimited). `ORDER BY score DESC, id ASC` makes results deterministic.

**Tech Stack:** Rust/SQLx (backend), Angular/TypeScript (frontend), Tauri (IPC bridge), SQLite

---

## File Map

| File | Change |
|---|---|
| `src-tauri/src/db.rs` | Add `get_subject_named_flags`; add `limit` + ordering to `get_merge_suggestions` |
| `src-tauri/src/clustering.rs` | Fetch named flags; skip unnamed↔unnamed pairs in the O(n²) loop |
| `src-tauri/src/commands.rs` | Accept `limit: Option<i64>` and pass to db |
| `src/app/services/photo.service.ts` | Accept optional `limit?: number` in `getMergeSuggestions` |
| `src/app/components/people-view/people-view.component.ts` | Pass `3` to `getMergeSuggestions` |

`subject-detail.component.ts` needs **no change** — it calls `getMergeSuggestions()` without a limit, which correctly returns all suggestions so the component can filter by subject ID client-side.

---

## Task 1: DB helper — `get_subject_named_flags`

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Write the failing test**

Add inside the existing `#[cfg(test)]` block at the bottom of `db.rs`:

```rust
#[tokio::test]
async fn get_subject_named_flags_returns_true_for_named_and_false_for_unnamed() {
    let pool = make_pool().await;
    let named_id: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let unnamed_id: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES (NULL, 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();

    let flags = get_subject_named_flags(&pool).await.unwrap();

    assert_eq!(flags.get(&named_id), Some(&true));
    assert_eq!(flags.get(&unnamed_id), Some(&false));
}
```

- [ ] **Step 2: Run test to see it fail**

```bash
cd src-tauri && cargo test get_subject_named_flags 2>&1 | tail -10
```

Expected: compile error — `get_subject_named_flags` not found.

- [ ] **Step 3: Add `get_subject_named_flags` to db.rs**

Add after the `get_merge_suggestions` function (after line 1032):

```rust
pub async fn get_subject_named_flags(pool: &SqlitePool) -> Result<std::collections::HashMap<i64, bool>> {
    let rows = sqlx::query("SELECT id, (name IS NOT NULL) as has_name FROM subjects")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<bool, _>("has_name")))
        .collect())
}
```

- [ ] **Step 4: Run test to see it pass**

```bash
cd src-tauri && cargo test get_subject_named_flags 2>&1 | tail -5
```

Expected: `test db::tests::get_subject_named_flags_returns_true_for_named_and_false_for_unnamed ... ok`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(TT-23): add get_subject_named_flags db helper"
```

---

## Task 2: Filter unnamed↔unnamed pairs in `find_merge_suggestions`

**Files:**
- Modify: `src-tauri/src/clustering.rs`

- [ ] **Step 1: Write the filter logic test**

Add inside the `#[cfg(test)]` block in `clustering.rs`:

```rust
#[test]
fn unnamed_unnamed_pair_is_skipped() {
    let mut named_flags = std::collections::HashMap::new();
    named_flags.insert(1i64, true);   // Alice — named
    named_flags.insert(2i64, false);  // unnamed
    named_flags.insert(3i64, false);  // unnamed

    let is_unnamed_pair = |a: i64, b: i64| -> bool {
        !named_flags.get(&a).copied().unwrap_or(false)
            && !named_flags.get(&b).copied().unwrap_or(false)
    };

    assert!(!is_unnamed_pair(1, 2), "named+unnamed should not be skipped");
    assert!(!is_unnamed_pair(1, 1), "named+named should not be skipped");
    assert!(is_unnamed_pair(2, 3),  "unnamed+unnamed must be skipped");
}
```

- [ ] **Step 2: Run test to see it pass (logic-only, no impl change needed)**

```bash
cd src-tauri && cargo test unnamed_unnamed_pair_is_skipped 2>&1 | tail -5
```

Expected: `test clustering::tests::unnamed_unnamed_pair_is_skipped ... ok`

- [ ] **Step 3: Integrate filter into `find_merge_suggestions`**

Replace the entire body of `find_merge_suggestions` (lines 105-145 in `clustering.rs`):

```rust
pub async fn find_merge_suggestions(pool: &SqlitePool) -> Result<()> {
    crate::db::clear_merge_suggestions(pool).await?;

    let named_flags = crate::db::get_subject_named_flags(pool).await?;

    let manual_raw = db::get_manual_face_embeddings_by_subject(pool).await?;
    let manual_decoded: Vec<(i64, Vec<f32>)> = manual_raw
        .into_iter()
        .filter_map(|(sid, blob)| crate::embedder::bytes_to_f32_vec(&blob).ok().map(|e| (sid, e)))
        .collect();

    let all_raw = db::get_subject_embeddings(pool).await?;
    let all_decoded: Vec<(i64, Vec<f32>)> = all_raw
        .into_iter()
        .filter_map(|(sid, blob)| crate::embedder::bytes_to_f32_vec(&blob).ok().map(|e| (sid, e)))
        .collect();

    let anchor_centroids = compute_anchor_centroids(&manual_decoded, &all_decoded);

    let mut subject_embeddings: Vec<(i64, Vec<f32>)> = anchor_centroids.into_iter().collect();
    subject_embeddings.sort_unstable_by_key(|(id, _)| *id);

    for i in 0..subject_embeddings.len() {
        for j in (i + 1)..subject_embeddings.len() {
            let (id_a, emb_a) = &subject_embeddings[i];
            let (id_b, emb_b) = &subject_embeddings[j];

            let a_named = named_flags.get(id_a).copied().unwrap_or(false);
            let b_named = named_flags.get(id_b).copied().unwrap_or(false);
            if !a_named && !b_named {
                continue;
            }

            let sim = crate::embedder::cosine_similarity(emb_a, emb_b);
            if sim > MERGE_CENTROID_SIMILARITY_THRESHOLD {
                crate::db::insert_merge_suggestion(pool, *id_a, *id_b, sim as f64).await?;
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Verify compilation**

```bash
cd src-tauri && cargo build 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 5: Run all clustering tests**

```bash
cd src-tauri && cargo test clustering 2>&1 | tail -10
```

Expected: all clustering tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/clustering.rs
git commit -m "feat(TT-23): skip unnamed↔unnamed pairs in find_merge_suggestions"
```

---

## Task 3: Add `limit` + ordering to `get_merge_suggestions`

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Write the failing test**

Add a `make_merge_pool` helper and the cap test inside the `#[cfg(test)]` block in `db.rs`, after `make_pool`:

```rust
async fn make_merge_pool() -> SqlitePool {
    let pool = make_pool().await;
    sqlx::query(
        "CREATE TABLE merge_suggestions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            subject_id_a INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            subject_id_b INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
            score REAL NOT NULL,
            created_at INTEGER NOT NULL
        )"
    ).execute(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn get_merge_suggestions_with_limit_returns_top_n_by_score() {
    let pool = make_merge_pool().await;

    let a: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let b: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Bob', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let c: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Carol', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let d: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Dave', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let e: i64 = sqlx::query_scalar(
        "INSERT INTO subjects (name, type, added_at) VALUES ('Eve', 'person', 0) RETURNING id"
    ).fetch_one(&pool).await.unwrap();

    for (sa, sb, score) in [
        (a, b, 0.95f64),
        (b, c, 0.90),
        (c, d, 0.80),
        (d, e, 0.70),
        (a, e, 0.60),
    ] {
        sqlx::query(
            "INSERT INTO merge_suggestions (subject_id_a, subject_id_b, score, created_at) VALUES (?, ?, ?, 0)"
        ).bind(sa).bind(sb).bind(score).execute(&pool).await.unwrap();
    }

    let top3 = get_merge_suggestions(&pool, Some(3)).await.unwrap();
    assert_eq!(top3.len(), 3);
    assert!((top3[0].score - 0.95).abs() < 1e-9, "first should be highest score");
    assert!((top3[1].score - 0.90).abs() < 1e-9);
    assert!((top3[2].score - 0.80).abs() < 1e-9);

    let all = get_merge_suggestions(&pool, None).await.unwrap();
    assert_eq!(all.len(), 5, "no limit should return all 5");
    assert!((all[0].score - 0.95).abs() < 1e-9, "first should still be highest score");
}
```

- [ ] **Step 2: Run test to see it fail**

```bash
cd src-tauri && cargo test get_merge_suggestions_with_limit 2>&1 | tail -10
```

Expected: compile error — `get_merge_suggestions` takes wrong number of arguments.

- [ ] **Step 3: Update `get_merge_suggestions` in db.rs**

Replace the existing function (lines 999-1032) with:

```rust
pub async fn get_merge_suggestions(pool: &SqlitePool, limit: Option<i64>) -> Result<Vec<crate::models::MergeSuggestion>> {
    let rows = match limit {
        Some(n) => {
            sqlx::query(
                r#"SELECT ms.id, ms.score,
                          sa.id as sa_id, sa.name as sa_name, sa.thumbnail_face_id as sa_thumbnail_face_id, sa.type as sa_type, sa.added_at as sa_added_at,
                          sb.id as sb_id, sb.name as sb_name, sb.thumbnail_face_id as sb_thumbnail_face_id, sb.type as sb_type, sb.added_at as sb_added_at
                   FROM merge_suggestions ms
                   JOIN subjects sa ON ms.subject_id_a = sa.id
                   JOIN subjects sb ON ms.subject_id_b = sb.id
                   ORDER BY ms.score DESC, ms.id ASC
                   LIMIT ?"#
            )
            .bind(n)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query(
                r#"SELECT ms.id, ms.score,
                          sa.id as sa_id, sa.name as sa_name, sa.thumbnail_face_id as sa_thumbnail_face_id, sa.type as sa_type, sa.added_at as sa_added_at,
                          sb.id as sb_id, sb.name as sb_name, sb.thumbnail_face_id as sb_thumbnail_face_id, sb.type as sb_type, sb.added_at as sb_added_at
                   FROM merge_suggestions ms
                   JOIN subjects sa ON ms.subject_id_a = sa.id
                   JOIN subjects sb ON ms.subject_id_b = sb.id
                   ORDER BY ms.score DESC, ms.id ASC"#
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|r| crate::models::MergeSuggestion {
            id: r.get("id"),
            subject_a: crate::models::Subject {
                id: r.get("sa_id"),
                name: r.get("sa_name"),
                thumbnail_face_id: r.get("sa_thumbnail_face_id"),
                subject_type: r.get("sa_type"),
                added_at: r.get("sa_added_at"),
            },
            subject_b: crate::models::Subject {
                id: r.get("sb_id"),
                name: r.get("sb_name"),
                thumbnail_face_id: r.get("sb_thumbnail_face_id"),
                subject_type: r.get("sb_type"),
                added_at: r.get("sb_added_at"),
            },
            score: r.get("score"),
        })
        .collect())
}
```

- [ ] **Step 4: Fix the commands.rs caller to compile**

`commands.rs` calls `db::get_merge_suggestions(&state.pool)` — it will now fail to compile. Temporarily pass `None` so the build compiles (this is replaced in Task 4):

In `commands.rs` line 332, change to:
```rust
db::get_merge_suggestions(&state.pool, None)
```

- [ ] **Step 5: Run the new test**

```bash
cd src-tauri && cargo test get_merge_suggestions_with_limit 2>&1 | tail -10
```

Expected: `test db::tests::get_merge_suggestions_with_limit_returns_top_n_by_score ... ok`

- [ ] **Step 6: Run all db tests**

```bash
cd src-tauri && cargo test db 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/db.rs
git commit -m "feat(TT-23): add limit+ordering to get_merge_suggestions"
```

---

## Task 4: Add `limit` to Tauri command

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Update command signature**

Replace lines 328-335 with:

```rust
#[tauri::command]
pub async fn get_merge_suggestions(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<MergeSuggestion>, String> {
    db::get_merge_suggestions(&state.pool, limit)
        .await
        .map_err(map_err)
}
```

- [ ] **Step 2: Verify build**

```bash
cd src-tauri && cargo build 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat(TT-23): pass limit through get_merge_suggestions Tauri command"
```

---

## Task 5: Update frontend

**Files:**
- Modify: `src/app/services/photo.service.ts`
- Modify: `src/app/components/people-view/people-view.component.ts`

- [ ] **Step 1: Update `getMergeSuggestions` in photo.service.ts**

Replace lines 317-319:

```typescript
async getMergeSuggestions(limit?: number): Promise<MergeSuggestion[]> {
    return await invoke<MergeSuggestion[]>('get_merge_suggestions', { limit: limit ?? null });
}
```

- [ ] **Step 2: Pass `3` in people-view.component.ts**

In `people-view.component.ts` inside `loadMergeSuggestions` (line 28), change:

```typescript
const suggestions = await this.photoService.getMergeSuggestions(3);
```

- [ ] **Step 3: Verify TypeScript compiles**

```bash
cd /home/pi/nebula && npx ng build --configuration development 2>&1 | grep -E "^Error|error TS" | head -20
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/app/services/photo.service.ts src/app/components/people-view/people-view.component.ts
git commit -m "feat(TT-23): cap People page to top-3 merge suggestions; subject-detail stays unlimited"
```

---

## Task 6: Final verification

- [ ] **Step 1: Run all Rust tests**

```bash
cd src-tauri && cargo test 2>&1 | tail -20
```

Expected: all tests pass. Confirm these three are present and green:
- `clustering::tests::unnamed_unnamed_pair_is_skipped`
- `db::tests::get_subject_named_flags_returns_true_for_named_and_false_for_unnamed`
- `db::tests::get_merge_suggestions_with_limit_returns_top_n_by_score`

- [ ] **Step 2: Full Angular build**

```bash
cd /home/pi/nebula && npx ng build --configuration development 2>&1 | tail -10
```

Expected: build succeeds.

- [ ] **Step 3: Acceptance criteria sign-off**

- [ ] No merge suggestion generated between two unnamed subjects
- [ ] People page shows at most 3 suggestions, highest score first
- [ ] Subject-detail page still shows suggestions for the open subject (not starved)
- [ ] `cargo test` passes with all new and existing tests
