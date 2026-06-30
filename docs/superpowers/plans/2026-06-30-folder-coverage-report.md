# Folder Coverage Report Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a feature to calculate subject frequency and coverage in a folder based on a target list of tags (rosters).

**Architecture:** A new Tauri command in the `people` slice fetches target subjects from selected tags, then queries subject frequency across all faces in the specified folder, groups them into missing, present, and others, and returns the result.

**Tech Stack:** Rust, sqlx, Tauri

---

### Task 1: Add Models

**Files:**
- Modify: `src-tauri/src/people/models.rs`

- [ ] **Step 1: Add structs to models**

Modify `src-tauri/src/people/models.rs` to include the coverage report models. Make sure to import `serde::{Serialize, Deserialize}`.

```rust
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
```

- [ ] **Step 2: Check compiler**

Run `cd src-tauri && cargo check` to ensure no syntax errors.

- [ ] **Step 3: Commit**

```bash
cd src-tauri && git add src/people/models.rs
git commit -m "feat(models): add CoverageReport structs"
```

---

### Task 2: Implement Repository Query

**Files:**
- Modify: `src-tauri/src/people/repo.rs`

- [ ] **Step 1: Write the repository function**

Add `get_folder_coverage` at the end of `src-tauri/src/people/repo.rs`.

```rust
use crate::people::models::{CoverageReport, CoverageSummary, SubjectCoverage};
use sqlx::Row;

pub async fn get_folder_coverage(
    pool: &sqlx::SqlitePool,
    folder_id: i64,
    tag_ids: &[i64],
) -> anyhow::Result<CoverageReport> {
    // 1. Get all target subjects from the selected tags
    let mut targets = std::collections::HashMap::new();
    
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

    // 2. Get frequency of all subjects in the given folder
    let folder_subjects = sqlx::query(
        "SELECT f.subject_id, s.name, COUNT(f.id) as frequency 
         FROM faces f
         JOIN images i ON i.id = f.image_id
         JOIN subjects s ON s.id = f.subject_id
         WHERE i.folder_id = ? AND f.subject_id IS NOT NULL
         GROUP BY f.subject_id"
    )
    .bind(folder_id)
    .fetch_all(pool)
    .await?;

    let mut present_targets = Vec::new();
    let mut others_found = Vec::new();
    
    // To track which targets we've seen
    let mut seen_targets = std::collections::HashSet::new();

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
```

- [ ] **Step 2: Check compiler**

Run `cd src-tauri && cargo check`

- [ ] **Step 3: Commit**

```bash
cd src-tauri && git add src/people/repo.rs
git commit -m "feat(repo): implement get_folder_coverage query"
```

---

### Task 3: Test Repository Query

**Files:**
- Modify: `src-tauri/src/db/tests.rs`

- [ ] **Step 1: Write test for get_folder_coverage**

Add this test at the end of `src-tauri/src/db/tests.rs`.

```rust
#[tokio::test]
async fn test_get_folder_coverage() {
    let pool = init_test_pool().await;
    
    // Insert mock data
    sqlx::query("INSERT INTO folders (id, path, added_at) VALUES (1, 'path', 0)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO subjects (id, name, type, added_at) VALUES (1, 'Alice', 'person', 0), (2, 'Bob', 'person', 0), (3, 'Charlie', 'person', 0)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO tags (id, name, added_at) VALUES (1, 'Cabin A', 0)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO subject_tags (subject_id, tag_id, added_at) VALUES (1, 1, 0), (2, 1, 0)")
        .execute(&pool).await.unwrap(); // Alice and Bob in Cabin A
        
    sqlx::query("INSERT INTO images (id, folder_id, path, file_hash, hash_status, file_size, mtime, semantic_analysis_done, subject_analysis_done, added_at, updated_at) VALUES (1, 1, 'p1', 'h1', 'ok', 0, 0, false, false, 0, 0)")
        .execute(&pool).await.unwrap();
        
    sqlx::query("INSERT INTO faces (id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, added_at) VALUES (1, 1, 1, 0,0,1,1,0), (2, 1, 3, 0,0,1,1,0), (3, 1, 1, 0,0,1,1,0)")
        .execute(&pool).await.unwrap(); // Alice has 2 faces, Charlie has 1 face, Bob has 0
        
    let report = crate::people::repo::get_folder_coverage(&pool, 1, &[1]).await.unwrap();
    
    assert_eq!(report.summary.total_targets, 2); // Alice and Bob
    assert_eq!(report.summary.present_targets, 1); // Alice
    
    assert_eq!(report.missing_targets.len(), 1);
    assert_eq!(report.missing_targets[0].name, "Bob");
    assert_eq!(report.missing_targets[0].frequency, 0);
    
    assert_eq!(report.present_targets.len(), 1);
    assert_eq!(report.present_targets[0].name, "Alice");
    assert_eq!(report.present_targets[0].frequency, 2);
    
    assert_eq!(report.others_found.len(), 1);
    assert_eq!(report.others_found[0].name, "Charlie");
    assert_eq!(report.others_found[0].frequency, 1);
}
```

- [ ] **Step 2: Run test**

Run `cd src-tauri && cargo test test_get_folder_coverage`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
cd src-tauri && git add src/db/tests.rs
git commit -m "test: add test for get_folder_coverage"
```

---

### Task 4: Expose Tauri Command

**Files:**
- Modify: `src-tauri/src/people/commands.rs`
- Modify: `src-tauri/src/app/mod.rs`

- [ ] **Step 1: Add command to people/commands.rs**

Add to `src-tauri/src/people/commands.rs`:

```rust
use crate::people::models::CoverageReport;

#[tauri::command]
pub async fn get_folder_coverage(
    folder_id: i64,
    tag_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<CoverageReport, String> {
    repo::get_folder_coverage(&state.pool, folder_id, &tag_ids)
        .await
        .map_err(map_err)
}
```

- [ ] **Step 2: Register command in app/mod.rs**

Modify `src-tauri/src/app/mod.rs` to register the new command in `invoke_handler`:

Locate the `tauri::generate_handler!` block and add `crate::people::commands::get_folder_coverage,`

- [ ] **Step 3: Verify build**

Run `cd src-tauri && cargo check`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/people/commands.rs src/app/mod.rs
git commit -m "feat(tauri): expose get_folder_coverage command"
```