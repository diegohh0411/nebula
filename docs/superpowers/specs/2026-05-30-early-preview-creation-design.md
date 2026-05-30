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

1. **Stage 1 (Decode) completes:** We have the `DecodedImage` in memory.
2. **Generate Thumbnail:** We immediately spawn a blocking task to resize the image and save it to the `thumbnails` directory.
3. **Database Update:** Once written, update the `thumbnail_path` in the `images` table.
4. **First Emit:** Emit the `image_updated` event to the frontend. This signals that the UI can now display the preview, even though ML analysis is still pending.
5. **Stage 2 (Inference) completes:** Embeddings and faces are saved to the DB.
6. **Second Emit:** Emit the `image_updated` event again. This ensures the frontend knows the image is fully processed (e.g. for search or status indicators).

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