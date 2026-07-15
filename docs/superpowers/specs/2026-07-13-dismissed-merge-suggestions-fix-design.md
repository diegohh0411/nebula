# Dismissed Merge Suggestions Reappear — Design

**Date:** 2026-07-13
**Status:** Approved, ready for implementation planning
**Source:** Notion task "Fix: dismissed merge suggestions reappear after reclustering" (page `39ce954d-b476-81d8-926c-f109f0e42d72`)

## Problem statement

When a user clicks "Not the same person" on a People > Possible Duplicates merge
recommendation, the suggestion disappears immediately but can reappear later, after an
idle reclustering pass runs. The dismissal is not durable across reclustering.

## Root cause

`dismiss_merge_suggestion` (`src-tauri/src/people/repo.rs:579-627`) persists a rejection
two ways:

1. A `dismissed_pairs` row keyed on `(subject_id_a, subject_id_b)`, with
   `ON DELETE CASCADE` on `subjects.id` (schema: `src-tauri/src/db/mod.rs:147-154`).
2. A single `cannot_link` constraint (`source = 'dismiss'`) between exactly **one**
   arbitrary representative face per subject, chosen with `LIMIT 1` and no `ORDER BY`
   (repo.rs:604-619).

Unnamed/auto-clustered subjects are ephemeral. Idle reclustering
(`clustering::relabel_from_edges`, clustering.rs:304-374, via `compute_label_actions`,
clustering.rs:194-245) can delete and recreate an unnamed subject under a **new id**
when its face cluster shifts — new photos change the k-NN graph
(`TAU_SIM = 0.45`, `K_NEAREST = 5`, clustering.rs). `delete_subjects_with_no_faces`
(repo.rs:350-357) then deletes the old, now-empty subject row, which **cascades away
the `dismissed_pairs` row** with it via the FK.

The single surviving face-level `cannot_link` only blocks that one exact face pair.
`find_merge_suggestions` (clustering.rs:489-546) already carries a `NOT EXISTS`
guard against `constraints` (lines 510-515) — but it is scoped to the *specific*
`face_edges` row being scored (`c.face_a = MIN(fe.face_a, fe.face_b) AND c.face_b =
MAX(fe.face_a, fe.face_b)`), not to the subject pair as a whole. Any other face pair
between the same two subjects — e.g. a new face added to either subject after
dismissal, or a fresh k-NN edge discovered on recluster — still scores a fresh match
and is not caught by that per-edge check. Combined with a new subject id never
appearing in `dismissed_pairs`, nothing suppresses the resurfaced pair.

Two failure paths, both need closing:

- **Subject-id churn** from reclustering (loses the `dismissed_pairs` row; the
  existing per-edge `cannot_link` check doesn't help because it's keyed to the old
  face pair by coincidence, not by subject).
- **New faces added** to either subject after dismissal (the single-face-pair
  `cannot_link` doesn't cover any pair involving the new face).

## Fix

Don't try to make `dismissed_pairs` survive id churn, and don't blanket-write
`cannot_link` across every face pair (O(n·m) rows would degrade clustering
performance — `build_components_with_constraints`, clustering.rs:247-298, does
`cannot_links.iter().any(...)` per candidate edge — and would poison future manual
merges with mass must_link/cannot_link contradictions, since `merge_subjects`,
repo.rs:473-558, writes `must_link` across every face pair of the two subjects being
merged). Instead, fix the **suggestion filter** to check face-level constraints across
the *whole* subject pair, since faces (not subjects) are the stable identity across
reclustering.

### Change 1 — `find_merge_suggestions` (`clustering.rs:489-546`)

Add a second `NOT EXISTS` clause, in addition to the existing per-edge one, that
checks whether any dismiss-sourced `cannot_link` spans the two subjects' *current*
face sets:

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

This is placed inside the existing `WHERE` clause of the aggregate query. `WHERE` is
evaluated per input row, *before* `GROUP BY` collapses rows into groups — the clause
is correct not because `subject_id` is "constant within a group" at evaluation time,
but because the predicate's truth value happens to be identical across every row that
would land in the same group, so filtering row-by-row and then grouping produces the
same result as filtering whole groups. (This is a different — and non-hazardous —
situation from bare, non-aggregated columns appearing in the `SELECT` list of a
`GROUP BY` query, where SQLite's "pick an arbitrary row per group" behavior actually
matters; that hazard doesn't apply here since this is a `WHERE`-clause predicate, not
a selected value.)

`source = 'dismiss'` is required: `cannot_link` rows are also written with
`source = 'removal'` (face-removal-from-subject flow, repo.rs:722-737, called from
commands.rs:262 and clustering.rs:911) and with those semantics a removal constraint
pins a *specific* face out of a subject — it must not be allowed to suppress a
same-named-subject merge suggestion between two otherwise unrelated subjects that
happen to share that one forbidden face pair.

**Known trade-off, accepted as in-scope for this fix:** the new check keys off each
constrained face's *current* `subject_id`, which is exactly what makes it survive
subject-id churn — but it also means that if a face involved in a `'dismiss'`
constraint later migrates (via reclustering) into a *different* subject than the one
it was dismissed against, the surviving constraint will suppress a suggestion between
the new subject pair that the user never actually dismissed. This is judged
acceptable: false-suppression of an occasional merge suggestion is a much smaller cost
than the current bug (a dismissal that doesn't stick at all), and the user can still
merge manually if a suppressed pair should in fact be merged. Not fixed here; flag if
it becomes a reported problem in practice.

**Known interaction with manual merges, accepted as in-scope for this fix:**
`merge_subjects` (repo.rs:473-558) does not delete any `source = 'dismiss'`
`cannot_link` rows between the two subjects being merged — it only ever *adds*
`must_link` rows and lets `build_components_with_constraints` (clustering.rs:268-278,
286-294) resolve the resulting contradiction in `must_link`'s favor (with a `warn!`
log). This was already true before this fix for the single dismiss-sourced pair;
Change 2 makes it true for up to 9 pairs instead of 1. Functionally harmless
(`must_link` always wins), just noisier in logs. Not fixed here — a follow-up could
have `merge_subjects` delete `source = 'dismiss'` constraints between the merged face
sets as part of its existing transaction.

Keep the existing per-edge `cannot_link` check (lines 510-515) and the existing
`dismissed_pairs` check (lines 526-535) as-is — both are cheap and remain correct for
their original cases (the per-edge check also still applies to `'removal'`-sourced
constraints, which the new subject-level check intentionally does not).

This closes both failure paths:
- **Id churn**: the constraint is face-keyed, so it survives the unnamed subject being
  deleted and recreated under a new id — the *faces* keep their identity, and the new
  subject row's `faces.subject_id` join picks the constraint up regardless of the
  subject id's value.
- **New faces**: one surviving `source='dismiss'` constraint pair suppresses the whole
  subject pair regardless of which specific face pair generated the new candidate
  edge, because the check joins on `subject_id`, not on the exact face pair.

### Change 2 — harden the constraint write in `dismiss_merge_suggestion` (`repo.rs:603-619`)

Replace the `LIMIT 1`, no-`ORDER BY` representative-face selection with a small
deterministic set: up to 3 representative faces per side total — the subject's
`thumbnail_face_id` if set, plus enough lowest-`id` faces (`ORDER BY id`) to fill the
remaining slots up to 3 — writing up to 3 × 3 = 9 `cannot_link` rows (one per cross
pair, fewer if either side has under 3 faces), via the existing
`repo::add_cannot_link(pool, face_a, face_b, "dismiss")` helper (repo.rs:722-737),
which already handles pair ordering and dedup (`INSERT OR IGNORE`).

Rationale: this is not a correctness fix for the reappearance bug — Change 1 already
makes a *single* surviving constraint pair sufficient, since it's subject-scoped, not
face-pair-scoped. It is robustness against the specific representative face's
**photo being deleted later**: `constraints` rows cascade on `faces.id` delete
(`db/mod.rs:157-158`), so a single representative face pair is a single point of
failure — if that one face is deleted (image removed from library, face
re-detected/re-cropped, etc.), the `cannot_link` row disappears and the pair is fully
unprotected again until the next dismiss. Nine rows across three faces per side keeps
this a bounded constant (not O(n·m) — capped regardless of how many faces the subject
eventually accumulates) while making single-face deletion non-fatal to the dismissal.

Selection order — prefer `thumbnail_face_id` first, then lowest `id`s — is chosen
because the thumbnail face is the one most likely to persist (subjects already prefer
never clearing an existing thumbnail, per `upgrade_subject_thumbnails`, repo.rs:373-405)
and is deterministic, which keeps the write idempotent and testable.

**Known edge case, pre-existing and not introduced by this fix:** `constraints`'
primary key is `(face_a, face_b, kind)` — `source` is not part of the key
(db/mod.rs:156-163) — and `add_cannot_link` writes via `INSERT OR IGNORE`. If a
representative face pair already has a `cannot_link` row from `source = 'removal'`
(written by the face-removal flow, repo.rs:722-737 / commands.rs:262 /
clustering.rs:911), the dismiss-time insert for that exact pair is silently a no-op
and the row keeps `source = 'removal'` — Change 1's new subject-scoped check only
matches `source = 'dismiss'`, so that particular pair does not count toward
suppression (though the existing per-edge check at lines 510-515, which has no
`source` filter, still suppresses that specific face pair regardless). This was
already true pre-fix for the single `LIMIT 1` pair; Change 2 only reduces the odds of
it mattering, by giving each dismissal up to 9 chances instead of 1 to land a
`source = 'dismiss'` row that survives id churn. It is not fully eliminated. Considered
acceptable for this fix (the collision requires a fairly specific prior-removal
history on the exact representative faces chosen), but flag as a candidate follow-up
if dismissals are observed not to stick even after this fix ships — the fix would be
to check for an existing `cannot_link` row (any source) before insert and, if found,
leave a `source = 'dismiss'` marker via a secondary mechanism rather than relying on
`INSERT OR IGNORE`'s silent no-op.

### Change 3 — leave `dismissed_pairs` and its CASCADE as-is

No schema change. After Change 1, `dismissed_pairs` is a fast-path/UI metadata table
(cheap early-exit in `find_merge_suggestions`, useful for a future "list of things
I've dismissed" UI) — not the source of truth for suppression. Its CASCADE-driven
loss on subject-id churn is expected and no longer load-bearing.

## Schema / data implications

No migration needed. Both changes operate entirely within the existing `constraints`
table shape (`db/mod.rs:156-163`, `PRIMARY KEY (face_a, face_b, kind)`) and the
existing `dismissed_pairs` table. `add_cannot_link` already dedupes via
`INSERT OR IGNORE`, so re-dismissing (if that ever becomes reachable) or dismissing a
pair whose representative-face set overlaps a prior dismissal is a no-op for the
overlapping rows.

## Why not the other options considered

- **Blanket all-face-pairs `cannot_link` on dismiss** (write a `cannot_link` row for
  every face-in-A × face-in-B pair, not just a bounded representative set): O(n·m)
  constraint rows, which degrades `build_components_with_constraints`'s per-edge
  `cannot_links.iter().any(...)` scan (clustering.rs:268) as subjects grow, and causes
  mass must_link/cannot_link contradictions the next time these two subjects (or their
  faces) are involved in a manual merge (`merge_subjects` writes `must_link` across
  every face pair of the merged subjects, repo.rs:515-524 — a prior blanket
  `cannot_link` set would collide with that must_link set face-pair-for-face-pair).
  It also doesn't actually fix the filter's face-pair-exactness for faces added
  *after* dismissal — a new face still needs the subject-level check from Change 1 to
  be caught, making the blanket write pure overhead with no additional coverage.
- **Kill the `dismissed_pairs` CASCADE, or propagate subject ids across reclustering**:
  `relabel_from_edges` creates new subjects with no lineage to the old ids it's
  replacing (`LabelAction::NewSubject`, clustering.rs:182-184, 339-345) — there is no
  "old subject id -> new subject id" mapping available at the point a new subject is
  created. Preserving `dismissed_pairs` across churn would require inventing a
  heuristic face-overlap lineage mechanism to decide which new subject "is" which old
  one. That solves only the id-churn path, not the new-faces path, and adds a second,
  parallel lineage mechanism when a face-keyed one (constraints) already exists and
  handles both paths for free.

## Semantic note for implementation

After this fix, "dismiss" means "never re-suggest these two people while any
dismissed face remains in each cluster." Because constraint rows are tagged
`source = 'dismiss'`, a future "un-dismiss / reconsider" affordance is cheap
(`DELETE FROM constraints WHERE source = 'dismiss' AND ...` plus the corresponding
`dismissed_pairs` row) — out of scope for this fix, worth a follow-up ticket if
wanted.

## Tests to add

Both new tests belong in `src-tauri/src/people/clustering.rs`'s existing `#[cfg(test)]
mod tests`, using the `make_integration_pool()` helper (clustering.rs:834-851) and
following the pattern of the existing `graph_suggestions_skipped_for_cannot_link_pair`
test (clustering.rs ~1103-1157), rather than in `src-tauri/src/db/tests.rs`.
Rationale: `find_merge_suggestions` itself only touches `face_edges`, `faces`,
`subjects`, `constraints`, `dismissed_pairs`, and `merge_suggestions` — it does not
require `face_vectors` directly. But `db/tests.rs`'s pools (`make_merge_pool` /
`make_dismissal_pool`) don't create a `face_edges` table at all, and the module
doesn't import anything from `clustering::*` (so `find_merge_suggestions` isn't even
callable from there without adding imports). `make_integration_pool` already wires up
`face_edges`, `constraints`, `dismissed_pairs`, and `merge_suggestions` together
correctly, plus `face_vectors`/`ensure_sqlite_vec_registered()` for the tests in that
module that also exercise reclustering — so it's the natural fixture to extend even
though these two specific new tests don't need the vector table.
`db/tests.rs`'s existing `dismiss_persists_pair_in_dismissed_pairs` test only exercises
the `dismissed_pairs` write in isolation via `people::repo::dismiss_merge_suggestion`,
which remains valid and unchanged by this fix (Change 2 only alters representative-face
selection, not the `dismissed_pairs` write path).

1. **`suggestion_not_resurfaced_after_subject_id_churn`**: Dismiss a pair where the
   churned side has **two** faces — one that received the `source='dismiss'`
   `cannot_link` (call it `f1`), one that didn't (`f1b`). Simulate id churn by
   deleting the unnamed subject row and re-inserting a new subject with a new id,
   re-pointing **both** `f1` and `f1b` to it (mirrors what `relabel_from_edges` does
   when a cluster shifts: old subject deleted via `delete_subjects_with_no_faces`,
   new subject created via `insert_subject` + `update_face_subject` for all faces in
   the shifted cluster). Add the *candidate* `face_edges` row on `(fa, f1b)` — the
   unconstrained pair — not on `(fa, f1)`. This matters: if the candidate edge were on
   the exact constrained pair `(fa, f1)`, the pre-existing per-edge `NOT EXISTS` check
   (lines 510-515, which has no `source` filter) would already suppress it, and the
   test would pass even without Change 1 — failing to actually exercise the new
   subject-scoped check, and failing the TDD "must fail before the fix" gate. Run
   `find_merge_suggestions`. Assert no suggestion row exists for the new subject id
   paired with the other side.
2. **`suggestion_not_resurfaced_after_new_face_added`**: Dismiss a pair (representative
   faces get `cannot_link`), then insert a *new* face into one of the two subjects with
   a `face_edges` row connecting it to a face on the other subject (a fresh candidate
   cross-subject edge that was never part of the original dismissal). Run
   `find_merge_suggestions`. Assert no suggestion is produced, because the new edge's
   two faces resolve to the same already-dismissed subject pair via Change 1's
   subject-level check.

## Self-review notes

- Verified all cited line numbers/function names against the current worktree
  (`fix/dismissed-merge-suggestions-reappear`) as of 2026-07-13; the Notion task's
  line numbers for `dismiss_merge_suggestion` (579-627) and `find_merge_suggestions`
  (489-546) still match exactly.
- One divergence from the Notion task write-up, called out above under Change 1 and
  the Tests section: the task's narrative implies `find_merge_suggestions` has no
  existing constraint-based filtering. In fact it already has a per-edge `NOT EXISTS`
  cannot_link check (lines 510-515) that predates this fix; Change 1 *adds* a second,
  subject-scoped check alongside it rather than introducing constraint-checking from
  scratch. This doesn't change the recommended fix (the new check is additive and the
  SQL in the task's brief is correct as a supplementary clause), but it does change
  where the test suite should live — see Tests section — since the task suggested
  `db/tests.rs` and that file lacks the fixtures `find_merge_suggestions` needs.
- No placeholders, TODOs, or unresolved brackets remain in this document.
- Terminology check: "subject" vs "face" vs "constraint" used consistently with their
  meaning in the codebase (subject = person entity, face = detected face row, faces
  carry `subject_id`, constraints are face-keyed).

**Second review pass (independent model, Fable):** this spec and the companion plan
were sent for an independent adversarial review against the current worktree. Findings
incorporated into this revision:
- **Blocking, fixed:** the id-churn test's originally-proposed fixture put the
  candidate `face_edges` row on the exact already-constrained face pair, which the
  pre-existing per-edge check already suppresses pre-fix — the test would never have
  failed before the fix, breaking the TDD gate and not actually exercising Change 1.
  Corrected above (churned side now has a second, unconstrained face; the candidate
  edge connects to that one instead).
- **Should-fix, documented above (not code-changed, by design of this being a spec, not
  an implementation):** the `source='removal'`-vs-`source='dismiss'` PK-collision edge
  case in Change 2 (see "Known edge case" under Change 2); the face-migration
  false-suppression trade-off and the stale-dismiss-constraint-after-manual-merge
  interaction (both under Change 1).
- **Nitpicks, fixed:** corrected the `WHERE`-vs-`GROUP BY` evaluation-order explanation
  under Change 1 (the original phrasing was technically backwards, though the SQL
  itself was always correct); corrected the "3 × 3" face-count wording under Change 2
  to make clear the cap is 3 total per side (thumbnail + up to 2 more), not thumbnail
  plus 3; corrected the overstated claim that `find_merge_suggestions` itself requires
  `face_vectors` (it doesn't — `make_integration_pool` is still the right fixture, but
  for different reasons, now stated accurately).
- One should-fix item flagged by the reviewer as a performance question (cost of the
  new correlated `NOT EXISTS` at scale) is noted here rather than resolved: neither
  `constraints` nor the new query path has an index on `(kind, source)`, only the
  existing PK `(face_a, face_b, kind)`. Likely fine for a background/idle pass at
  current expected data volumes, but worth a `cargo test`-adjacent timing check or an
  `EXPLAIN QUERY PLAN` look during implementation if `constraints` has grown large in
  practice; not blocking for writing the spec/plan.
