# TT-93 — Decode images at reduced resolution in the inference pipeline

- **Notion task:** TT-93 — https://app.notion.com/p/396e954db47681ad8951fea67246abdb
- **Status:** Detailed → (to be moved to In development on implementation)
- **Date:** 2026-07-07
- **Branch:** `optimize-inference-pipeline`

## Problem

`pipeline/decoded_image.rs::load_decoded` calls `image::open(path)`, decoding the
**entire original image**. A 24–48MP JPEG becomes a 70–140MB bitmap. Every
downstream consumer only ever uses a shrunken version:

- SigLIP embedding resizes to 224–256px (`vision/preprocess.rs`).
- The face detector resizes to its own fixed input (~640px).
- Sharpness measurement crops face regions from the bitmap (`pipeline/face_actor.rs`).

Full-size decode is the single biggest per-image cost on the import critical path,
and it inflates the cost of every downstream resize proportionally.

## Key discovery — the mechanism already exists

The `media` slice already solved this problem for the **preview/thumbnail** path.
`media/preview.rs::decode_at_most(path, target_long_edge)` performs DCT-scaled JPEG
decoding via the `jpeg-decoder` crate (already a dependency), with a full-decode
fallback for non-JPEG formats and unsupported pixel formats. It is live,
production code:

- Called by `write_preview` (256px) and `write_thumbnail` (1600px→800px).
- Runs inside `PreviewService`, started at boot (`app/mod.rs`) and fed by the indexer.
- Covered by three unit tests.

The inference path (`decoded_image.rs`) never received the same treatment. TT-93 is
therefore not a green-field feature but an application of an existing, proven
optimization to the second decode path.

**Consequence:** we do NOT add `zune-jpeg`/`turbojpeg` (as the task text tentatively
suggested). Doing so would introduce a second JPEG decoder alongside `jpeg-decoder`
and duplicate tested logic. No `Cargo.toml` change is required.

## Design

### 1. Extract a shared decode primitive → `media/decode.rs`

Move `decode_at_most` and its private helper `decode_jpeg_scaled` out of
`media/preview.rs` into a new `media/decode.rs` module. This makes "decode an image
bounded to a maximum long edge" a first-class primitive owned by the `media` slice,
consumed by both the preview path and the inference path, instead of one path
reaching into the other's module.

- Add `pub mod decode;` to `media/mod.rs`.
- Move the three existing decode tests (`decode_at_most_scales_large_jpeg_down`,
  `decode_at_most_does_not_upscale_small_image`, `decode_at_most_errors_on_missing_file`)
  into `media/decode.rs`.
- `preview.rs` changes only its call sites: `decode_at_most(...)` →
  `crate::media::decode::decode_at_most(...)`. No behavior change to previews; they
  keep their own final resize (`CatmullRom` to 256/800px).

`pipeline → media` is an established cross-slice dependency (`pipeline/mod.rs`
already calls `crate::media::thumbnail::*`), so `pipeline → media::decode` is
consistent with the slice architecture rules (cross-slice access via public API).

### 2. Rewrite `load_decoded` to bound the decode

```rust
pub const DECODE_MAX_LONG_EDGE: u32 = 2048;

pub fn load_decoded(image_id: i64, path: &Path) -> Result<DecodedImage> {
    // Coarse DCT-scaled decode (JPEG) or full decode + fallback (other formats).
    let img = crate::media::decode::decode_at_most(path, DECODE_MAX_LONG_EDGE)?;
    // decode_at_most rounds JPEG down to a power-of-two whose long edge is >= target,
    // so it can return up to ~2x the bound; clamp to the exact bound. Images already
    // at or below the bound are returned untouched (no resize pass, no upscaling).
    let full = if img.width().max(img.height()) > DECODE_MAX_LONG_EDGE {
        img.resize(
            DECODE_MAX_LONG_EDGE,
            DECODE_MAX_LONG_EDGE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    Ok(DecodedImage { image_id, full: Arc::new(full) })
}
```

- `DECODE_MAX_LONG_EDGE = 2048` is a named constant so the bound can be tuned.
- The exact resize pass is required because `decode_at_most` only rounds to a
  power-of-two ≥ target; without it the ≤2048 acceptance criterion is not met.
- `Triangle` (bilinear) filter: fast, adequate for output feeding ML models rather
  than human eyes. (Previews deliberately keep the sharper `CatmullRom`.)

### 3. `DecodedImage` contract is unchanged

`DecodedImage { image_id, full: Arc<DynamicImage> }` stays identical, so
`embed_actor`, `face_actor`, the sharpness crop, and any other consumer need no
changes.

## Why the downstream consumers are safe

- **Face detection / embedding:** both already resize `full` down to their own fixed
  input sizes (≤640px, 224–256px). A 2048px source still downsamples ≥3× into the
  detector — equal or greater detail than they see today post-resize.
- **Sharpness crop (`face_actor.rs`):** bboxes are stored **relative (0..1)** and
  multiplied by the *live* image dimensions at crop time
  (`x = bbox.x1 * img.width()`, etc.). Downscaling the image needs no coordinate
  translation — the crop lands on the same region at lower resolution. Verified in
  code.
- **Previews/thumbnails:** generated by the `media` slice directly from the original
  file; they do **not** consume `DecodedImage`. Unaffected.

## Risks & mitigations

| Risk | Mitigation |
|---|---|
| Tiny-face recall: faces <1% of a 48MP frame lose pixels | 2048px is still above the detector's input size; detector sees ≥ today's detail. Validate face counts on a face-heavy library before merge. |
| Bbox coordinate drift | Bboxes are relative (0..1); no translation needed. Verified in `face_actor.rs`. |
| Preview/thumbnail regression from the extraction | Pure code move; preview call sites and final resize filters unchanged. Existing preview tests must still pass. |

## Testing

**Unit (in `pipeline/decoded_image.rs`):**
- Oversized input → returned long edge ≤ 2048 (synthesize a >2048px image).
- ≤2048 input → returned untouched (dimensions preserved, no upscale).
- Keep the existing missing-file error test.

**Unit (in `media/decode.rs`):** the three relocated `decode_at_most` tests continue
to pass unchanged.

**Regression:** full `cargo test` for the `media` and `pipeline` slices.

## Acceptance criteria (from TT-93)

- [ ] `load_decoded` returns an image no larger than 2048px on the long edge for
      oversized inputs; smaller inputs are untouched.
- [ ] Existing `decoded_image.rs` unit tests pass; new tests cover the downscale
      bound and the ≤2048 passthrough.
- [ ] Face count on a reference library matches the pre-change baseline (±0).
- [ ] Measured wall-time improvement on a bulk import of camera-resolution JPEGs
      (log `[embed]`/`[face]` timings before/after).

## Files touched

- `src-tauri/src/media/decode.rs` — **new** (relocated `decode_at_most` + helper + tests)
- `src-tauri/src/media/mod.rs` — add `pub mod decode;`
- `src-tauri/src/media/preview.rs` — update `decode_at_most` call sites; remove moved code
- `src-tauri/src/pipeline/decoded_image.rs` — bounded `load_decoded` + constant + tests
- `src-tauri/Cargo.toml` — **no change** (`jpeg-decoder` already present)
