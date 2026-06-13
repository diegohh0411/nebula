# Design: PageContainer Layout Primitive

## Goal
Add a reusable `PageContainer` layout primitive to standardize page width and padding across the application.

## Variants
- `text`: Limits width to `max-w-4xl` and centers the content (suitable for Settings, etc.).
- `full`: Allows the content to span the full width of the container (suitable for image galleries/grids).

## Implementation
- Created `src/app/libs/ui/layout/page-container/`
- Implemented as an Angular component with `ng-content` projection.
- Uses Tailwind CSS for all styling.
