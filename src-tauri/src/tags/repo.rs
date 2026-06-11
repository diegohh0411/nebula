use anyhow::Result;
use sqlx::{Row, SqlitePool};

use crate::models::{Subject, Tag, TagWithCount, SubjectMatch};
use crate::db::{normalize, like_pattern, matches_tokens};

pub async fn create_tag(pool: &SqlitePool, name: &str) -> Result<Tag> {
    let display = name.trim();
    let norm = normalize(name);
    if norm.is_empty() {
        anyhow::bail!("Tag name cannot be empty");
    }
    let now = chrono::Utc::now().timestamp();
    sqlx::query("INSERT INTO tags (name, name_normalized, added_at) VALUES (?, ?, ?) ON CONFLICT(name_normalized) DO NOTHING")
        .bind(display).bind(&norm).bind(now)
        .execute(pool).await?;
    let row = sqlx::query("SELECT id, name, added_at FROM tags WHERE name_normalized = ?")
        .bind(&norm).fetch_one(pool).await?;
    Ok(Tag { id: row.get("id"), name: row.get("name"), added_at: row.get("added_at") })
}

pub async fn add_subject_tag(pool: &SqlitePool, subject_id: i64, name: &str) -> Result<Tag> {
    let tag = create_tag(pool, name).await?;
    let now = chrono::Utc::now().timestamp();
    sqlx::query("INSERT OR IGNORE INTO subject_tags (subject_id, tag_id, added_at) VALUES (?, ?, ?)")
        .bind(subject_id).bind(tag.id).bind(now)
        .execute(pool).await?;
    Ok(tag)
}

pub async fn remove_subject_tag(pool: &SqlitePool, subject_id: i64, tag_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM subject_tags WHERE subject_id = ? AND tag_id = ?")
        .bind(subject_id).bind(tag_id).execute(pool).await?;
    Ok(())
}

pub async fn get_subject_tags(pool: &SqlitePool, subject_id: i64) -> Result<Vec<Tag>> {
    let rows = sqlx::query(
        "SELECT t.id, t.name, t.added_at FROM tags t
         JOIN subject_tags st ON st.tag_id = t.id
         WHERE st.subject_id = ? ORDER BY t.name COLLATE NOCASE")
        .bind(subject_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| Tag { id: r.get("id"), name: r.get("name"), added_at: r.get("added_at") }).collect())
}

pub async fn list_tags_with_counts(pool: &SqlitePool) -> Result<Vec<TagWithCount>> {
    let rows = sqlx::query(
        "SELECT t.id, t.name, t.added_at, COUNT(st.subject_id) AS subject_count
         FROM tags t LEFT JOIN subject_tags st ON st.tag_id = t.id
         GROUP BY t.id ORDER BY t.name COLLATE NOCASE")
        .fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| TagWithCount {
        id: r.get("id"), name: r.get("name"), added_at: r.get("added_at"),
        subject_count: r.get("subject_count"),
    }).collect())
}

pub async fn rename_tag(pool: &SqlitePool, tag_id: i64, name: &str) -> Result<()> {
    let display = name.trim();
    let norm = normalize(name);
    if norm.is_empty() {
        anyhow::bail!("Tag name cannot be empty");
    }
    let collision = sqlx::query("SELECT id FROM tags WHERE name_normalized = ? AND id != ?")
        .bind(&norm).bind(tag_id).fetch_optional(pool).await?;
    if collision.is_some() {
        anyhow::bail!("A tag with that name already exists");
    }
    sqlx::query("UPDATE tags SET name = ?, name_normalized = ? WHERE id = ?")
        .bind(display).bind(&norm).bind(tag_id).execute(pool).await?;
    Ok(())
}

pub async fn delete_tag(pool: &SqlitePool, tag_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM tags WHERE id = ?").bind(tag_id).execute(pool).await?;
    Ok(())
}

pub async fn get_tag_image_ids_ordered(pool: &SqlitePool, query: &str) -> Result<Vec<i64>> {
    let like = match like_pattern(query) {
        Some(p) => p,
        None => return Ok(vec![]),
    };
    let rows = sqlx::query(
        "SELECT f.image_id
         FROM faces f
         JOIN subject_tags st ON st.subject_id = f.subject_id
         JOIN tags t ON t.id = st.tag_id
         JOIN images i ON i.id = f.image_id
         WHERE t.name_normalized LIKE ? ESCAPE '\\' AND i.deleted_at IS NULL
         GROUP BY f.image_id
         ORDER BY COUNT(DISTINCT f.subject_id) DESC, MAX(i.date_taken) DESC")
        .bind(&like).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| r.get("image_id")).collect())
}

pub async fn search_subjects_matching(pool: &SqlitePool, query: &str) -> Result<Vec<SubjectMatch>> {
    let q = normalize(query);
    let like = match like_pattern(query) {
        Some(p) => p,
        None => return Ok(vec![]),
    };
    let mut matched: Vec<Subject> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let rows = sqlx::query("SELECT id, name, thumbnail_face_id, type, added_at FROM subjects WHERE name IS NOT NULL")
        .fetch_all(pool).await?;
    for r in rows {
        let name: String = r.get("name");
        if matches_tokens(&normalize(&name), &q) {
            let s = Subject {
                id: r.get("id"), name: Some(name),
                thumbnail_face_id: r.get("thumbnail_face_id"),
                subject_type: r.get("type"), added_at: r.get("added_at"),
            };
            if seen.insert(s.id) { matched.push(s); }
        }
    }

    let rows = sqlx::query(
        "SELECT s.id, s.name, s.thumbnail_face_id, s.type, s.added_at
         FROM subjects s
         JOIN subject_tags st ON st.subject_id = s.id
         JOIN tags t ON t.id = st.tag_id
         WHERE t.name_normalized LIKE ? ESCAPE '\\'")
        .bind(&like).fetch_all(pool).await?;
    for r in rows {
        let s = Subject {
            id: r.get("id"), name: r.get("name"),
            thumbnail_face_id: r.get("thumbnail_face_id"),
            subject_type: r.get("type"), added_at: r.get("added_at"),
        };
        if seen.insert(s.id) { matched.push(s); }
    }

    matched.truncate(20);
    let mut out = Vec::with_capacity(matched.len());
    for s in matched {
        let tags = get_subject_tags(pool, s.id).await?;
        out.push(SubjectMatch { subject: s, tags });
    }
    Ok(out)
}

pub async fn get_subjects_for_tag(pool: &SqlitePool, tag_id: i64) -> Result<Vec<Subject>> {
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.thumbnail_face_id, s.type, s.added_at
         FROM subjects s JOIN subject_tags st ON st.subject_id = s.id
         WHERE st.tag_id = ? ORDER BY s.name COLLATE NOCASE")
        .bind(tag_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| Subject {
        id: r.get("id"), name: r.get("name"),
        thumbnail_face_id: r.get("thumbnail_face_id"),
        subject_type: r.get("type"), added_at: r.get("added_at"),
    }).collect())
}

pub async fn get_image_ids_for_subjects(pool: &SqlitePool, subject_ids: &[i64]) -> Result<Vec<i64>> {
    if subject_ids.is_empty() {
        return Ok(vec![]);
    }
    let params = format!("?{}", ", ?".repeat(subject_ids.len() - 1));
    let query_str = format!(
        "SELECT DISTINCT image_id FROM faces WHERE subject_id IN ({})",
        params
    );
    let mut query = sqlx::query(&query_str);
    for id in subject_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| r.get("image_id")).collect())
}
