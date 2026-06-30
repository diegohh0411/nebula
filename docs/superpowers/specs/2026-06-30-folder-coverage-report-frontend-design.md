# Folder Coverage Report Frontend Design

## Overview
This design implements the frontend for the Folder Coverage Report, a feature allowing users to check which subjects from a specific target list (roster) are present or missing within a selected folder, as well as identifying non-target subjects found in that folder.

## Route and Navigation
- **Route:** A new route `/reports` will be added to `src/app/app.routes.ts`.
- **Sidebar:** A "Reports" navigation link will be added to `src/app/components/sidebar/sidebar.component.html` and `.ts`, making the page easily accessible.

## Service Additions (`PhotoService`)
- Add `getFolderCoverage(folderId: number, tagIds: number[]): Promise<CoverageReport>` to call the Tauri `get_folder_coverage` command.
- Add interfaces for `CoverageSummary`, `SubjectCoverage`, and `CoverageReport` based on the backend structs.

## UI Component (`ReportsComponent`)
**File:** `src/app/components/reports/reports.component.ts`

### 1. Controls
The top of the page will contain inputs to configure the report:
- **Folder Selector:** A native `<select>` dropdown populated by `this.photos.folders()`.
- **Target Tags:** A UI (e.g. multi-select or list of toggleable pills) populated by `this.photos.listTags()`.
- **Run Button:** (Optional/Implied) The report can auto-generate on selection or require a "Run Report" button. We will use auto-generate when both a folder and at least one tag are selected.

### 2. Summary
A brief text summary displaying `CoverageSummary.present_targets` out of `CoverageSummary.total_targets`.

### 3. Sections
The report will display three distinct vertical sections:
1. **Missing:** Subjects from the target list that have 0 frequency in the folder.
2. **Present:** Subjects from the target list that have > 0 frequency in the folder.
3. **Others Found:** Subjects found in the folder that are not part of the target list.

### 4. Interactive Cards (Card Mapping)
- The results will use the existing `SubjectPersonCardComponent`.
- **Data Mapping:** Since the backend returns `SubjectCoverage` (which lacks `thumbnail_face_id`), the frontend will cross-reference the returned `subject_id` with `this.photos.subjects()` to retrieve the full `Subject` object required by `SubjectPersonCardComponent`.
- **Frequency Badge:** To display the number of photos without altering the `SubjectPersonCardComponent`, the frontend will inject a pseudo-tag (e.g., `{ id: -1, name: "15 photos", added_at: 0 }`) into the `tags` array of the `SubjectMatch` passed to the component.

## Dependencies and State
- The component will depend on `PhotoService` for data.
- It will require `this.photos.loadSubjects()` to be executed (or rely on the app having loaded it) so that full subject objects are available for the cards.