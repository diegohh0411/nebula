# TT-79: Smart-crop Merge Modal Design

## Goal
Make the merge modal's side-by-side subject comparison faster and more reliable by showing larger, face-centered thumbnails that still retain a little image context.

## Background
The merge modal (`MergeReviewComponent`) currently shows two columns of subject photos using `app-photo-grid` with a fixed `[cellSize]="104"`. Faces are often too small to compare confidently. We already store face bounding boxes in the database and have commands to fetch faces per subject/image, so we can smart-crop each thumbnail around the subject's face.

## Approach
**Smart-crop thumbnails, one grid cell per face occurrence.**

If a subject appears twice in the same photo, that photo appears twice in the grid — once per face — each smart-cropped to a different face.

## Backend

### New command
`get_subject_photos_with_faces(subject_id: i64) -> Vec<SubjectPhotoFace>`

### New type
```rust
pub struct SubjectPhotoFace {
    pub image_id: i64,
    pub path: String,
    pub thumbnail_path: Option<String>,
    pub preview_path: Option<String>,
    pub date_taken: Option<i64>,
    pub mtime: i64,
    pub face_bbox: FaceBBox,
}

pub struct FaceBBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
```

### Query behavior
- Join `images` with `faces` where `faces.subject_id = ?`.
- Return one row per face.
- Order by `date_taken DESC`, falling back to `mtime DESC` when `date_taken` is null.

## Frontend

### New type
```ts
export interface SubjectPhotoFace {
  image_id: number;
  path: string;
  thumbnail_path: string | null;
  preview_path: string | null;
  date_taken: number | null;
  mtime: number;
  face_bbox: FaceBBox;
}

export interface FaceBBox {
  x: number;
  y: number;
  w: number;
  h: number;
}
```

### New service method
`PhotoService.getSubjectPhotosWithFaces(subjectId: number): Promise<SubjectPhotoFace[]>`

### New component
`MergePhotoGridComponent`:
- Dedicated grid for the merge modal.
- Responsive square-ish cells using `clamp(140px, 25vw, 220px)`.
- Each cell renders the original image with `object-fit: cover`.
- `object-position` is computed from the face bbox center plus 20% context padding.
- Click opens the lightbox on the underlying image.

### Smart-crop math
```
cx = face_bbox.x + face_bbox.w / 2
cy = face_bbox.y + face_bbox.h / 2
focusX = cx * 100%
focusY = cy * 100%
```
Applied via inline CSS variables `--focus-x` and `--focus-y`.

### Modal layout updates
- Replace `app-photo-grid` with `app-merge-photo-grid` in `merge-review.component.html`.
- Update count label from "X photos" to "X faces".
- Keep modal `width: min(90vw, 900px)` and `height: 85vh`.
- Ensure the photo area scrolls internally and action buttons remain pinned at the bottom.

## Edge cases
- Images with no face for the subject are not returned by the new endpoint.
- If a subject has zero faces, show an empty state with text like "No faces found".
- Window resize is handled by CSS `clamp()`; no JS resize listener needed.
- If a face bbox is missing or invalid, fall back to center crop.

## Testing
- Unit tests for bbox-to-focus-point math.
- Update `MergeReviewComponent` tests to use `SubjectPhotoFace`.
- Backend test for the new repo query ordering and flat face expansion.
