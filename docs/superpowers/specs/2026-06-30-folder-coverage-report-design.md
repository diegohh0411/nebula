# Folder Coverage Report Design

## Overview
A new "Reports" feature designed to solve the "summer camp" use case: ensuring specific groups of people (e.g., a cabin roster) are photographed at least once per folder/upload. It allows users to cross-reference the subjects found in a specific folder against one or more roster tags, easily surfacing who is missing.

## UI/UX
- **Navigation:** A new "Reports" section in the main app navigation.
- **Configuration Controls:**
  - **Folder Select:** Dropdown to select a single folder.
  - **Tag Select:** Multi-select input to choose one or more target tags (e.g., "Cabin 4", "Cabin 5").
- **Report View:**
  - **Summary Statistic:** e.g., "Overall Coverage: 21 of 22 present".
  - **Missing Section (Priority):** A flattened list of all subjects belonging to the selected tags who have exactly 0 photos in the folder. Highlighted/styled to grab attention.
  - **Present Section:** A flattened list of subjects from the selected tags who have >= 1 photo in the folder, displaying their frequency count.
  - **Others Found Section:** A list of subjects who have >= 1 photo in the folder but *do not* belong to any of the selected tags.

## Backend Architecture

### New Tauri Command
`get_folder_coverage(folder_id: i64, tag_ids: Vec<i64>) -> CoverageReport`

### Data Structures
```rust
pub struct CoverageReport {
    pub summary: CoverageSummary,
    pub missing_targets: Vec<SubjectCoverage>,
    pub present_targets: Vec<SubjectCoverage>,
    pub others_found: Vec<SubjectCoverage>,
}

pub struct CoverageSummary {
    pub total_targets: usize,
    pub present_targets: usize,
}

pub struct SubjectCoverage {
    pub subject_id: i64,
    pub name: String,
    pub frequency: usize,
}
```

### Business Logic
1. **Fetch Target Roster:** Query the `subject_tags` and `subjects` tables to get a distinct list of all subjects that have at least one of the provided `tag_ids`.
2. **Fetch Folder Frequencies:** Query the `images` and `faces` tables for the given `folder_id` to get a frequency count of all `subject_id`s in that folder.
3. **Categorize:**
   - Iterate through the Target Roster. If their frequency is 0, add to `missing_targets`. If > 0, add to `present_targets`.
   - Iterate through the Folder Frequencies. If a `subject_id` is *not* in the Target Roster, add to `others_found`.

## Error Handling
- Handle cases where a folder has no images/faces.
- Handle cases where selected tags have no subjects assigned.
- Return standard Tauri `Result<_, Error>` strings for UI consumption.
