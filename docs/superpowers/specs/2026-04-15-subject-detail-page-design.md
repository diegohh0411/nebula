# Subject Detail Page Design

Add a dedicated Subject Detail page to Nebula, enabling users to navigate from face bounding boxes or PeopleView cards to a full page where they can rename subjects, browse their photos, and choose a representative face thumbnail.

## Routing

Introduce Angular Router, replacing the existing signal-based `currentView` navigation.

### Routes

| Path | Component | Purpose |
|------|-----------|---------|
| `/` | GalleryComponent | Default gallery view |
| `/people` | PeopleViewComponent | Grid of all subjects |
| `/subject/:id` | SubjectDetailComponent | Subject detail with photos |
| `/subject/:id/face-picker` | FacePickerComponent | Choose representative face |

### Migration from Signal Nav

- Remove `currentView` signal from `PhotoService`
- `AppComponent` template replaces `@if` conditionals with `<router-outlet>`
- `SidebarComponent` uses `routerLink` and derives active state from `Router`
- The lightbox overlay sits above `<router-outlet>` and is unaffected by route changes

### Navigation Flows

- Sidebar "All Photos" → `/`
- Sidebar "People" → `/people`
- PeopleView card click → `/subject/:id`
- Lightbox face bounding box click → close lightbox, navigate to `/subject/:subjectId`
- Lightbox sidebar person click → close lightbox, navigate to `/subject/:subjectId`
- Subject detail back button → `Location.back()`
- 3-dot menu "Choose Representative Face" → `/subject/:id/face-picker`
- Face picker back button → `Location.back()`

## Backend Changes

No schema changes. The `thumbnail_face_id` column already exists in `subjects`.

### New Tauri Commands

**`get_face_crop(face_id: i64) -> String`**
Generates a 200x200 cropped face image from the original image using the face's bounding box. Caches the result in the thumbnail cache directory (`{data_dir}/thumbnails/face-crops/{face_id}.webp`). Returns the absolute path for the frontend to convert via `convertFileSrc`.

**`set_subject_thumbnail(subject_id: i64, face_id: i64) -> ()`**
Updates `subjects.thumbnail_face_id` to the given face ID. Validates that the face belongs to the subject.

**`get_subject_photos(subject_id: i64) -> Vec<SearchResult>`**
Returns all images containing faces belonging to the given subject. Joins `faces` → `images` and maps to `SearchResult` (reuses existing model).

**`get_subject_detail(subject_id: i64) -> SubjectDetail`**
Returns a subject with its photo count and face count. New response model:

```rust
pub struct SubjectDetail {
    pub subject: Subject,
    pub photo_count: i64,
    pub face_count: i64,
}
```

### Auto-Selection of Representative Face

When a subject is created or when faces are assigned during clustering, if `thumbnail_face_id` is NULL, the embedding worker selects the face with the largest bounding box area (`bbox_w * bbox_h`) as the default representative. This ensures PeopleView cards always display a face crop immediately.

## Frontend Components

### New: SubjectDetailComponent (`/subject/:id`)

Layout (top to bottom):
- **Header**: Back button (Lucide `arrow-left`), representative face circle (72px, accent border), subject name with pencil edit icon, photo/face counts, 3-dot menu (Spartan popover dropdown)
- **3-dot dropdown menu** (Spartan): Single option "Choose Representative Face" with Lucide `star` icon. Navigates to `/subject/:id/face-picker`.
- **Photo grid**: All photos containing this subject, using the existing justified gallery layout. Clicking a photo opens the lightbox.

Name editing: Pencil icon toggles inline edit mode (input field replaces the name text, Enter saves, Escape cancels). Calls existing `nameSubject` command.

Uses Spartan.ng components for the dropdown menu.

### New: FacePickerComponent (`/subject/:id/face-picker`)

Layout:
- **Sub-header**: Back button (Lucide `arrow-left`), title "Choose Representative Face"
- **Instruction text**: "Select the face you want to appear on the People card."
- **Face crop grid**: Wrapping grid of face crop images (100x100, rounded corners). Each crop has a star button (Lucide `star` icon) in the top-right corner. The active representative gets an accent border and filled star. Unselected crops have an unfilled star.

Interaction: Clicking a star calls `set_subject_thumbnail`, then navigates back to `/subject/:id`.

### Updated: PeopleViewComponent

- Remove inline renaming (delete `editingId`, `editName`, `startEdit`, `saveEdit`, `cancelEdit` and the edit-mode template block)
- Card click navigates to `/subject/:id` via router
- Display actual face crop thumbnail: load the crop path from the subject's `thumbnail_face_id` via `get_face_crop`, render with `convertFileSrc`. Fall back to placeholder avatar if no thumbnail is set.

### Updated: LightboxComponent

- Face bounding box click → call `close()`, then navigate to `/subject/:subjectId`
- Sidebar person item click → call `close()`, then navigate to `/subject/:subjectId`

### Updated: SidebarComponent

- "All Photos" uses `routerLink="/"`
- "People" uses `routerLink="/people"`
- Active state derived from current route via `Router` service instead of `currentView` signal

### Updated: AppComponent

- Template replaces `@if`/`@else` conditional rendering with `<router-outlet>`
- Lightbox overlay remains outside the router-outlet as a global overlay
- Removes `PeopleViewComponent` from imports (handled by routing)

## Data Flow

### Face Crop Lifecycle

1. Face detected → bounding box stored in `faces` table
2. Clustering assigns face to subject → if subject's `thumbnail_face_id` is NULL, auto-select largest face
3. Frontend requests crop via `get_face_crop(face_id)` → backend generates 200x200 WebP, caches it, returns path
4. Subsequent requests serve from cache (check existence before generating)
5. Frontend converts path to asset URL via `convertFileSrc`

### Navigation from Lightbox

1. User clicks face bounding box or sidebar person item
2. Lightbox closes (existing view transition animation)
3. Router navigates to `/subject/:id`
4. SubjectDetailComponent loads data on init via `get_subject_detail` and `get_subject_photos`

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Subject not found (invalid ID) | Navigate back to `/people` with a brief error state |
| No faces for subject | Face picker shows "No face detections yet" empty state |
| No photos for subject | Photo grid shows "No photos yet" empty state |
| Face crop generation failure | Fall back to placeholder avatar emoji on the PeopleView card |
| Invalid face ID in `set_subject_thumbnail` | Return error, show toast/snackbar feedback |

## UI Libraries

- **Spartan.ng**: Dropdown menu (popover-based), buttons, inputs
- **Lucide Angular**: Icons (`arrow-left`, `pencil`, `star`, `ellipsis-vertical`)
- **Tailwind CSS**: Styling consistent with existing patterns

## Out of Scope

- Subject merging (future feature)
- Subject deletion
- Subject types (pet/mascot support)
- Fixing the fuzzy search bug (separate issue)
- Drag-and-drop face crop reordering
