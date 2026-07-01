# Report UX Improvements Design

## Overview
This document outlines a set of UX improvements for the newly added Reports feature in the application, ensuring it feels consistent with the rest of the application and provides a polished user experience.

## Improvements

### 1. Sidebar Icon Update
- Replace the current `file-text` icon for the Reports section in the sidebar with `file-bar-chart`.
- Ensures better visual association with analytics/reporting.
- **Icon Registration:** Ensure `FileBarChart` (and `Trash2` used in the reports list) are correctly imported and added to the `APP_ICONS` registry in `src/app/app-icons.ts`.

### 2. Dark Mode Support
- All Reports UI elements (Reports List, Report Creation form, and Report Detail view) must correctly implement Tailwind dark mode variants (e.g., `dark:bg-slate-900`, `dark:text-white`).
- Validates that the forms, inputs, buttons, and text contrast well when the user has dark mode enabled.

### 3. Report Detail CRUD Actions
- **Delete Report:** Add a "Delete Report" button (with destructive styling, e.g., red text/border) in the header of the Report Detail view.
  - Upon successful deletion, redirect the user back to the `/reports` route.
- **Edit Name:** Add an "Edit Name" button adjacent to the Delete button. 
  - Clicking this could either convert the title into an inline editable input or open a prompt/dialog to change the report's name. (Will use inline edit or prompt for simplicity).

### 4. Subject Frequency Display
- **Current Issue:** The frequency count of a subject is currently injected as a fake `Tag` object, which appears visually alongside actual roster tags.
- **Solution:** 
  - Update `SubjectPersonCardComponent` to accept a new optional input `subtitle` (`readonly subtitle = input<string>()`).
  - Add markup in `subject-person-card.component.html` to display the `subtitle` directly beneath the `person-card-name` with secondary styling (smaller text, muted color).
  - In `ReportDetailComponent`, remove the fake tag generation.
  - The frequency count (`item.frequency`) is already computed and returned by the backend. The frontend will simply format it into a string (e.g., "Appears in X photo(s)") and pass it to the card's `subtitle` input.

## Data Flow & Architecture
- No changes to backend database schemas or core services.
- `PhotoService` already supports `deleteSavedReport`. We will need to ensure it also has a method for renaming a report (`updateSavedReport` or similar) to support the Edit Name functionality. If it doesn't exist, we will add it to the Rust backend and the Angular service.

## Testing & Verification
- Verify the sidebar icon displays correctly (no unregistered icon errors).
- Verify dark mode colors match the rest of the app.
- Ensure clicking Delete successfully removes the report and navigates away.
- Ensure editing the name persists properly to the database.
- Verify the frequency subtitle displays correctly on the cards and the fake tags are gone.
