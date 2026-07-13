# Dismissed Merge Suggestions Reappear — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop dismissed "Not the same person" merge suggestions from reappearing
after idle reclustering. Two backend-only changes: a subject-scoped `cannot_link`
guard in `find_merge_suggestions`, and a hardened multi-face representative-face
write in `dismiss_merge_suggestion`. No schema migration, no frontend changes.

**Design reference:** `docs/superpowers/specs/2026-07-13-dismissed-merge-suggestions-fix-design.md`

**Architecture:** Both changes live in the `people` slice per this repo's
vertical-slice CLAUDE.md convention — `find_merge_suggestions` in
`src-tauri/src/people/clustering.rs` (business logic that queries via `sqlx`
directly, consistent with how that function already works) and
`dismiss_merge_suggestion` in `src-tauri/src/people/repo.rs`. No new files.

**Tech stack:** Rust, `sqlx` (SQLite), `tokio::test` for async tests, existing
in-memory-SQLite test-pool helpers (`make_integration_pool` in clustering.rs).

**Review note:** this plan (and its companion spec) was sent for an independent
adversarial review (Fable) after the first draft. That review caught a blocking
defect in Task 1's original Test A fixture — the candidate `face_edges` row was
placed on the exact already-constrained face pair, which the pre-existing per-edge
`NOT EXISTS` check suppresses regardless of this fix, so the test would never have
failed pre-fix and wouldn't have exercised Change 1 at all. Task 1 Step 1 below is
already corrected to require a second, unconstrained face on the churned side. Task 2
Step 1 is corrected similarly (S4: the original fixture didn't force the
thumbnail-preference branch to matter). See the spec's "Self-review notes" section
for the full list of findings from that pass, including should-fix items recorded as
accepted trade-offs rather than code changes (face-migration false-suppression,
`source='removal'` PK-collision edge case, stale dismiss-constraints surviving a
later manual merge).

## Global constraints

- No schema/migration changes — both changes operate on the existing `constraints`
  and `dismissed_pairs` tables as-is.
- No frontend changes — this is purely a backend suggestion-filtering + write-path fix.
- Follow this repo's TDD convention (superpowers:test-driven-development): write each
  new/changed test first, watch it fail for the right reason, then implement.
- `source = 'dismiss'` must gate the new subject-level check in `find_merge_suggestions`
  — `cannot_link` rows also exist with `source = 'removal'` (repo.rs:722-737,
  called from commands.rs:262 and clustering.rs:911) and must NOT be swept into the
  new subject-level suppression (see spec's "Why not the other options" for the
  must_link/cannot_link contradiction risk if scope creeps here).
- Reuse `people::repo::add_cannot_link(pool, face_a, face_b, source)` (repo.rs:722-737)
  for all new constraint writes in Change 2 — it already handles pair ordering
  (`ordered_pair`) and dedup (`INSERT OR IGNORE`). Do not hand-roll new INSERT SQL.
- Run `cargo test -p <crate> people::clustering` and the full `cargo test` for the
  `src-tauri` crate before considering any task done (see Verification below for the
  exact commands — confirm the crate/package name first, don't guess).

## File structure

- Modify: `src-tauri/src/people/clustering.rs` — `find_merge_suggestions` (add the
  subject-scoped `NOT EXISTS`), plus two new `#[tokio::test]`s in the existing
  `#[cfg(test)] mod tests`.
- Modify: `src-tauri/src/people/repo.rs` — `dismiss_merge_suggestion` (replace the
  `LIMIT 1` representative-face selection with the bounded multi-face selection).
- No changes to `src-tauri/src/db/mod.rs`, `src-tauri/src/db/tests.rs`, or any
  frontend file.

---

### Task 1: Add the subject-scoped `cannot_link` guard to `find_merge_suggestions`

**Files:**
- Modify: `src-tauri/src/people/clustering.rs`

**Context:** `find_merge_suggestions` (clustering.rs:489-546) already has a per-edge
`NOT EXISTS` cannot_link check (lines 510-515) and a `dismissed_pairs` lookup
(lines 526-535). Both stay. This task adds a second, subject-scoped `NOT EXISTS`
clause to the same `WHERE` in the main query, so a single dismiss-sourced
`cannot_link` pair suppresses the entire subject pair regardless of which exact face
pair generated the candidate edge.

- [ ] **Step 1: Write the failing tests**

  Add two `#[tokio::test]`s to the `mod tests` block in `clustering.rs`, placed near
  the existing `graph_suggestions_skipped_for_cannot_link_pair` test (~line 1103) so
  they share its `make_integration_pool()` fixture pattern:

  **Test A — `suggestion_not_resurfaced_after_subject_id_churn`:**

  *Caution (caught in review — get this right or the test is a no-op):* the churned
  side must end up with **two** faces after churn — one that carries the
  `source='dismiss'` `cannot_link`, and a second, unconstrained one that the
  candidate `face_edges` row connects to. If the candidate edge instead connects the
  exact already-constrained face pair, the pre-existing per-edge `NOT EXISTS` check
  (clustering.rs:510-515, which has no `source` filter) already suppresses it today —
  the test would pass before Change 1 is implemented, silently skipping the TDD
  "must fail first" gate and never actually exercising the new subject-scoped check.

  1. Create subjects `alice` (named) and `unnamed1` (unnamed, `name = NULL`).
  2. Insert **two** faces on `unnamed1`: `f1` and `f1b`. Insert one face `fa` on
     `alice`. Give `fa` and `f1` `face_vectors` rows (reuse the `emb_bytes` helper
     already in this test module) — `f1b` doesn't need one, it's never scored by KNN
     in this test, only referenced by id.
  3. Insert a `cannot_link` constraint on `(fa, f1, 'dismiss')` via
     `crate::people::repo::add_cannot_link(&pool, fa, f1, "dismiss")` — this simulates
     the state left behind by a prior dismissal. Deliberately do **not** constrain
     `(fa, f1b)`.
  4. Simulate id churn: delete `unnamed1` from `subjects` (its faces get
     `subject_id` set to NULL by the schema's `ON DELETE SET NULL` — note this is a
     simplification for test setup, not a literal replay of `relabel_from_edges`,
     which re-points faces to the new subject directly without an intermediate NULL
     state; the two converge to the same final DB row values, which is all
     `find_merge_suggestions` reads), then insert a **new** subject `unnamed2` and
     `UPDATE faces SET subject_id = ? WHERE id IN (f1, f1b)` to point **both** faces
     at it (mirrors the net effect of `relabel_from_edges`'s `LabelAction::NewSubject`
     path re-assigning a shifted cluster's faces to a freshly `insert_subject`-ed row).
  5. Insert a `face_edges` row on `(fa, f1b)` — the **unconstrained** pair — not
     `(fa, f1)` (see caution above). Check existing test inserts in this file for the
     face-id ordering the table's `PRIMARY KEY (face_a, face_b)` expects.
  6. Call `find_merge_suggestions(&pool).await.unwrap()`.
  7. Assert `SELECT COUNT(*) FROM merge_suggestions` is `0` — the pair must not
     resurface even though the live candidate edge is between `fa` and a face
     (`f1b`) that was never directly constrained, `f1`/`f1b` now live under
     `unnamed2`'s new id, and `dismissed_pairs` has no row naming `unnamed2`.

  **Test B — `suggestion_not_resurfaced_after_new_face_added`:**
  1. Create subjects `alice` (named) and `bob` (named), each with one face
     (`fa`, `fb`) and `face_vectors` rows.
  2. Dismiss the pair: insert `cannot_link (fa, fb, 'dismiss')` via `add_cannot_link`.
  3. Add a **new** face `fc` to `bob` (a face added after dismissal), with its own
     `face_vectors` row.
  4. Insert a **new** `face_edges` row `(fa, fc, weight)` — a fresh cross-subject
     candidate edge that was never part of the original dismissal and has no
     `cannot_link` row of its own.
  5. Call `find_merge_suggestions(&pool).await.unwrap()`.
  6. Assert `SELECT COUNT(*) FROM merge_suggestions` is `0` — the `(fa, fc)` edge
     resolves to the same `(alice, bob)` subject pair, which must stay suppressed via
     the still-standing `(fa, fb, 'dismiss')` constraint.

  Run `cargo test -p <crate-name> --lib people::clustering::tests::suggestion_not_resurfaced` —
  confirm the crate/package name from `src-tauri/Cargo.toml` first, don't assume.
  Both tests must **fail** at this point (suggestions of `1` instead of `0`), for the
  reason described (no subject-level check exists yet) — not for an unrelated
  compile error or fixture bug. If a test fails to compile, fix the fixture, not the
  assertion.

- [ ] **Step 2: Implement the subject-scoped `NOT EXISTS` clause**

  In `find_merge_suggestions`'s SQL (clustering.rs:496-517), add a second `NOT EXISTS`
  clause to the `WHERE`, alongside the existing per-edge one:

  ```sql
  AND NOT EXISTS (
      SELECT 1 FROM constraints c
      JOIN faces fca ON fca.id = c.face_a
      JOIN faces fcb ON fcb.id = c.face_b
      WHERE c.kind = 'cannot_link' AND c.source = 'dismiss'
        AND ((fca.subject_id = f1.subject_id AND fcb.subject_id = f2.subject_id)
          OR (fca.subject_id = f2.subject_id AND fcb.subject_id = f1.subject_id))
  )
  ```

  Keep the existing per-edge `NOT EXISTS` (lines 510-515) and the `dismissed_pairs`
  loop (lines 526-535) unchanged.

- [ ] **Step 3: Verify both new tests pass, and existing tests still pass**

  Run `cargo test -p <crate-name> --lib people::clustering` (full module, not just
  the two new tests) — confirm the existing
  `graph_suggestions_skipped_for_cannot_link_pair`,
  `integration_remove_face_then_recluster_not_reassigned`, and
  `integration_merge_distant_groups_stays_merged_after_recluster` tests are still
  green (the new clause must not suppress `'removal'`-sourced or same-subject cases
  it shouldn't touch).

---

### Task 2: Harden representative-face selection in `dismiss_merge_suggestion`

**Files:**
- Modify: `src-tauri/src/people/repo.rs`

**Context:** `dismiss_merge_suggestion` (repo.rs:579-627) currently picks one
arbitrary face per subject (`LIMIT 1`, no `ORDER BY`) and writes a single
`cannot_link` row. Replace with up to 3 representative faces per side (thumbnail face
first, then lowest ids), writing up to 9 pairwise `cannot_link` rows via
`add_cannot_link`.

- [ ] **Step 1: Write the failing test**

  Add a test to `src-tauri/src/db/tests.rs` (alongside
  `dismiss_persists_pair_in_dismissed_pairs`, ~line 313), using
  `make_dismissal_pool()` extended with a `faces` table (that helper's underlying
  `make_pool()` already creates `faces` — confirm before adding a duplicate
  `CREATE TABLE`).

  **Test — `dismiss_writes_multiple_representative_cannot_links`:**
  1. Create subjects `alice`, `bob`.
  2. Insert 4 faces under `alice` (so the 3-face cap is actually exercised) and 2
     faces under `bob`, with distinct `id`s (rely on `AUTOINCREMENT` ordering).
     Critically, set `alice.thumbnail_face_id` to alice's **4th / highest-id** face,
     not one of the lowest 3. If the thumbnail happens to already be one of the
     lowest-3-by-id faces, a plain `ORDER BY id LIMIT 3` would select it anyway and
     the "thumbnail preferred" assertion below would pass even against an
     implementation that never reads `thumbnail_face_id` at all — so the fixture only
     pins down the selection logic if the thumbnail face is one that `ORDER BY id
     LIMIT 3` alone would otherwise have excluded.
  3. Insert a `merge_suggestions` row for `(alice, bob)`.
  4. Call `dismiss_merge_suggestion(&pool, suggestion_id).await.unwrap()`.
  5. Assert `SELECT COUNT(*) FROM constraints WHERE kind='cannot_link' AND
     source='dismiss'` is exactly `6` (`3 faces-of-alice × 2 faces-of-bob`, since bob
     only has 2 faces) — not just a range, so the test pins down the selection logic.
  6. Assert the row set includes a pair containing alice's `thumbnail_face_id`
     (proves the thumbnail was actually selected despite not being in the
     lowest-3-by-id set) **and** assert the row set does *not* contain alice's
     3rd-lowest-id face (proves the thumbnail displaced it rather than the
     implementation just taking the 3 lowest ids regardless of thumbnail).

  Run the test, confirm it fails against the current `LIMIT 1` implementation (should
  see `COUNT = 1`, not the expected `6`).

- [ ] **Step 2: Implement the bounded multi-face selection**

  Replace repo.rs:603-619 (`rep_a`/`rep_b` `LIMIT 1` selects and the single
  `add_cannot_link`-equivalent hand-rolled INSERT) with:

  1. Two queries, one per side (`lo`, `hi`), each selecting up to 3 face ids:
     ```sql
     SELECT id FROM faces WHERE subject_id = ?
     ORDER BY (id != (SELECT thumbnail_face_id FROM subjects WHERE id = ?)), id
     LIMIT 3
     ```
     (or equivalent: fetch `thumbnail_face_id` first, then `SELECT id FROM faces
     WHERE subject_id = ? AND id != ? ORDER BY id LIMIT 2` for the rest, unioning
     with the thumbnail id if present — pick whichever reads more clearly in Rust;
     both are O(1) queries, not O(n)).
  2. For each `(face_a, face_b)` in the cross product of the two id lists, call
     `add_cannot_link(pool, face_a, face_b, "dismiss").await?`.
  3. Skip entirely (as today) if either subject has zero faces.

  Keep the `dismissed_pairs` write (repo.rs:594-601) and the final
  `DELETE FROM merge_suggestions` (repo.rs:622-625) unchanged.

- [ ] **Step 3: Verify the new test passes, and existing dismissal tests still pass**

  Run `cargo test -p <crate-name> --lib db::tests::dismiss` — confirm
  `dismiss_persists_pair_in_dismissed_pairs`, `get_dismissed_pair_set_returns_stored_pairs`,
  and the new `dismiss_writes_multiple_representative_cannot_links` are all green.

---

### Task 3: Full verification pass

**Files:** none (verification only)

- [ ] **Step 1:** Run the full `src-tauri` test suite (`cargo test` from
  `src-tauri/`, or the workspace-root equivalent — confirm which from
  `src-tauri/Cargo.toml` / the workspace `Cargo.toml`) and confirm zero regressions,
  not just the touched modules.
- [ ] **Step 2:** Run `cargo clippy` (this repo's pre-commit hooks already run
  `cargo fmt`/`clippy` per `b291163`, so this should be a no-op if the diff is clean
  — confirm no new warnings on the touched files regardless).
- [ ] **Step 3:** Manually sanity-check the new `find_merge_suggestions` SQL against
  SQLite's semantics for bare (non-aggregated) column references inside a `GROUP BY`
  query one more time before merging — the existing per-edge check at lines 510-515
  already relies on this same pattern (`fe.face_a`/`fe.face_b` referenced without an
  aggregate under `GROUP BY`), so the new clause is consistent with established
  precedent in this file, but it's worth a second look since it's easy to get subtly
  wrong when SQLite picks an arbitrary row per group.

## Out of scope (explicitly, per spec)

- No "un-dismiss / reconsider" UI or command — noted in the spec as a cheap follow-up
  (`source='dismiss'` tagging already supports it) but not built here.
- No change to `dismissed_pairs`'s schema or its `ON DELETE CASCADE`.
- No frontend changes — `find_merge_suggestions` and `dismiss_merge_suggestion` are
  both already wired into existing commands/flows; this plan changes their internals
  only.
