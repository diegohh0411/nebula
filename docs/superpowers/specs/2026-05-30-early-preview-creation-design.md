# Early Preview Creation (TT-9)

## Goal
Make images appear in the gallery as soon as possible after discovery, rather than waiting for the potentially slow ML inference pipeline (SigLIP/Face detection) to finish.

## Architecture

The inference pipeline (`src-tauri/src/pipeline/mod.rs`) processes images in two main stages:
1. **Decode:** Load the image from disk into memory.
2. **Inference:** Dispatch to the embedder (SigLIP) and face detector, then write results to DB.

Currently, thumbnail generation happens at the very end of Stage 2.

We will move thumbnail generation to happen immediately after Stage 1 (Decode) succeeds.

## Data Flow & Events

Up to two `image_updated` events may be emitted per image. **Their order is not guaranteed** — see the Ordering Contract section below. The Stage 1 emit is conditional (only fires on success); Stage 2 always emits.

**Stage 1 path (detached, concurrent with Stage 2):**
1. Stage 1 (Decode) completes — `DecodedImage` is in memory.
2. A `tokio::spawn` thumbnail task is launched (not awaited).
3. Inside that task: thumbnail is written to disk, `thumbnail_path` is updated in the DB.
4. **First Emit ("preview ready"):** `image_updated` is emitted only if both the write and DB update succeed.

**Stage 2 path (main loop):**
5. Stage 2 (Inference) completes — embeddings and faces are saved to the DB.
6. **Second Emit ("analysis complete"):** `image_updated` is emitted unconditionally.

Steps 2–4 and steps 5–6 run concurrently. Either emit may arrive at the frontend first.

## Ordering Contract (Option A — adopted in TT-12)

Every `image_updated` event means **"refetch this image"** — nothing more. Handlers must:
- Schedule a re-fetch in response to notifications; coalescing rapid events is fine.
- Tolerate `thumbnail_path = null` (thumbnail write may not have completed yet).
- Not assume that any particular emit implies a specific set of fields are populated.

The frontend (`PhotoService`) uses `auditTime(2000)` + unconditional `refreshImages()`, which satisfies this contract: rapid successive emits are coalesced into one refresh, and a late Stage 1 emit triggers a second refresh that displays the thumbnail once it is written.

## Code Changes

- In `src-tauri/src/pipeline/mod.rs`:
  - Move the thumbnail generation block (which uses `crate::thumbnail::write_thumbnail_from_image`) from the end of the Stage 2 loop.
  - Insert it into the loop that collects the decoded images (`let mut decoded = Vec::new(); for h in handles { ... }`), right after a successful decode.
  - Add the first `image_updated` emit inside that success block.
  - Keep a second `image_updated` emit at the end of the Stage 2 loop.

## Error Handling

If thumbnail generation fails, we log the error but proceed with Stage 2 inference. (An image without a thumbnail can still be analyzed and searched).

## Out of Scope

- Changing the indexer to generate thumbnails (keep indexer fast).
- Changing the frontend gallery layout logic.