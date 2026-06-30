use serde::{Deserialize, Serialize};
pub use crate::models::{Face, Subject};

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
