# Local Inference Throughput Optimization — Design

**Date:** 2026-05-28
**Status:** Approved (design); pending implementation plan
**Goal:** Increase image-processing throughput on existing laptop hardware (Windows 11, integrated GPU) **without** switching to lighter ML models. Optimize the algorithms and reorganize the pipeline to use resources better.

## Context

Nebula is a Tauri (Rust backend + Angular frontend) local photo manager. Two background workers process images pulled from SQLite queues:

- **Semantic worker** (`embedder::run_semantic_worker`) → `VisionEngine::embed_image` (SigLIP, 224px) → vector index.
- **Subject worker** (`embedder::run_subject_worker`) → `face_id` detect + recognize + gender/age (Buffalo_S) → clustering.

Both run with `CONCURRENT_WORKERS = 3`. Thumbnails and face crops are generated separately.

### Workload

Incremental, but in **bursts of 300–1000 images** per increment. Throughput (images/sec across a burst) is the primary metric, not single-image latency.

### Hardware

Integrated GPU only (no discrete GPU). ONNX Runtime currently runs **CPU-only** (`ort = "2.0.0-rc.12"` with no execution-provider feature; `intra_threads(4)` hardcoded).

## Problems in the current pipeline

Grounded in the current source:

1. **Redundant decoding — the dominant cost.** A single new photo with 3 faces is decoded from the original file **6 times**: thumbnail (`thumbnail.rs:29`), semantic embed (`embedder.rs:84`), subject/faces (`embedder.rs:150`), and once per face crop (`thumbnail.rs:52`, ×3). On 12–24 MP JPEGs, decode likely dominates total cost.
2. **Inference is serialized despite "3 concurrent workers."** `VisionEngine` holds a single `Session` behind a `Mutex` and holds the lock across the entire `session.run()` (`vision_engine.rs:137-149`). The 3 workers parallelize decode but queue single-file for the model run.
3. **Every image embed also runs the text tower.** `embed_image` feeds dummy `input_ids` and discards `text_embeds`, but the combined SigLIP graph still computes the text tower — roughly 2× the needed compute per embedding (`vision_engine.rs:117-155`).
4. **Batch size is always 1.** No batched `session.run`, leaving CPU throughput unused.
5. **Scalar preprocessing + slowest resize.** Per-pixel normalization loop (`vision_engine.rs:127-132`) and `Lanczos3` resize for an image that is immediately shrunk to 224–256px.
6. **CPU-only EP, fixed thread count.** No execution-provider selection; `intra_threads` hardcoded to 4 regardless of the machine.

## Non-goals

- Switching to smaller/lighter ML models (explicitly excluded).
- Hardware upgrades.
- Changing search quality or clustering behavior (embedding *values* may shift only where noted, requiring re-embed; ranking behavior is preserved).

---

## Architecture: staged, decode-once pipeline

Replace the two competing workers with **one coordinator** (`run_pipeline`) built as a 3-stage pipeline connected by bounded channels:

```
[Stage 1: LOAD]            [Stage 2: INFER]              [Stage 3: WRITE]
 decode original once  ->   batched SigLIP embed     ->   DB write + index add
 (shared rayon pool)        batched face det/rec/age      thumbnail + face crops
 emit: DecodedImage         (dedicated inference actor)    (from the SAME buffer)
```

### `DecodedImage`

A struct carrying the **single** decoded `image::DynamicImage` (plus a pre-downscaled RGB working copy) through the pipeline. Thumbnail, semantic embed, face detection, and face crops all read from this in-memory buffer. Eliminates 5 of the 6 decodes.

### Stage responsibilities

- **Stage 1 — LOAD:** CPU/IO-bound decode + downscale on a shared `rayon`/blocking pool. Produces `DecodedImage`.
- **Stage 2 — INFER:** the model stage. Accumulates batches and runs the embed and face models via a dedicated inference actor (see below).
- **Stage 3 — WRITE:** async DB writes, vector-index `add`, thumbnail + face-crop encoding (reusing the decoded buffer).

### Channels & backpressure

Bounded channels between stages cap in-flight decoded images (memory safety on 24 MP buffers) and create natural backpressure: if inference is the bottleneck, decoders idle rather than accumulating RAM. Channel depths are config knobs.

### Queue compatibility

The existing `semantic` and `subject` SQLite queue tables stay (still used for resumability, retry, and progress reporting). The two logical queues **merge into one work item per image**, so each image flows through both embed and face stages from a single decode. The coordinator replaces `run_semantic_worker` + `run_subject_worker`.

---

## Inference layer

### 2.1 Remove the session-mutex serialization

Introduce an **inference actor**: a dedicated task that owns the `Session`, fed by an `mpsc` channel of work items each carrying a `oneshot` reply. Callers `await` a result instead of acquiring a `Mutex`. ORT's intra-op thread pool parallelizes *within* each (batched) run — more efficient on a small core count than multiple sessions contending for the same cores.

### 2.2 Split the SigLIP towers

A dual-encoder model has two independent sub-networks ("towers"): the **vision tower** (image → embedding) and the **text tower** (text → embedding). They meet only at cosine-similarity time. The current combined `model.onnx` runs both per call.

Point the model registry at the `onnx-community` **pre-split** files:

- `embed_image` → `onnx/vision_model.onnx`
- `embed_text` → `onnx/text_model.onnx`

Result: ~2× embed throughput, no quality change. The combined-model code path is removed. `embed_text` (rare, search-time) loads its session lazily so the text tower isn't resident during bulk indexing. onnx-community exports already include dynamic batch axes.

### 2.3 Batched inference

Stage 2 accumulates decoded images into a batch of size **B** (default ~12, tunable), flushed on **full-or-timeout**, and runs one `session.run` with a `(B,3,H,W)` tensor. This is the largest CPU throughput lever for 300–1000-image bursts. Requires the dynamic-batch-axis ONNX (already present in onnx-community exports). Face detection/recognition is batched where the `face_id` API allows; otherwise per-image calls remain but run on the dedicated actor.

### 2.4 Execution provider + DirectML experiment

- Add `ort` EP features; auto-size `intra_threads` to **physical core count** instead of hardcoded 4.
- Register the **DirectML** EP to offload SigLIP onto the iGPU. The benefit is two-fold even if the iGPU isn't faster per-image: it moves embedding *off* the CPU so face inference and decoding keep the cores.
- DirectML is a **measured experiment with a CPU fallback** — never assumed. EP choice is a config knob.

### 2.5 Preprocessing & resize

- Replace the scalar per-pixel normalization loop with a vectorized `ndarray` fill.
- Switch embed resize `Lanczos3` → `Triangle` (CatmullRom for thumbnails).
- Decode JPEGs at **reduced scale** (`image` scaled decoding / DCT-domain downscale) since the result is immediately shrunk to 224–256px — also shrinks Stage 1 cost.

**Quality note:** 2.5 alters embedding *values* slightly and triggers a one-time re-embed. Acceptable: the library is not yet permanently indexed (still in experimentation). 2.1–2.4 are behavior-preserving.

---

## Scheduling & resource arbitration

Scheduling is expressed through the channel topology rather than a separate scheduler component:

- **Bounded channels** are the throttle (memory cap + backpressure).
- **One shared rayon pool** for all CPU decode/resize work, sized to physical cores, so decoding cannot oversubscribe the cores ORT needs.
- **EP-aware co-scheduling** via a single `enum ComputePlacement { Gpu, Cpu }` branch:
  - `Gpu` (SigLIP on DirectML): CPU is free → semantic and face stages run concurrently.
  - `Cpu` (SigLIP fallback): serialize the two model stages, interleaved at batch granularity, to avoid CPU thrashing.
- A **`PipelineConfig`** struct centralizes knobs: batch size B, channel depths, intra-threads, EP choice, resize filter.

---

## Measurement

Measurement is **step one** of implementation and is kept permanently, because integrated-GPU gains are uncertain and tuning (B, thread count, EP) must be empirical.

A **benchmark harness** processes a fixed sample folder end-to-end and emits:

- Per-stage timings: decode ms, embed ms, face ms, write ms.
- Overall images/sec across the burst.

Every change is validated against the harness baseline; B and thread-count are tuned from its output, not guessed.

---

## Testing

- **Unit:**
  - `DecodedImage` fan-out: one decode produces correct thumbnail + embed + face crops.
  - Batched embed of B images equals B single embeds within floating-point tolerance.
  - Vectorized preprocessing matches the old scalar loop within tolerance.
- **Integration:**
  - Pipeline processes a fixture folder; DB rows, index entries, thumbnails, and face crops are all produced.
  - Retry/failure paths still mark queue rows failed (preserves current `mark_failed` semantics).
- **Regression:**
  - Search returns the same top-k on a fixed set after the tower split (behavior-preserving check for 2.1–2.4).

---

## Rollout order

Each step is independently shippable and measured against the harness:

1. **Benchmark harness** (baseline numbers).
2. **Decode-once pipeline** (Stage 1 fan-out) — likely the biggest single win.
3. **Inference actor** (remove session mutex).
4. **Split towers.**
5. **Batched inference.**
6. **EP config + DirectML experiment** (keep CPU path if iGPU underperforms).
7. **Preprocessing/resize + scaled decode** (triggers one-time re-embed).
8. **Resource arbitration** (placement-aware concurrency) + final tuning pass.

Each step keeps the app working. If DirectML underperforms on the iGPU, the CPU path remains and nothing is lost.

---

## Risks & mitigations

- **DirectML on iGPU may not help or may be unstable.** Mitigation: measured experiment, CPU fallback, EP is a config knob.
- **`face_id` may not expose batched inference.** Mitigation: keep per-image face calls on the dedicated actor; still benefit from decode-once and concurrency.
- **Memory pressure from in-flight 24 MP buffers during 1000-image bursts.** Mitigation: bounded channels cap in-flight count; downscale early in Stage 1.
- **Batched dynamic-axis ONNX behaving differently from fixed-shape.** Mitigation: regression test (batched == single within tolerance) before adopting.
- **Merging two workers reduces independent pausability.** Accepted; coordinator can still gate stages if pause is needed later.
