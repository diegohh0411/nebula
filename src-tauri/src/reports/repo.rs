//! Reports persistence: folder coverage aggregation, saved report CRUD.
use crate::reports::models::{CoverageReport, CoverageSummary, SavedReport, SubjectCoverage};
use anyhow::Result;
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};

pub async fn get_folder_coverage(
    pool: &SqlitePool,
    folder_id: i64,
    tag_ids: &[i64],
) -> Result<CoverageReport> {
    // 1. Get all target subjects from the selected tags
    let mut targets = HashMap::new();

    if !tag_ids.is_empty() {
        let q = format!(
            "SELECT DISTINCT s.id, s.name
             FROM subjects s
             JOIN subject_tags st ON st.subject_id = s.id
             WHERE st.tag_id IN ({})",
            tag_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",")
        );

        let mut query = sqlx::query(&q);
        for id in tag_ids {
            query = query.bind(id);
        }

        let rows = query.fetch_all(pool).await?;
        for row in rows {
            let id: i64 = row.get("id");
            let name: Option<String> = row.get("name");
            targets.insert(id, name.unwrap_or_else(|| "Unknown".to_string()));
        }
    }

    // 2. Get frequency (distinct photos, not raw face detections) of all
    // subjects in the given folder.
    let folder_subjects = sqlx::query(
        "SELECT f.subject_id, s.name, COUNT(DISTINCT i.id) as frequency
         FROM faces f
         JOIN images i ON i.id = f.image_id
         JOIN subjects s ON s.id = f.subject_id
         WHERE i.folder_id = ? AND f.subject_id IS NOT NULL
         GROUP BY f.subject_id",
    )
    .bind(folder_id)
    .fetch_all(pool)
    .await?;

    let mut present_targets = Vec::new();
    let mut others_found = Vec::new();

    // To track which targets we've seen
    let mut seen_targets = HashSet::new();

    for row in folder_subjects {
        let subj_id: i64 = row.get("subject_id");
        let name: Option<String> = row.get("name");
        let name = name.unwrap_or_else(|| "Unknown".to_string());
        let freq: i64 = row.get("frequency");

        let coverage = SubjectCoverage {
            subject_id: subj_id,
            name,
            frequency: freq as usize,
        };

        if targets.contains_key(&subj_id) {
            present_targets.push(coverage);
            seen_targets.insert(subj_id);
        } else {
            others_found.push(coverage);
        }
    }

    // 3. Find missing targets
    let mut missing_targets = Vec::new();
    for (id, name) in targets.iter() {
        if !seen_targets.contains(id) {
            missing_targets.push(SubjectCoverage {
                subject_id: *id,
                name: name.clone(),
                frequency: 0,
            });
        }
    }

    // Sort lists by name
    missing_targets.sort_by(|a, b| a.name.cmp(&b.name));
    present_targets.sort_by(|a, b| a.name.cmp(&b.name));
    others_found.sort_by(|a, b| a.name.cmp(&b.name));

    let summary = CoverageSummary {
        total_targets: targets.len(),
        present_targets: present_targets.len(),
    };

    Ok(CoverageReport {
        summary,
        missing_targets,
        present_targets,
        others_found,
    })
}

/// Dedupe while preserving first-occurrence order, so callers that pass
/// duplicate tag ids don't hit the `(report_id, tag_id)` primary key.
fn dedupe_ids(ids: &[i64]) -> Vec<i64> {
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(**id))
        .copied()
        .collect()
}

pub async fn create_saved_report(
    pool: &SqlitePool,
    name: &str,
    folder_id: i64,
    tag_ids: &[i64],
) -> Result<SavedReport> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("Report name cannot be empty");
    }
    let tag_ids = dedupe_ids(tag_ids);

    let now = chrono::Utc::now().timestamp();
    let mut tx = pool.begin().await?;

    let report_id =
        sqlx::query("INSERT INTO saved_reports (name, folder_id, added_at) VALUES (?, ?, ?)")
            .bind(name)
            .bind(folder_id)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();

    for tag_id in &tag_ids {
        sqlx::query("INSERT INTO saved_report_tags (report_id, tag_id) VALUES (?, ?)")
            .bind(report_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    Ok(SavedReport {
        id: report_id,
        name: name.to_string(),
        folder_id,
        tag_ids,
    })
}

async fn tags_by_report_id(
    pool: &SqlitePool,
    report_ids: &[i64],
) -> Result<HashMap<i64, Vec<i64>>> {
    let mut tags: HashMap<i64, Vec<i64>> = HashMap::new();
    if report_ids.is_empty() {
        return Ok(tags);
    }

    let q = format!(
        "SELECT report_id, tag_id FROM saved_report_tags WHERE report_id IN ({}) ORDER BY report_id, rowid",
        report_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",")
    );
    let mut query = sqlx::query(&q);
    for id in report_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    for row in rows {
        let report_id: i64 = row.get("report_id");
        let tag_id: i64 = row.get("tag_id");
        tags.entry(report_id).or_default().push(tag_id);
    }
    Ok(tags)
}

pub async fn list_saved_reports(pool: &SqlitePool) -> Result<Vec<SavedReport>> {
    let rows = sqlx::query("SELECT id, name, folder_id FROM saved_reports ORDER BY added_at DESC")
        .fetch_all(pool)
        .await?;

    let ids: Vec<i64> = rows.iter().map(|r| r.get("id")).collect();
    let mut tags = tags_by_report_id(pool, &ids).await?;

    let mut reports = Vec::new();
    for row in rows {
        let id: i64 = row.get("id");
        reports.push(SavedReport {
            id,
            name: row.get("name"),
            folder_id: row.get("folder_id"),
            tag_ids: tags.remove(&id).unwrap_or_default(),
        });
    }

    Ok(reports)
}

pub async fn get_saved_report(pool: &SqlitePool, report_id: i64) -> Result<Option<SavedReport>> {
    let row = sqlx::query("SELECT id, name, folder_id FROM saved_reports WHERE id = ?")
        .bind(report_id)
        .fetch_optional(pool)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let mut tags = tags_by_report_id(pool, &[report_id]).await?;

    Ok(Some(SavedReport {
        id: row.get("id"),
        name: row.get("name"),
        folder_id: row.get("folder_id"),
        tag_ids: tags.remove(&report_id).unwrap_or_default(),
    }))
}

pub async fn delete_saved_report(pool: &SqlitePool, report_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM saved_reports WHERE id = ?")
        .bind(report_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_saved_report_name(
    pool: &SqlitePool,
    report_id: i64,
    new_name: &str,
) -> Result<()> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        anyhow::bail!("Report name cannot be empty");
    }
    sqlx::query("UPDATE saved_reports SET name = ? WHERE id = ?")
        .bind(new_name)
        .bind(report_id)
        .execute(pool)
        .await?;
    Ok(())
}
