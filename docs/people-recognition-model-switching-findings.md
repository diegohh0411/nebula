# People Recognition: Model-Switching Bug & Clusterization-Preservation Findings

**Date:** 2026-07-02
**Scope:** `src-tauri/src/{pipeline,people,settings,models,app}/`
**Status:** Findings only — no code changed yet.

This report documents two related problems in the face recognition ("People Recognition") system and provides research on model alternatives, to brief whoever implements the fix.

---

## 1. Bug: the "Standard" preset is never actually used for inference

`pipeline/mod.rs:161` hardcodes the face-recognition preset:

```rust
// src-tauri/src/pipeline/mod.rs:161
let preset = &crate::models::registry::BUFFALO_S_PRESET;
```

This ignores whatever the user has selected via the `subject_model` setting. Contrast with how the *Smart Search* embedding model is resolved correctly one call site up, in `app/mod.rs:88-93`:

```rust
// src-tauri/src/app/mod.rs:88-93
let model_id = crate::settings::repo::get_setting(&pool_pipe, "embedding_model")
    .await
    .unwrap_or(None)
    .unwrap_or_else(|| crate::models::registry::SIGLIP_BASE.id.to_string());
let spec = crate::models::registry::ModelSpec::find_by_id(&model_id)
    .unwrap_or(&crate::models::registry::SIGLIP_BASE);
```

`spec` (the embedding model) is threaded through as a parameter into `run_pipeline(..., spec)` (`app/mod.rs:99-109`). There is **no equivalent parameter or lookup for the face-recognition preset** — `run_pipeline`'s signature (`pipeline/mod.rs:140-147`) takes `requested_spec` for embeddings but nothing for the face preset; line 161 just hardcodes Blitz's `BUFFALO_S_PRESET` inside the function body.

The face analyzer built from this hardcoded preset is what actually gets used:

```rust
// src-tauri/src/pipeline/mod.rs:170-186
for face_spec in [preset.detector, preset.embedder, preset.gender_age] {
    manager.ensure_ready(&app, face_spec).await?;
}
let analyzer = engine.get_face_analyzer(&manager, preset).await?;
```

Meanwhile, `settings/commands.rs:121-147` (`update_setting`, key `"subject_model"`) does the right things *except* wire the choice into the running pipeline:

```rust
// src-tauri/src/settings/commands.rs:121-147
if key == "subject_model" {
    let current = crate::settings::repo::get_setting(pool, &key).await.unwrap_or(None);
    if current.as_ref() != Some(&value) {
        let preset = crate::models::registry::FaceIdPreset::find_by_id(&value)
            .ok_or_else(|| format!("Unknown preset: {}", value))?;
        state.model_manager.ensure_ready(&app, preset.detector).await?;
        state.model_manager.ensure_ready(&app, preset.embedder).await?;
        state.model_manager.ensure_ready(&app, preset.gender_age).await?;
        crate::people::repo::reset_all_subject_data(pool).await?;
    }
}
// ... setting is persisted to the `settings` table below this block
```

It downloads the new preset's ONNX files, marks it "Active" in the UI, and triggers a full destructive reset + re-queue (see §2) — but the pipeline loop started once at app launch (`app/mod.rs:99-109`) never re-reads `subject_model` and never rebuilds its `FaceAnalyzer`. It keeps using the `BUFFALO_S_PRESET` (Blitz) analyzer it was constructed with at startup, permanently, regardless of what's shown as "Active" in Settings.

**Net effect: every face detected in this app today has been detected/embedded with Blitz's `buffalo_s` models, even on installs where "Standard" shows as Active.** Selecting Standard currently costs the user a large download + a full destructive wipe of all subjects/names/faces (§2) for **zero behavioral change** in the actual recognition quality.

### Where the fix needs to land
- `pipeline/mod.rs`: `run_pipeline` needs a `preset: &'static FaceIdPreset` parameter (mirroring `requested_spec`), used at line 161 instead of the hardcoded constant.
- `app/mod.rs:86-110`: needs to resolve the preset from the `subject_model` setting the same way `spec` is resolved from `embedding_model` (lines 88-93), and pass it into `run_pipeline`.
- Longer-term: the pipeline loop is spawned once at app startup and apparently never restarted when settings change mid-session (worth confirming whether `reset_all_subject_data` alone is sufficient to make an *already-running* pipeline loop pick up a new preset, or whether the fix also needs to signal/restart the loop after a `subject_model` change while the app is running).

---

## 2. By design (but destructive, and worth reconsidering): switching presets wipes all subject data

Confirmed in `people/repo.rs:820-856`, called from `settings/commands.rs:143-145` whenever `subject_model` changes:

```rust
// src-tauri/src/people/repo.rs:820-856
pub async fn reset_all_subject_data(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM constraints").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM merge_suggestions").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM face_vectors").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM faces").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM subjects").execute(&mut *tx).await?;
    sqlx::query("UPDATE images SET subject_analysis_done = 0 WHERE deleted_at IS NULL")
        .execute(&mut *tx).await?;
    sqlx::query("DELETE FROM embedding_queue WHERE pipeline = 'subject'").execute(&mut *tx).await?;
    // ... re-enqueues every non-deleted image for the 'subject' pipeline
    tx.commit().await?;
    Ok(())
}
```

This deletes, unconditionally and synchronously:

| Table | What's lost |
|---|---|
| `subjects` | **Every named person** — `id`, `name`, `thumbnail_face_id` all gone |
| `faces` | Every detected face (bbox, detection/quality scores) |
| `face_vectors` | Every 512-d face embedding (`vec0` virtual table, `db/mod.rs:106`) |
| `constraints` | Every manual must-link/cannot-link correction the user made (`db/mod.rs:155-162`) |
| `merge_suggestions` | Pending "these might be the same person" suggestions (`db/mod.rs:133-139`) |
| `face_edges` | *(not explicitly deleted here, but rebuilt from scratch by clustering since its rows reference deleted `faces` via `ON DELETE CASCADE`, `db/mod.rs:164-169`)* |

### Why there's no partial-preservation path today

There is **no model/preset identifier stored anywhere in the schema** — not on `faces`, not on `subjects`, not on `face_vectors`. Checked `db/mod.rs:82-106` directly. The only place preset identity exists is in code, as the `FaceIdPreset`/`ModelSpec` constants in `models/registry.rs` (`BUFFALO_S_PRESET`, `ANTELOPE_V2_PRESET`). Nothing stamps "this face/embedding was produced by preset X" onto a row, so the system has no way to know, given an existing embedding, whether it's safe to compare it against a newly-computed one — hence the current all-or-nothing wipe. This is a defensible reason for *some* invalidation (embeddings from two different models are not comparable — cosine distance between a `buffalo_s` embedding and a `glintr100` embedding is meaningless), but the current implementation invalidates strictly more than necessary: it deletes `subjects` (names) too, when in principle a subject's identity/name could survive if its faces were re-clustered and reassigned rather than the whole row being dropped.

`subjects.id` is what `faces.subject_id` (`db/mod.rs:93`, `ON DELETE SET NULL`) and downstream naming actually key off — so names are *technically* decoupled from embeddings/clusters at the schema level. The loss of names on preset switch is a consequence of `reset_all_subject_data` deleting `subjects` rows outright, not an inherent limitation of the schema.

`VERSIONED_MIGRATIONS` (`db/mod.rs:185-198`) currently contains only 3 unrelated migrations (two indexes, `saved_reports` tables) — no model-version-aware migration machinery exists yet.

### What a preservation-aware design needs to solve
1. **Tag rows with model provenance.** Add a preset/model identifier column to `faces` and/or `face_vectors` (e.g. `face_id_preset TEXT NOT NULL`) so the system can tell which rows were produced by which model.
2. **Support incremental re-embedding instead of full delete.** On preset switch, re-detect/re-embed images whose faces carry a stale preset id, without necessarily deleting `subjects`.
3. **Re-link new embeddings to existing subjects instead of only building fresh clusters.** E.g., after re-embedding a face with the new model, compare it against centroids/exemplars of *existing* subjects (computed with the new model, requiring at least one re-embedded reference face per subject) rather than only running fresh unsupervised clustering (`people/clustering.rs`, `cluster_unassigned_faces` / `relabel_from_edges`) that has no memory of prior subject identity.
4. **Decide what to do with `constraints`.** Manual must-link/cannot-link constraints are expressed as `(face_a, face_b)` pairs (`db/mod.rs:155-162`); if `faces` rows are recreated with new ids on re-embedding, constraints need to be either migrated to new face ids or re-derived (e.g. from subject-level "these two subjects were manually merged" history rather than face-pair history).
5. **Handle the migration UX**: re-embedding a whole library is expensive (network + compute for detector/recognizer + gender/age on every image). Consider whether this needs to be resumable/background/cancellable rather than a single blocking reset-and-requeue.

---

## 3. Current model registry (for reference)

`src-tauri/src/models/registry.rs`:

| Preset (`id`) | Detector | Recognizer | Gender/Age | Detector input |
|---|---|---|---|---|
| **Blitz** (`"blitz"`, lines 222-230) | `buffalo_s` detection.onnx, ~4MB (`immich-app/buffalo_s`) | `buffalo_s` recognition.onnx, ~19MB | `buffalo_s`/insightface genderage.onnx, ~1.3MB | 640×640 |
| **Standard** (`"precision"`, lines 276-284) | `antelopev2` detection.onnx (SCRFD), ~17MB (`immich-app/antelopev2`) | `antelopev2` recognition.onnx (glintr100), ~261MB | same `buffalo_s` genderage model, reused | 640×640 |

Both presets share the exact same detector input resolution and ONNX Runtime thread configuration (`vision/engine.rs:64,109`) — the only technical difference between tiers is model capacity (glintr100 is ~14× larger than `buffalo_s` recognition).

---

## 4. Research: alternative/better face recognition models on Hugging Face

| Model | Source | Size | Notes |
|---|---|---|---|
| **`immich-app/buffalo_l`** | HF, same repo layout your `ModelManager`/registry already knows how to consume (`detection/model.onnx`, `recognition/model.onnx`) | ~326MB total (w600k_r50 head, ResNet50@WebFace600K) | Community benchmarking on a 60k-face library found it gave the best precision/recall tradeoff of the InsightFace packs, edging out antelopev2 on LFW (99.83% vs 99.80%) while smaller than antelopev2's recognition head alone (261MB). Good candidate to replace or sit between the current Blitz/Standard tiers. ([immich-app/immich discussion #7838](https://github.com/immich-app/immich/discussions/7838))|
| **`immich-app/buffalo_m`** | HF, same repo layout | mid-size | Untested by this investigation; same integration path as buffalo_l. |
| **AdaFace** | Not published as a ready `immich-app`-style ONNX package | — | Uses an adaptive-margin loss tuned for variable-quality/low-res/occluded faces rather than clean studio shots — a plausible match for real photo libraries (old scans, low light, extreme angles), which is exactly the use case "Standard" targets today. Integration is heavier: requires pairing with a separate detector (SCRFD/RetinaFace) and verifying the embedding output format matches what the `face_id` crate (`Cargo.toml`, v0.4.1) expects. ([AdaFace paper](https://ar5iv.labs.arxiv.org/html/2204.00964), [AdaFace GitHub](https://github.com/mk-minchul/adaface)) |

General accuracy context from the InsightFace model zoo (IJB-C TAR @ FAR=1e-4): mobile/MBF-class models ~90-93%, R50-class (buffalo_l) ~95-96%, R100/glintr100-class (antelopev2) ~96-97.5%. LFW is near-saturated across all of these (99.5%+) and is only useful as a sanity check, not a differentiator. ([InsightFace model_zoo README](https://github.com/deepinsight/insightface/blob/master/model_zoo/README.md), [InsightFace guide](https://www.insightface.ai/guides/choose-face-recognition-model-and-evaluate))

---

## 5. Suggested task scope for the implementing agent

1. Fix the preset-wiring bug (§1) so the `subject_model` setting actually controls which models the pipeline runs.
2. Design and implement a preservation-aware re-embedding flow for preset switches (§2), replacing (or making optional) the current `reset_all_subject_data` full wipe — at minimum, preserve `subjects.name`/`id` across a model switch by re-linking re-embedded faces to existing subjects instead of unconditionally deleting them.
3. Evaluate whether `buffalo_l` should replace `antelopev2` as "Standard," or be added as a third tier, given the community evidence it may be strictly better and smaller (§4).
4. Since #1 and #2 both touch the same code paths (`pipeline/mod.rs`, `settings/commands.rs`, `people/repo.rs`), they should probably be designed together rather than sequentially — the preservation logic needs to know which rows are "stale" (produced by the old preset), which is naturally solved by whatever schema change (preset id column) the fix ends up needing anyway.
