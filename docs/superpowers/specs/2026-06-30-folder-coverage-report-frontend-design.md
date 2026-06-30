# Folder Coverage Report & Saved Reports Design

## Overview
This design expands the Folder Coverage Report to allow users to create, save, and run multiple report configurations. Users can select a folder and a set of target tags, save this configuration to the database, and run it to see which subjects are present, missing, or found outside the target list.

## Database & Backend (New Additions)
### Schema
- `saved_reports` table:
  - `id` (INTEGER PRIMARY KEY)
  - `name` (TEXT)
  - `folder_id` (INTEGER)
  - `added_at` (INTEGER)
- `saved_report_tags` table:
  - `report_id` (INTEGER)
  - `tag_id` (INTEGER)

### Tauri Commands
- `create_saved_report(name: String, folder_id: i64, tag_ids: Vec<i64>) -> Result<SavedReport>`
- `list_saved_reports() -> Result<Vec<SavedReport>>`
- `delete_saved_report(id: i64) -> Result<()>`
- (The existing `get_folder_coverage(folder_id, tag_ids)` will remain as the engine to run the report).

## Route and Navigation
- **Route:** `/reports` (List view) and `/reports/:id` (Run/View view).
- **Sidebar:** A "Reports" navigation link in `SidebarComponent`.

## UI Component (`ReportsComponent` & `ReportDetailComponent`)

### Flow B Architecture
1. **Reports Index (`/reports`):**
   - Displays a grid or list of saved report cards.
   - Each card shows the report name, folder name, and target tags.
   - Includes a "Delete" button on each card.
   - A prominent "Create New Report" button opens a modal or navigates to a builder view to select a Folder, select Tags, name the report, and save it.

2. **Report Detail (`/reports/:id`):**
   - Fetches the saved report configuration (folder_id, tag_ids).
   - Automatically calls `getFolderCoverage(folder_id, tag_ids)`.
   - Displays the report name as the header.

### Results Display (Inside Report Detail)
- **Summary:** Quick text showing "Targets present: X / Y".
- **Sections:** Three distinct vertical sections:
  1. **Missing:** Target subjects with 0 frequency.
  2. **Present:** Target subjects with > 0 frequency.
  3. **Others Found:** Non-target subjects found in the folder.
- **Interactive Cards:**
  - Uses the existing `SubjectPersonCardComponent`.
  - Frontend looks up full `Subject` data from `photoService.subjects()`.
  - **Frequency Badge:** A pseudo-tag (e.g. `{ id: -1, name: "15 photos", added_at: 0 }`) is injected into the `tags` array so the card naturally displays the photo count without needing modifications.