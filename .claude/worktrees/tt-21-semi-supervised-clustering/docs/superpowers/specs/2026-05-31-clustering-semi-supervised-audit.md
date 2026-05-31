# Clustering Semi-Supervised Audit (TT-21)

**Date:** 2026-05-31
**Status:** Spike / research — no production code change.
**Goal:** Audit the face clustering / assignment / merge-suggestion flow against
semi-supervised principles. Document where user corrections are dropped, and
propose a durable-constraint design (must-link / cannot-link) with a schema
sketch and dependency-chained follow-up tasks.

All line references are against `main` @ `3509ff1`.

---

## 1. How clustering works today

`cluster_unassigned_faces` (`src-tauri/src/clustering.rs:8`) runs in two passes:

1. **Anchor build** — `compute_anchor_centroids` (`clustering.rs:157`) builds one
   centroid per subject. It prefers `is_manual = 1` faces
   (`get_manual_face_embeddings_by_subject`, `db.rs:698`) and *falls back to all
   faces of the subject* (`get_subject_embeddings`, `db.rs:684`) when a subject
   has no manual faces.
2. **Pass 1 — greedy anchor match** — every unassigned face
   (`get_unassigned_faces_with_embeddings`, `db.rs:881`) is assigned to the
   nearest anchor whose cosine similarity exceeds
   `ANCHOR_MATCH_THRESHOLD = 0.75` (`clustering.rs:155`, `find_nearest_anchor`).
3. **Pass 2 — residual HDBSCAN** — leftover faces are clustered; each new cluster
   becomes a brand-new unnamed subject.
4. After clustering, `find_merge_suggestions` (`clustering.rs:105`) is always
   re-run. It **clears the whole `merge_suggestions` table**
   (`clear_merge_suggestions`, `db.rs:968`) and recomputes every subject-pair
   whose anchor-centroid similarity exceeds
   `MERGE_CENTROID_SIMILARITY_THRESHOLD = 0.65`.

The only signal that survives a re-cluster is the **`is_manual` flag on `faces`**.
Everything else the user expresses is either not stored as a constraint, or
stored and never read.

---

## 2. Correction data-flow map

Every place a user expresses an intent about identity, and what happens to it:

| User action | Command → DB write | Persisted? | Influences future assignment? | Gap |
|---|---|---|---|---|
| **Rename a subject** | `update_subject_name` (`commands.rs:249` → `db.rs:733`) sets `subjects.name` | ✅ name persists | ⚠️ Indirect only. Clustering keys on `subject_id`, not name. A rename never moves faces, so it is safe — but it also carries no similarity signal. | None critical. Names are labels, not constraints. |
| **Manually assign a face** to a subject | `assign_face_to_subject` (`commands.rs:373` → `db.rs:1084`) sets `subject_id`, `is_manual = 1` | ✅ | ✅ **Yes** — `is_manual = 1` faces become the subject's anchor centroid. This is the one correction that already feeds the model. | Anchor is an unweighted mean; a single manual face is diluted once auto-assigned faces are folded into `get_subject_embeddings` fallback (only when there are *zero* manual faces, so impact is limited). |
| **Create subject for a face** | `create_subject_for_face` (`commands.rs:384` → `db.rs:1093`) inserts subject, sets `subject_id`, `is_manual = 1` | ✅ | ✅ Same as manual assign. | None new. |
| **Unassign a face** (this is *not* mine) | `unassign_face` (`commands.rs:395`): `db.rs:1145` sets `subject_id = NULL, is_manual = 1`, then `record_face_correction(old, NULL)` (`db.rs:1131`) writes a `face_corrections` row | ⚠️ Partly. The NULL sticks; the *correction row is written but never read*. | ❌ **No — actively regresses.** Next re-cluster sees the face as unassigned and Pass-1 greedy match will happily re-assign it to the *same* anchor it was just pulled from (the rejected subject's centroid is unchanged and still > 0.75). | **Core regression.** `face_corrections` is write-only. The unassign does not become a cannot-link between this face and the rejected subject. |
| **Merge two subjects** | `merge_subjects` (`commands.rs:352` → `db.rs:1034`): repoints `faces.subject_id`, deletes the source subject, deletes `merge_suggestions` for the pair | ✅ faces moved | ⚠️ Weak. Moved faces keep their *existing* `is_manual` value — auto-assigned faces stay `is_manual = 0`, so the merge does **not** strengthen the anchor. No must-link is recorded. | If the two identities drift apart later (new faces), nothing biases them back together. A confirmed merge is not a durable must-link. |
| **Dismiss a merge suggestion** | `dismiss_merge_suggestion` (`commands.rs:363` → `db.rs:1058`): `DELETE FROM merge_suggestions WHERE id = ?` | ❌ forgotten | ❌ **No.** `find_merge_suggestions` clears + recomputes the whole table on the next re-cluster, so the dismissed pair is re-suggested every time. | **Core regression.** A dismissal is the clearest "these are different people" signal the user can give, and it evaporates. No cannot-link. |

**Summary of the two hard gaps:**
- **Unassign → cannot-link (face↔subject):** recorded in `face_corrections` but ignored; the face is re-assigned to the rejected subject on the next pass.
- **Dismiss → cannot-link (subject↔subject):** not recorded at all; re-suggested forever.

And the one soft gap:
- **Merge → must-link (subject↔subject):** the merge happens but isn't a durable constraint; nothing keeps the identity fused as new faces arrive, and the merge doesn't raise anchor confidence.

---

## 3. Semi-supervised evaluation: must-link / cannot-link

Constrained clustering frames user corrections as pairwise constraints:

- **Must-link (ML):** two items belong to the same cluster. Source here = a
  **confirmed merge**. Effect wanted: future faces close to *either* merged
  identity should resolve to the merged identity, and the pair should never be
  re-split by automation.
- **Cannot-link (CL):** two items must never share a cluster. Sources here =
  a **dismissed merge suggestion** (subject↔subject) and an **unassign**
  (face↔subject). Effect wanted: never re-suggest / never auto-assign across a
  CL edge.

Nebula's current state vs. the constrained-clustering ideal:

| Constraint | Currently captured | Currently enforced | Verdict |
|---|---|---|---|
| Must-link (merge) | implicitly (faces moved) | not as a forward constraint | **Partial** — works until new faces arrive or someone re-clusters; no anti-split guarantee. |
| Cannot-link (dismiss) | no | no | **Missing.** |
| Cannot-link (unassign) | row written, unused | no | **Missing in effect.** |

### Where enforcement must hook in

- **Pass-1 greedy match** (`clustering.rs:36–49`): before assigning face *f* to
  subject *s*, skip if a `cannot-link(face=f, subject=s)` exists. This closes the
  unassign regression.
- **`find_merge_suggestions`** (`clustering.rs:125–142`): before inserting a
  suggestion for pair *(a,b)*, skip if `cannot-link(a,b)` exists. This closes the
  dismiss regression. (Coordinate with TT-23 "merge-suggestion business rules"
  and TT-24 "Merge Review modal" — the same query should gate both the rule
  layer and the modal.)
- **Must-link** is better enforced at *merge time* (record the link + optionally
  promote the moved faces' anchor weight) and at *suggestion time* (a standing
  ML can auto-resolve rather than re-suggest). A full re-clustering ML solver is
  out of scope for the alpha.

---

## 4. Anchor strategy assessment

Two concrete questions from the task:

1. **Should manual corrections increase a subject's anchor weight?**
   Today the anchor is an *unweighted mean* of `is_manual = 1` faces
   (`compute_anchor_centroids`, `db.rs`/`clustering.rs:157`). A subject with one
   deliberate manual face and a subject with twenty share the same
   "confidence" — only the centroid position differs, never the match threshold.
   Recommendation: keep the mean for now (cheap, predictable) but treat manual
   count as a **confidence signal** later — e.g. a subject with ≥N manual faces
   could use a slightly *lower* match threshold (more eager to claim faces it is
   sure about) and a higher merge threshold (harder to merge a well-defined
   identity into another). This is a tuning follow-up, not a blocker.

2. **Should `face_corrections` feed centroid computation?**
   Not directly as positive signal — a correction is a *negative* example
   (this face is **not** subject X). It should feed the **cannot-link** set
   consumed by Pass-1 and by `find_merge_suggestions`, not the centroid mean.
   Folding negatives into a mean-centroid model is ill-defined. So:
   `face_corrections` → cannot-link, **not** → anchors.

---

## 5. Where constraints should live (schema sketch)

Per the alpha **no-migrations** rule, this goes **inline in `BASE_SCHEMA`**
(`db.rs:7`); reset by wiping `APP_DATA`, never via `VERSIONED_MIGRATIONS`.

A single normalized constraint table covers both subject↔subject links. Face↔subject
cannot-links are already representable via the existing `face_corrections` table —
we just need to *read* it — so the new table is scoped to subject pairs.

```sql
-- Persistent constrained-clustering edges between two subjects.
-- type: 'must'   = confirmed merge (same identity)
--       'cannot' = dismissed suggestion (different identities)
-- source: provenance for debugging / undo ('merge', 'dismiss', 'manual')
CREATE TABLE IF NOT EXISTS subject_links (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id_a  INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    subject_id_b  INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    link_type     TEXT    NOT NULL CHECK (link_type IN ('must','cannot')),
    source        TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);

-- Canonical ordering (a<b) so (1,2) and (2,1) collapse — mirrors the existing
-- idx_merge_pair trick on merge_suggestions (db.rs:108).
CREATE UNIQUE INDEX IF NOT EXISTS idx_subject_link_pair ON subject_links(
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_a ELSE subject_id_b END,
    CASE WHEN subject_id_a < subject_id_b THEN subject_id_b ELSE subject_id_a END,
    link_type
);
```

### How the two hot paths query it

**`find_merge_suggestions`** (`clustering.rs:105`) — gate insertion:

```rust
// pseudo: after computing sim for pair (id_a, id_b)
if sim > MERGE_CENTROID_SIMILARITY_THRESHOLD
    && !db::has_subject_link(pool, id_a, id_b, "cannot").await?
{
    db::insert_merge_suggestion(pool, id_a, id_b, sim).await?;
}
```

**`cluster_unassigned_faces` Pass-1** (`clustering.rs:36`) — gate assignment with
the *existing* `face_corrections` rows (face↔subject cannot-link):

```rust
// pseudo: candidate sid from find_nearest_anchor for face_id
if let Some(sid) = find_nearest_anchor(&emb, &anchor_centroids, THRESH) {
    if db::face_rejected_subject(pool, face_id, sid).await? {
        residual_faces.push((face_id, emb));   // fall through to HDBSCAN
    } else {
        db::update_face_subject(pool, face_id, Some(sid)).await?;
    }
}
```

where `face_rejected_subject` is `SELECT 1 FROM face_corrections WHERE face_id = ?
AND old_subject_id = ? AND new_subject_id IS NULL`.

**Must-link on merge** (`merge_subjects`, `db.rs:1034`): when subject *src* is
merged into *tgt*, (a) record `subject_links(tgt, src-identity, 'must', 'merge')`
is moot because *src* row is deleted — so the durable ML must instead be
expressed as a CL-suppression + optional **anchor promotion**: set
`is_manual = 1` on the moved faces (or a new `is_anchor` flag) so the merged
identity's centroid is reinforced. The cleaner long-term model (keep a tombstoned
identity key for ML) is noted as a stretch follow-up.

---

## 6. Acceptance-criteria checklist

- [x] Findings doc committed listing every correction type and whether it persists + influences assignment (§2).
- [x] Concrete must-link / cannot-link persistence recommendation with schema sketch (§3–§5).
- [x] The dismiss→cannot-link gap documented with a proposed fix, coordinated with the Review-modal work (§2, §3; cross-refs TT-23/TT-24).
- [x] Follow-up tasks created in Notion and dependency-chained (§7).
- [x] No production code change — spike only.

---

## 7. Follow-up tasks (created in Notion, Detailed)

Created and dependency-chained (see Notion for IDs / links):

1. **Persist cannot-link on dismiss** — add `subject_links`, write a `cannot`
   row on `dismiss_merge_suggestion`, and gate `find_merge_suggestions`.
   *Blocks #2 and #3 (they reuse the `subject_links` table + helpers).*
2. **Enforce unassign as a face↔subject cannot-link** — read the existing
   `face_corrections` rows in Pass-1 of `cluster_unassigned_faces`.
   *Blocked by #1.*
3. **Must-link + anchor promotion on merge** — reinforce the merged identity's
   anchor (promote moved faces) and suppress re-suggestion of merged identities.
   *Blocked by #1.*
4. **Weight anchors by manual-correction confidence** — tuning task; adaptive
   thresholds from manual-face counts. *Independent; lowest priority.*
