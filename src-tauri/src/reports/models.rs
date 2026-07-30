use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoverageReport {
    pub summary: CoverageSummary,
    pub missing_targets: Vec<SubjectCoverage>,
    pub present_targets: Vec<SubjectCoverage>,
    pub others_found: Vec<SubjectCoverage>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoverageSummary {
    pub total_targets: usize,
    pub present_targets: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubjectCoverage {
    pub subject_id: i64,
    pub name: String,
    pub frequency: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SavedReport {
    pub id: i64,
    pub name: String,
    pub folder_ids: Vec<i64>,
    pub tag_ids: Vec<i64>,
}
