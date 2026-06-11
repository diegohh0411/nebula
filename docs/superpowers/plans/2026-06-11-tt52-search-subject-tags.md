# TT-52 Subject Tags + Deterministic Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deterministic subject lookup in the nav-bar search (typeahead + subjects row), subject-level metadata tags (e.g. `cabaña-21`), tag-aware search with group-photos-first ordering, and a `/tags` management view.

**Architecture:** Tauri app — Rust backend (`src-tauri/`, SQLite via sqlx) + Angular 19 frontend (`src/`, standalone components, signals, OnPush). New `tags` + `subject_tags` tables added to the base schema (NO migration — user resets APP_DATA). All matching is accent/case-insensitive via a Rust `normalize()` helper; subject names are matched in Rust at query time, tags via a `name_normalized` column.

**Tech Stack:** Rust (sqlx, tokio, anyhow), Angular 19 + signals, vitest, Tailwind.

**Spec:** `docs/superpowers/specs/2026-06-11-tt52-search-subject-tags-design.md` — read it first.

**Test commands:**
- Rust: `cd /home/pi/nebula/src-tauri && cargo test` (single test: `cargo test test_name`)
- Frontend: `cd /home/pi/nebula && pnpm test`

**Hard rules:**
- Never push to `main`. Work on branch `tt-52-subject-tags`, commit after every task.
- Do not touch the vector search math in `src-tauri/src/search.rs`.
- The existing `search` Tauri command already pins subject-name matches at score 1.0 (`commands.rs:94-113`). You will UPGRADE that block, not add a parallel one.

---

### Task 0: Branch

- [ ] **Step 1:** `cd /home/pi/nebula && git checkout -b tt-52-subject-tags`

---

### Task 1: Schema — `tags` and `subject_tags` tables

**Files:**
- Modify: `src-tauri/src/db.rs` (the big `CREATE TABLE` schema string near the top, after the `subjects`/`faces` block around line 74-98)

- [ ] **Step 1:** In the schema string in `db.rs`, immediately after the `CREATE INDEX IF NOT EXISTS idx_faces_subject ON faces(subject_id);` line, add:

```sql
CREATE TABLE IF NOT EXISTS tags (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    name_normalized TEXT NOT NULL UNIQUE,
    added_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS subject_tags (
    subject_id INTEGER NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (subject_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_subject_tags_tag ON subject_tags(tag_id);
```

- [ ] **Step 2:** Run `cd src-tauri && cargo test` — everything must still pass (existing tests create the schema through the same init path; if a test sets up its own minimal schema and now fails on a missing table, add the same two CREATE TABLE statements to that test fixture).
- [ ] **Step 3:** Commit: `git add -A && git commit -m "feat(TT-52): add tags and subject_tags tables to base schema"`

---

### Task 2: `normalize()` helper

**Files:**
- Modify: `src-tauri/src/db.rs` (add the function near the top, after the imports; add tests inside the existing `mod tests` at the bottom)

- [ ] **Step 1: Write the failing tests.** Inside `mod tests` in `db.rs`:

```rust
#[test]
fn test_normalize_strips_accents_and_case() {
    assert_eq!(normalize("Cabaña-21"), "cabana-21");
    assert_eq!(normalize("JOSÉ"), "jose");
    assert_eq!(normalize("  Über  "), "uber");
    assert_eq!(normalize("plain"), "plain");
    assert_eq!(normalize(""), "");
}
```

- [ ] **Step 2:** Run `cargo test test_normalize_strips_accents_and_case` — expect FAIL (function not found).
- [ ] **Step 3: Implement.** No new crate needed — decompose the common Latin-1 accents manually to keep dependencies flat. Add to `db.rs` (public, used by commands too):

```rust
/// Lowercase + strip diacritics so "Cabaña" matches "cabana".
pub fn normalize(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}
```

- [ ] **Step 4:** `cargo test test_normalize_strips_accents_and_case` — expect PASS.
- [ ] **Step 5:** Commit: `git add -A && git commit -m "feat(TT-52): accent/case normalization helper"`

---

### Task 3: Rust models — `Tag`, `TagWithCount`, `SubjectMatch`

**Files:**
- Modify: `src-tauri/src/models/entities.rs` (same file that defines `Subject`; check with `grep -n "pub struct Subject" src-tauri/src/models/*.rs` and put these next to it)

- [ ] **Step 1:** Add (match the serde style of the surrounding structs — they derive `Serialize`/`Deserialize` and use `rename` for the `type` field):

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub added_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TagWithCount {
    pub id: i64,
    pub name: String,
    pub added_at: i64,
    pub subject_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubjectMatch {
    pub subject: Subject,
    pub tags: Vec<Tag>,
}
```

Export them the same way `Subject` is exported (check `src-tauri/src/models/mod.rs` for the `pub use` line and extend it).

- [ ] **Step 2:** `cargo build` — expect success. Commit: `git commit -am "feat(TT-52): Tag/TagWithCount/SubjectMatch models"`

---

### Task 4: DB layer — tag CRUD

**Files:**
- Modify: `src-tauri/src/db.rs` (functions near `search_subjects_by_name` around line 895; tests in `mod tests`)

- [ ] **Step 1: Write failing tests** in `mod tests`. Look at an existing async db test in that module first and copy its pool-setup helper (there is one that creates an in-memory pool and runs the schema — reuse it exactly):

```rust
#[tokio::test]
async fn test_tag_crud() {
    let pool = setup_test_db().await; // use the module's actual helper name

    // find-or-create dedups on normalized name, keeps first display name
    let t1 = add_subject_tag(&pool, 1, "Cabaña-21").await.unwrap();
    let t2 = add_subject_tag(&pool, 2, "cabana-21").await.unwrap();
    assert_eq!(t1.id, t2.id);
    assert_eq!(t2.name, "Cabaña-21");

    // adding same tag to same subject twice is idempotent
    add_subject_tag(&pool, 1, "cabaña-21").await.unwrap();
    let tags = get_subject_tags(&pool, 1).await.unwrap();
    assert_eq!(tags.len(), 1);

    // list with counts
    let all = list_tags_with_counts(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].subject_count, 2);

    // empty name rejected
    assert!(add_subject_tag(&pool, 1, "   ").await.is_err());

    // rename + collision
    let other = add_subject_tag(&pool, 1, "cabin-3").await.unwrap();
    assert!(rename_tag(&pool, other.id, "CABAÑA-21").await.is_err()); // collides
    rename_tag(&pool, other.id, "cabin-4").await.unwrap();

    // remove junction, then delete tag entirely
    remove_subject_tag(&pool, 2, t1.id).await.unwrap();
    assert_eq!(list_tags_with_counts(&pool).await.unwrap().iter().find(|t| t.id == t1.id).unwrap().subject_count, 1);
    delete_tag(&pool, t1.id).await.unwrap();
    assert!(get_subject_tags(&pool, 1).await.unwrap().iter().all(|t| t.id != t1.id));
}
```

Note: if the test helper schema requires real subject rows for FK enforcement, insert two subjects first the same way neighboring tests do (`INSERT INTO subjects (name, type, added_at) VALUES ('Alice', 'person', 0) RETURNING id` — see `db.rs:1584` for the pattern) and use those ids instead of `1`/`2`.

- [ ] **Step 2:** `cargo test test_tag_crud` — expect FAIL (functions not found).
- [ ] **Step 3: Implement** in `db.rs` (imports: these functions follow the same `pub async fn …(pool: &SqlitePool, …) -> Result<…>` style as the rest of the file; `Tag`/`TagWithCount` come from `crate::models`):

```rust
pub async fn add_subject_tag(pool: &SqlitePool, subject_id: i64, name: &str) -> Result<Tag> {
    let tag = create_tag(pool, name).await?; // find-or-create, validates non-empty
    let now = chrono::Utc::now().timestamp(); // use the same "now" idiom as the rest of db.rs
    sqlx::query("INSERT OR IGNORE INTO subject_tags (subject_id, tag_id, added_at) VALUES (?, ?, ?)")
        .bind(subject_id).bind(tag.id).bind(now)
        .execute(pool).await?;
    Ok(tag)
}

/// Standalone find-or-create (used by the /tags view's inline create).
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
```

Check how the rest of `db.rs` obtains timestamps (grep `added_at`) and copy that idiom instead of `chrono` if it differs.

- [ ] **Step 4:** `cargo test test_tag_crud` — expect PASS. Run full `cargo test` too.
- [ ] **Step 5:** Commit: `git commit -am "feat(TT-52): tag CRUD db layer"`

---

### Task 5: DB layer — subject/tag matching for typeahead

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Failing test:**

```rust
#[tokio::test]
async fn test_search_subjects_matching() {
    let pool = setup_test_db().await;
    // create subjects José and Maria (see db.rs:1584 pattern for inserts)
    // tag Maria with "Cabaña-21"

    // accent-insensitive name match
    let hits = search_subjects_matching(&pool, "jose").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].subject.name.as_deref(), Some("José"));

    // tag match returns the tagged subject, with tags populated
    let hits = search_subjects_matching(&pool, "cabana").await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].subject.name.as_deref(), Some("Maria"));
    assert_eq!(hits[0].tags[0].name, "Cabaña-21");

    // a query matching BOTH name and tag dedups by subject id
    let hits = search_subjects_matching(&pool, "maria").await.unwrap();
    assert_eq!(hits.len(), 1);

    // no match → empty
    assert!(search_subjects_matching(&pool, "zzz").await.unwrap().is_empty());
}
```

- [ ] **Step 2:** `cargo test test_search_subjects_matching` — expect FAIL.
- [ ] **Step 3: Implement.** Names are matched in Rust (not SQL) so `normalize()` applies; subject counts are small so a full scan is fine:

```rust
pub async fn search_subjects_matching(pool: &SqlitePool, query: &str) -> Result<Vec<SubjectMatch>> {
    let q = normalize(query);
    if q.is_empty() {
        return Ok(vec![]);
    }
    let mut matched: Vec<Subject> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 1. name matches — fetch all named subjects, filter in Rust
    let rows = sqlx::query("SELECT id, name, thumbnail_face_id, type, added_at FROM subjects WHERE name IS NOT NULL")
        .fetch_all(pool).await?;
    for r in rows {
        let name: String = r.get("name");
        if normalize(&name).contains(&q) {
            let s = Subject {
                id: r.get("id"), name: Some(name),
                thumbnail_face_id: r.get("thumbnail_face_id"),
                subject_type: r.get("type"), added_at: r.get("added_at"),
            };
            if seen.insert(s.id) { matched.push(s); }
        }
    }

    // 2. tag matches — tags.name_normalized is already normalized
    let like = format!("%{}%", q);
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.thumbnail_face_id, s.type, s.added_at
         FROM subjects s
         JOIN subject_tags st ON st.subject_id = s.id
         JOIN tags t ON t.id = st.tag_id
         WHERE t.name_normalized LIKE ?")
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
```

Verify the exact `Subject` field names against the struct (`subject_type` is serialized as `type` — copy how `search_subjects_by_name` at `db.rs:895` builds it).

- [ ] **Step 4:** `cargo test test_search_subjects_matching` — PASS. Commit: `git commit -am "feat(TT-52): subject/tag matching for typeahead"`

---

### Task 6: DB layer — tag-matched images ordered by tagged-subject count

**Files:**
- Modify: `src-tauri/src/db.rs`

- [ ] **Step 1: Failing test.** Scenario: tag two subjects `cabin-9`; image A has faces of both, image B has a face of one, image C has no tagged faces, image D has both but is soft-deleted:

```rust
#[tokio::test]
async fn test_tag_image_ids_ordered_by_subject_count() {
    let pool = setup_test_db().await;
    // Insert: folder, 4 images (one with deleted_at set), 2 subjects, faces linking
    // them (copy insert patterns from neighboring tests in this module — images
    // need folder_id/path/file_hash/mtime/added_at/updated_at at minimum).
    // Tag both subjects "cabin-9" via add_subject_tag.

    let ids = get_tag_image_ids_ordered(&pool, "cabin 9").await.unwrap();
    assert_eq!(ids, vec![image_a_id, image_b_id]); // A first (2 tagged subjects), B second (1), C absent, D (deleted) absent
}
```

Note the query "cabin 9" with a space — normalization does NOT turn spaces into hyphens, so match on LIKE with the space replaced: see implementation note below.

- [ ] **Step 2:** `cargo test test_tag_image_ids_ordered_by_subject_count` — expect FAIL.
- [ ] **Step 3: Implement:**

```rust
/// Images containing faces of subjects whose tag matches `query`,
/// ordered by how many distinct tagged subjects appear in each image (desc),
/// then date_taken desc. Soft-deleted images excluded.
pub async fn get_tag_image_ids_ordered(pool: &SqlitePool, query: &str) -> Result<Vec<i64>> {
    let q = normalize(query).replace(' ', "%"); // "cabin 9" matches "cabin-9"
    if q.is_empty() {
        return Ok(vec![]);
    }
    let like = format!("%{}%", q);
    let rows = sqlx::query(
        "SELECT f.image_id
         FROM faces f
         JOIN subject_tags st ON st.subject_id = f.subject_id
         JOIN tags t ON t.id = st.tag_id
         JOIN images i ON i.id = f.image_id
         WHERE t.name_normalized LIKE ? AND i.deleted_at IS NULL
         GROUP BY f.image_id
         ORDER BY COUNT(DISTINCT f.subject_id) DESC, MAX(i.date_taken) DESC")
        .bind(&like).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| r.get("image_id")).collect())
}
```

- [ ] **Step 4:** Test PASS, full `cargo test` PASS. Commit: `git commit -am "feat(TT-52): tag-matched images ordered by tagged-subject count"`

---

### Task 7: Tauri commands + registration

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs:116-142` (the `generate_handler!` list)

- [ ] **Step 1:** Add to `commands.rs` (follow the existing `map_err` style; add `Tag, TagWithCount, SubjectMatch` to the `crate::models` import at the top):

```rust
#[tauri::command]
pub async fn search_subjects(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SubjectMatch>, String> {
    db::search_subjects_matching(&state.pool, &query).await.map_err(map_err)
}

#[tauri::command]
pub async fn add_subject_tag(
    subject_id: i64,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Tag, String> {
    db::add_subject_tag(&state.pool, subject_id, &name).await.map_err(map_err)
}

#[tauri::command]
pub async fn create_tag(
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<Tag, String> {
    db::create_tag(&state.pool, &name).await.map_err(map_err)
}

#[tauri::command]
pub async fn remove_subject_tag(
    subject_id: i64,
    tag_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::remove_subject_tag(&state.pool, subject_id, tag_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn get_subject_tags(
    subject_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Tag>, String> {
    db::get_subject_tags(&state.pool, subject_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn list_tags(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TagWithCount>, String> {
    db::list_tags_with_counts(&state.pool).await.map_err(map_err)
}

#[tauri::command]
pub async fn rename_tag(
    tag_id: i64,
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::rename_tag(&state.pool, tag_id, &name).await.map_err(map_err)
}

#[tauri::command]
pub async fn delete_tag(
    tag_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    db::delete_tag(&state.pool, tag_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn get_tag_subjects(
    tag_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SubjectMatch>, String> {
    let pool = &state.pool;
    let rows = db::get_subjects_for_tag(pool, tag_id).await.map_err(map_err)?;
    let mut out = Vec::with_capacity(rows.len());
    for s in rows {
        let tags = db::get_subject_tags(pool, s.id).await.map_err(map_err)?;
        out.push(SubjectMatch { subject: s, tags });
    }
    Ok(out)
}
```

This requires one more small db function:

```rust
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
```

- [ ] **Step 2:** Register all 9 new commands in `lib.rs` inside `generate_handler![…]`, after `commands::unassign_face`:

```rust
commands::search_subjects,
commands::create_tag,
commands::add_subject_tag,
commands::remove_subject_tag,
commands::get_subject_tags,
commands::list_tags,
commands::rename_tag,
commands::delete_tag,
commands::get_tag_subjects,
```

- [ ] **Step 3:** `cargo build` succeeds, `cargo test` passes. Commit: `git commit -am "feat(TT-52): tag + subject-search tauri commands"`

---

### Task 8: Upgrade the `search` command's Text arm

**Files:**
- Modify: `src-tauri/src/commands.rs:93-149` (the `SearchQuery::Text` arm)

Current behavior (lines 94-113): collects images of name-matched subjects into a `HashSet` (unordered!) and pins them at score 1.0. Required behavior: pinned set = name-matched-subject images **plus** tag-matched images, with tag-matched ordering (group photos first) preserved, then name-derived images, then semantic results deduplicated.

- [ ] **Step 1:** Replace lines 94-113 with:

```rust
// 1. Tag-derived images, already ordered by tagged-subject count desc.
let tag_image_ids = db::get_tag_image_ids_ordered(pool, query).await.unwrap_or_default();

// 2. Name-derived images (accent-insensitive), appended after tag matches.
let matched = db::search_subjects_matching(pool, query).await.unwrap_or_default();
let subject_ids: Vec<i64> = matched.iter().map(|m| m.subject.id).collect();
let name_image_ids = db::get_image_ids_for_subjects(pool, &subject_ids).await.unwrap_or_default();

let mut pinned_ids: Vec<i64> = Vec::new();
let mut pinned_set: HashSet<i64> = HashSet::new();
for id in tag_image_ids.into_iter().chain(name_image_ids.into_iter()) {
    if pinned_set.insert(id) {
        pinned_ids.push(id);
    }
}

let mut results = vec![];
for image_id in &pinned_ids {
    if let Ok(Some(img)) = db::get_image_by_id(pool, *image_id).await {
        if img.deleted_at.is_some() {
            continue;
        }
        results.push(SearchResult {
            image_id: *image_id,
            path: img.path,
            thumbnail_path: img.thumbnail_path,
            preview_path: img.preview_path,
            score: 1.0,
            date_taken: img.date_taken,
            mtime: img.mtime,
            semantic_analysis_done: img.semantic_analysis_done,
            subject_analysis_done: img.subject_analysis_done,
        });
    }
}
```

- [ ] **Step 2:** Further down (current line ~141), the dedup check `!subject_image_ids.contains(&res.image_id)` must become `!pinned_set.contains(&res.image_id)`.
- [ ] **Step 3:** The old `db::search_subjects_by_name` may now be unused — if `cargo build` warns it's dead code and nothing else calls it, delete it and its tests.
- [ ] **Step 4:** Keep the rest of the arm (embedding cache, semantic search, `delete_stale_cache_entries`) untouched. `cargo test` + `cargo build` pass. Commit: `git commit -am "feat(TT-52): tag-aware pinned ordering in text search"`

---

### Task 9: Frontend models + PhotoService wrappers

**Files:**
- Modify: `src/app/models/models.ts`
- Modify: `src/app/services/photo.service.ts`
- Test: `src/app/services/photo.service.spec.ts`

- [ ] **Step 1:** Add to `models.ts` next to `Subject`:

```typescript
export interface Tag {
  id: number;
  name: string;
  added_at: number;
}

export interface TagWithCount extends Tag {
  subject_count: number;
}

export interface SubjectMatch {
  subject: Subject;
  tags: Tag[];
}
```

- [ ] **Step 2:** In `photo.service.ts` add a signal and wrappers (mirror the style of `getSubjectPhotos` etc.):

```typescript
readonly subjectMatches = signal<SubjectMatch[]>([]);

async searchSubjects(query: string): Promise<SubjectMatch[]> {
  return await invoke<SubjectMatch[]>('search_subjects', { query });
}

async createTag(name: string): Promise<Tag> {
  return await invoke<Tag>('create_tag', { name });
}

async addSubjectTag(subjectId: number, name: string): Promise<Tag> {
  return await invoke<Tag>('add_subject_tag', { subjectId, name });
}

async removeSubjectTag(subjectId: number, tagId: number): Promise<void> {
  await invoke('remove_subject_tag', { subjectId, tagId });
}

async getSubjectTags(subjectId: number): Promise<Tag[]> {
  return await invoke<Tag[]>('get_subject_tags', { subjectId });
}

async listTags(): Promise<TagWithCount[]> {
  return await invoke<TagWithCount[]>('list_tags', {});
}

async renameTag(tagId: number, name: string): Promise<void> {
  await invoke('rename_tag', { tagId, name });
}

async deleteTag(tagId: number): Promise<void> {
  await invoke('delete_tag', { tagId });
}

async getTagSubjects(tagId: number): Promise<SubjectMatch[]> {
  return await invoke<SubjectMatch[]>('get_tag_subjects', { tagId });
}
```

- [ ] **Step 3:** In `searchByText(query)` (line ~242), alongside the existing search invoke, populate the subjects row (failures must NOT break image search):

```typescript
this.searchSubjects(query)
  .then((m) => this.subjectMatches.set(m))
  .catch(() => this.subjectMatches.set([]));
```

In `clearSearch()` (line ~258-261 region) add `this.subjectMatches.set([]);`.

- [ ] **Step 4:** Add a vitest spec in `photo.service.spec.ts` following the file's existing mock pattern for `invoke`: assert that `searchByText` sets `subjectMatches` from the mocked `search_subjects` response, and that `clearSearch` empties it.
- [ ] **Step 5:** `pnpm test` — PASS. Commit: `git commit -am "feat(TT-52): frontend tag models + PhotoService wrappers"`

---

### Task 10: Search bar typeahead

**Files:**
- Modify: `src/app/components/search-bar/search-bar.component.ts`, `.html`, `.css`

Behavior: typing ≥2 chars triggers a 200 ms-debounced `photos.searchSubjects()`; results render in a dropdown anchored under the input. Each row: subject thumbnail (reuse whatever the People view uses to render `thumbnail_face_id` — check `people-view` component and copy its image-src approach), name, small tag chips. Click → `router.navigate(['/subject', match.subject.id])` and close. Enter → existing `onSearch()` and close. Escape or blur → close.

- [ ] **Step 1:** Component logic (add to `search-bar.component.ts`):

```typescript
private router = inject(Router);
protected typeaheadMatches = signal<SubjectMatch[]>([]);
protected typeaheadOpen = signal(false);
private typeaheadTimer: ReturnType<typeof setTimeout> | null = null;
private typeaheadSeq = 0;

protected onQueryInput(): void {
  const q = this.query().trim();
  if (this.typeaheadTimer !== null) clearTimeout(this.typeaheadTimer);
  if (q.length < 2) {
    this.typeaheadOpen.set(false);
    this.typeaheadMatches.set([]);
    return;
  }
  this.typeaheadTimer = setTimeout(() => {
    const seq = ++this.typeaheadSeq;
    this.photos.searchSubjects(q).then((m) => {
      if (seq !== this.typeaheadSeq) return; // stale response
      this.typeaheadMatches.set(m);
      this.typeaheadOpen.set(m.length > 0);
    }).catch(() => { /* typeahead failures are silent */ });
  }, 200);
}

protected onTypeaheadSelect(match: SubjectMatch): void {
  this.typeaheadOpen.set(false);
  void this.router.navigate(['/subject', match.subject.id]);
}
```

Wire `(input)="onQueryInput()"` on the existing input element, close the dropdown inside `onSearch()` / `onClear()` / on `keydown.escape`, and clear `typeaheadTimer` in the existing `ngOnDestroy`.

- [ ] **Step 2:** Template: absolutely-positioned panel below the input (`position: relative` wrapper). Check `src/app/libs/ui/command` — if `hlm-command` fits naturally use it; if it fights the existing markup, a plain styled `<ul>` matching the app's Tailwind styling is acceptable. Keep it simple.
- [ ] **Step 3:** Vitest spec: with `invoke` mocked, simulate input of "jo", advance fake timers 200 ms, assert dropdown renders the mocked match; simulate a 1-char query, assert dropdown closed.
- [ ] **Step 4:** `pnpm test` PASS. Commit: `git commit -am "feat(TT-52): subject typeahead in search bar"`

---

### Task 11: Subjects row above the gallery grid

**Files:**
- Modify: `src/app/components/gallery/gallery.component.ts`, `.html`

- [ ] **Step 1:** The gallery already reads `photos.searchResults()`. Add a horizontal, scrollable row that renders when `photos.subjectMatches().length > 0` AND a search is active (`photos.searchResults() !== null`), above the photo grid. Each card: subject thumbnail + name + tag chips; click navigates to `/subject/:id`. Reuse the card/thumbnail markup style from `people-view`.
- [ ] **Step 2:** Vitest spec: with `subjectMatches` set on a mocked service, assert the row renders cards; with empty matches, assert it's absent.
- [ ] **Step 3:** `pnpm test` PASS. Commit: `git commit -am "feat(TT-52): subjects row above search results"`

---

### Task 12: Tag chips on subject detail

**Files:**
- Modify: `src/app/components/subject-detail/subject-detail.component.ts`, `.html`

- [ ] **Step 1:** On load (where the component already fetches `getSubjectDetail`), also fetch `getSubjectTags(subjectId)` into a `tags = signal<Tag[]>([])`.
- [ ] **Step 2:** UI under the subject name: chips with an `×` button calling `removeSubjectTag` then refreshing the signal; an "Add tag" input that (a) autocompletes from `listTags()` (fetch once on focus, filter client-side, simple datalist or dropdown), (b) on Enter calls `addSubjectTag(subjectId, value)` and refreshes. Empty input does nothing. Backend errors surface via the component's existing error pattern (check how `name_subject` errors are shown and copy it).
- [ ] **Step 3:** Vitest spec: mocked service — renders chips from `getSubjectTags`, remove calls `removeSubjectTag`, Enter on input calls `addSubjectTag`.
- [ ] **Step 4:** `pnpm test` PASS. Commit: `git commit -am "feat(TT-52): tag editing on subject detail"`

---

### Task 13: `/tags` management route

**Files:**
- Create: `src/app/components/tags-view/tags-view.component.ts`, `.html`, `.css`
- Modify: `src/app/app.routes.ts` (add `{ path: "tags", component: TagsViewComponent },` after the `people` route)
- Modify: `src/app/components/sidebar/` (add a nav entry "Tags" next to People — copy the People link markup exactly)

- [ ] **Step 1:** Component structure (standalone, OnPush, signals — copy the People view component skeleton):
  - `tags = signal<TagWithCount[]>([])` loaded from `listTags()` on init.
  - `selectedTag = signal<TagWithCount | null>(null)`; selecting loads `getTagSubjects(tag.id)` into `tagSubjects = signal<SubjectMatch[]>([])`.
  - Master/detail layout: tag list (name + subject count) on the left; selected tag's subjects as cards (People-view card style) on the right, each with a "Remove from tag" button calling `removeSubjectTag` then reloading both lists.
  - Create: an inline "New tag" input + button at the top of the tag list calling `createTag(name)` then reloading the list. Empty input does nothing; backend errors (e.g. empty after trim) show inline.
  - Rename: reuse the existing `EditableText` component (`src/app/components/editable-text/`) on the tag name, calling `renameTag`; on backend error (name collision) show the error and keep the old name.
  - Delete: button with a `confirm()` dialog ("Delete tag X? Subjects are not affected."), calls `deleteTag`, reloads, clears selection.
- [ ] **Step 2:** Vitest spec: mocked service — list renders with counts; selecting a tag renders its subjects; delete calls `deleteTag`.
- [ ] **Step 3:** `pnpm test` PASS. Commit: `git commit -am "feat(TT-52): /tags management view"`

---

### Task 14: Final verification

- [ ] **Step 1:** `cd src-tauri && cargo test && cargo clippy -- -D warnings 2>/dev/null || cargo test` — all Rust tests pass.
- [ ] **Step 2:** `cd /home/pi/nebula && pnpm test` — all frontend tests pass.
- [ ] **Step 3:** `pnpm exec tsc --noEmit -p tsconfig.app.json` (or the project's lint/build script if one exists in `package.json`) — no type errors.
- [ ] **Step 4:** Push branch and open a PR titled `feat(TT-52): subject tags + deterministic search`. Do NOT merge. Set the Notion task TT-52 (page `37ae954d-b476-8179-bb36-c74fdd040022`) status to **Ready for review** and record the PR number, per the nebula-notion-workflow skill.
