# Subject Person Card Visual Redesign

## Context

`2026-07-01-subject-card-inline-tag-editing-design.md` added inline tag
add/remove and inline name editing to `SubjectPersonCardComponent`. To fit the
new tag list + add-tag input, that change split the card into a photo section
plus a separate `bg-card` footer strip below it. This kept the card
functional but regressed the visual design: the card lost its full-bleed
photo-overlay look, grew a plain white/light footer, and grew taller
(no longer a fixed `aspect-[3/4]` tile).

This spec keeps all functionality from the tag-editing work (inline name
edit, inline tag add/remove, merge-conflict dialog) but restores the card to
a single full-bleed photo tile with everything — name, subtitle, tags, and
the add-tag input — layered on top of the photo inside one dark scrim, no
separate footer block.

## Goals

- Name text is white with guaranteed contrast against the photo, in both
  light and dark app themes.
- The dark overlay covers the whole card (no separate light-colored footer
  region).
- Tag list and add-tag input remain fully functional, styled to sit inside
  the photo overlay rather than a separate strip.
- Card returns to a fixed `aspect-[3/4]` tile, which brings it back to its
  pre-footer height (no width change).
- Hover gives a clear "interactive" cue without changing the card's layout
  box (no dimension change that would reflow the grid) and without an
  effect that reads as the card shrinking.

## Non-goals

- No changes to `subject-tagging.composable.ts` business logic, merge-conflict
  flow, or the `EditableTextComponent` itself.
- No changes to how/where `SubjectPersonCardComponent` is used (report
  detail grid, etc.) beyond its own visual footprint.

## Design

### Structure

Collapse back to a single-element photo tile (matching the pre-tag-editing
structure), instead of the current photo-div + footer-div split:

```
.person-card (relative, aspect-[3/4], w-56, rounded-2xl, overflow-hidden)
  img.person-card-img (absolute inset-0, object-cover)
  .person-card-scrim (absolute inset-0, gradient, pointer-events-none)
  .person-card-content (absolute inset-x-0 bottom-0, p-3, flex-col gap-1.5)
    name (app-editable-text, white)
    subtitle (optional, white/70)
    tag pills (flex-wrap, glass style)
    add-tag row (input + button, glass style)
    tag error (if present)
```

### Scrim

Strengthen/extend the existing gradient so it has enough opaque room at the
bottom to host name + subtitle + tags + input legibly over any photo:

```css
background: linear-gradient(to top,
  rgba(8, 8, 11, 0.95) 0%,
  rgba(8, 8, 11, 0.75) 35%,
  rgba(8, 8, 11, 0.25) 60%,
  transparent 80%);
```

This is a fixed rgba overlay, independent of the app's light/dark theme, so
contrast for the white name/tag text against it is guaranteed the same in
both themes — the contrast that matters here is against the photo, not
against the app background.

### Tags and add-tag input

Restyle from the current solid `bg-secondary` (designed for the light
footer) to a frosted-glass treatment that reads as part of the photo
overlay:

- Tag pill: `bg-white/15 backdrop-blur-sm border border-white/20 text-white`
- Remove (×) button: `text-white/70 hover:text-destructive`
- Add-tag input: `bg-white/10 backdrop-blur-md border border-white/20
  text-white placeholder-white/50 rounded-full`
- Add button: `text-white hover:text-white/80`
- Tag error text: keep legible against the dark scrim (e.g.
  `text-destructive` still reads fine on the dark overlay; verify and adjust
  to a lighter destructive tint if needed)

Datalist and `stopPropagation` click/keydown handling on the tag/input
region are unchanged from the current implementation.

### Sizing

- `.person-card` returns to `w-56 aspect-[3/4]` (matches the pre-footer
  version). Removing the separate footer block is what restores the
  smaller/shorter card — no explicit width change needed.

### Hover

Replace the current `translateY(-4px)` lift (which, combined with the
shadow, reads as the card shrinking) with a static-position effect:

- Card position and box dimensions never change on hover (no transform on
  `.person-card` itself).
- Photo zooms in slightly: `.person-card-img { transform: scale(1.05) }` on
  hover, clipped by the card's `overflow-hidden`.
- A soft glow appears around the card edge on hover: increased
  `box-shadow` and/or a subtle `ring`/border brightening (e.g.
  `ring-2 ring-white/20`), added only on hover.
- Respect `prefers-reduced-motion: no-preference` guard already in place for
  the zoom transition.

## Testing

- Existing `subject-person-card.component.spec.ts` covers behavior (name
  edit, tag add/remove, merge dialog) and should continue to pass unchanged,
  since no template bindings or component logic change — only CSS classes
  and element structure inside the card.
- Manual visual check in both light and dark app theme: name/tags legible,
  no white/light footer visible, hover shows zoom + glow with no layout
  shift, card is back to the shorter aspect-[3/4] footprint.
