# TT-80 — Lightbox prev/next navigation from scoped views

**Status:** Design approved
**Date:** 2026-06-16
**Task:** TT-80 — *fix: lightbox prev/next arrows don't navigate from scoped views (merge modal & subject detail)*

## Problem

When the lightbox is opened from a **scoped** photo set — the merge / review-duplicate
modal grid, or a subject's detail view — the prev/next arrows (keys and on-screen) don't
move between that view's images. The lightbox opens on the clicked image, but navigation is
a no-op.

### Root cause

`PhotoService.navigateLightbox()` resolves its candidate list from **global** gallery state:

```typescript
const allImages = this.searchResults() ?? this.images();
const idx = allImages.findIndex(i => idOf(i) === currentId);
if (idx === -1) return; // scoped image isn't in the global list, so nav bails
```

Scoped images (per-subject photos, merge-grid faces) aren't present in
`searchResults()`/`images()`, so `findIndex` returns `-1` and navigation silently does
nothing. There is no notion of "the list the lightbox was opened from."

### Where it shows up

- **Merge / Review Possible Duplicate modal** — `merge-photo-grid.component.ts` (per cell).
  Used twice in `merge-review.component.html` (one grid per subject, `photosA` / `photosB`).
- **Subject detail view** — `subject-detail.component.html` renders `<app-photo-grid>` per
  justified row. The leaf grid only ever receives **one row** (`row.images`); the full
  ordered set (`subjectPhotos()`) lives in the container.

The leaf `photo-grid` never has the full list — so the navigation context must come from the
container that owns the ordered set.

## Approach (chosen: full unification)

Make the lightbox **always carry its own source list**. A single signal holds the exact
ordered list the user is browsing: set at open, read by navigate, cleared on close. There is
**no global-state fallback** — every opener supplies its list.

A `required` input on `photo-grid` makes "every opener supplies a list" a compile-time
guarantee. That is the safety net that makes removing the global fallback safe: any host that
forgets to wire the list fails to compile rather than silently breaking navigation.

### Rejected alternatives

- **Optional nav-context with global fallback** — smaller diff, but keeps two code paths and
  leaves the fallback as a latent footgun. The user preferred the cleaner long-term shape.
- **Out-of-band source signal set in `ngOnInit`/`ngOnDestroy`** — lifecycle decoupled from
  open/close; easy to leave stale context (gallery → subject → gallery reuses subject photos).

## Design

### Service — `photo.service.ts`

- Add `readonly lightboxItems = signal<(Image | SearchResult)[]>([])`.
- `openLightbox(img, items)` — `items` is now **required**; sets `selectedImage` and
  `lightboxItems`.
- `navigateLightbox(direction)` — index into `lightboxItems()` only; wrap-around with modulo;
  no-op when the list is empty or the current image isn't found.
- `closeLightbox()` — also clears `lightboxItems` (set to `[]`).
- Add `readonly galleryImages = computed(() => this.dayGroups().flatMap(g => g.images))` —
  the full visual-ordered list for the main gallery / search results.
  - **Side benefit:** gallery/search navigation now follows the on-screen day-grouped order
    instead of the raw `images()` signal order.

### Components

- **`photo-grid`** — add `@Input({ required: true }) navigationItems: (Image | SearchResult)[]`;
  `onPhotoClick(img)` calls `openLightbox(img, this.navigationItems)`.
  - `gallery.component.html`: `<app-photo-grid ... [navigationItems]="photos.galleryImages()" />`
  - `subject-detail.component.html`: `<app-photo-grid ... [navigationItems]="subjectPhotos()" />`
- **`merge-photo-grid`** — extract the cell→lightbox mapping into a helper, build the full
  mapped list (in `images` order), and have `onClick` call
  `openLightbox(mappedClicked, mappedList)`. `merge-review`'s two grids each pass their own
  list automatically because each has its own `images` input.

### Data flow

```
container (owns ordered list)
  └─ grid [navigationItems]=<full ordered list>
       └─ click → openLightbox(clicked, list)
            └─ lightboxItems := list ; selectedImage := clicked
                 └─ navigateLightbox(±1) walks lightboxItems()
                      └─ closeLightbox() clears lightboxItems
```

### Edge cases

- Empty list / current item not in list → `navigateLightbox` is a guarded no-op.
- Collapsed day rows: **out of scope** — navigation still spans those images.
- Image deleted/merged while the lightbox is open: **out of scope**.

## Testing

- **Service unit tests:** navigate wraps in both directions within a supplied list; no-op on
  empty list and on an item not present in the list; `closeLightbox` clears `lightboxItems`.
- **`merge-photo-grid.spec`:** clicking a cell opens the lightbox with the full mapped list;
  arrows cycle within that subject's photos.
- **Regression:** existing search/gallery lightbox navigation still works (now via
  `galleryImages`), exercised through the gallery path.

## Files touched

- `src/app/services/photo.service.ts`
- `src/app/components/photo-grid/photo-grid.component.ts`
- `src/app/components/gallery/gallery.component.html`
- `src/app/components/subject-detail/subject-detail.component.html`
- `src/app/components/merge-photo-grid/merge-photo-grid.component.ts`
- `src/app/components/merge-photo-grid/merge-photo-grid.component.spec.ts` (extend)
- `src/app/services/photo.service.spec.ts` (or equivalent service spec)

## Acceptance criteria

- Opening the lightbox from the merge modal lets you arrow through that subject's photos.
- Opening the lightbox from a subject detail view lets you arrow through that subject's photos.
- Global gallery / search lightbox navigation is unchanged (behaviourally).
