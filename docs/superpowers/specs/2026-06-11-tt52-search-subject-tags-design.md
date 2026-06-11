# TT-52 — Navigation bar search: deterministic subject lookup + subject tags

**Notion task:** TT-52 (`37ae954d-b476-8179-bb36-c74fdd040022`)
**Date:** 2026-06-11
**Status:** Approved design

## Problem

During weekly cabin-by-cabin counselor review sessions, the search bar must be fast and precise. Today search is purely semantic (text → embedding → vector lookup in `src-tauri/src/search.rs`). There is no exact-name matching and no way to label subjects with structured tags like `cabaña-21`.

## Scope decisions (settled during brainstorming)

- **Tags attach to subjects only.** Image-level tags are deferred to a follow-up Notion task ("Image-level metadata tags (follow-up to TT-52)").
- **Deterministic subject matches surface in BOTH places:** a typeahead dropdown under the search input while typing, and a "Subjects" row above the photo grid after submitting a search.
- **Tags are editable from the subject detail page** (chips + autocomplete) **and** from a new full-management `/tags` route.
- **Tag-matched images are pinned above semantic results**, ordered by the number of distinct tagged subjects in each image, descending (cabin group photos first, solo shots last), then date taken descending as tiebreaker. Semantic results follow, deduplicated by `image_id`.
- **No SQL migration needed.** Modify the base `CREATE TABLE` schema in `db.rs` directly; the user will reset their APP_DATA folder.
- **Matching strategy:** normalized substring `LIKE` (lowercase + accent-stripped) — no FTS5, no fuzzy matching. "cabana" must match "cabaña".

## Schema (edit base schema in `src-tauri/src/db.rs`)

```sql
-- subjects table is UNCHANGED. Subject-name matching normalizes in Rust at
-- query time (subjects number in the hundreds; a scan is instant). This avoids
-- maintaining a normalized column across the three subjects.name write paths.

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

Normalization function (Rust, in `db.rs` or a small util module): lowercase + Unicode NFD decomposition + strip combining marks. `normalize("Cabaña-21") == "cabana-21"`. Used for every write to `tags.name_normalized`, applied to user queries before matching, and applied to subject names in Rust when matching.

## Backend (Tauri commands in `src-tauri/src/commands.rs`, queries in `db.rs`)

| Command | Behavior |
|---|---|
| `search_subjects(query: String) -> Vec<SubjectMatch>` | Normalize the query. Return subjects whose `name_normalized LIKE '%q%'` **plus** subjects linked to tags whose `name_normalized LIKE '%q%'`. Each `SubjectMatch` carries: subject id, name, thumbnail face info (same shape the People view uses), and the subject's tags. Dedup by subject id. Cap at 20. |
| `create_tag(name: String) -> Tag` | Standalone find-or-create by normalized name (for the `/tags` view's inline create). |
| `add_subject_tag(subject_id: i64, name: String) -> Tag` | Normalize name; find-or-create the tag by `name_normalized` (preserve the first-seen display name); insert into `subject_tags` (idempotent via `INSERT OR IGNORE`). |
| `remove_subject_tag(subject_id: i64, tag_id: i64)` | Delete the junction row. |
| `list_tags() -> Vec<TagWithCount>` | All tags with `COUNT(subject_id)` from `subject_tags`. |
| `get_subject_tags(subject_id: i64) -> Vec<Tag>` | Tags for one subject. |
| `rename_tag(tag_id: i64, name: String)` | Update name + name_normalized; reject (error) if the new normalized name collides with another tag. |
| `delete_tag(tag_id: i64)` | Delete tag; junction rows cascade. Subjects untouched. |
| `get_tag_subjects(tag_id: i64) -> Vec<SubjectMatch>` | Subjects carrying the tag (for the tag detail view). |

**Note:** the existing `search` command already pins subject-name matches at score 1.0 via `db::search_subjects_by_name` (`LIKE COLLATE NOCASE` — case-insensitive but not accent-insensitive). This work upgrades that path to accent-insensitive matching, adds tag matches, and adds the count-based ordering.

### Tag-aware text search

Extend the existing text-search flow (the command that calls `search_images` in `search.rs`):

1. Normalize the query; find tags with `name_normalized LIKE '%q%'`.
2. If any tags match: collect images via `faces JOIN subject_tags` where the face's subject carries a matching tag, excluding soft-deleted images. Order by `COUNT(DISTINCT subject_id) DESC, date_taken DESC` per image.
3. Run the existing semantic vector search unchanged (elbow heuristic applies only to the semantic list).
4. Final result list = pinned tag matches (given `score = 1.0` so the UI sorts them first), then semantic results with their real scores, deduplicated by `image_id` (pinned wins).
5. The frontend `SearchResult` shape is unchanged; pinned results are just rows with score 1.0 at the front.

The "Subjects" row data comes from a separate `search_subjects` call made by the frontend alongside the search — no change to `SearchResult` needed.

## Frontend (Angular, standalone components, OnPush, signals)

1. **Search bar typeahead** (`search-bar.component`): debounced (~200 ms) call to `search_subjects` while typing ≥2 chars; dropdown built with the existing `hlm-command` UI lib (`src/app/libs/ui/command`). Each row: subject thumbnail, name, tag chips. Click → navigate to subject detail and close. Enter still submits the full search. Escape closes the dropdown.
2. **Subjects row in gallery** (`gallery.component`): when a text search is active, show matching subject cards (from `search_subjects`) in a horizontal row above the photo grid. Hide when empty. Click → subject detail.
3. **Subject detail tags** (`subject-detail.component`): tag chips with an × to remove; an add field with autocomplete against `list_tags()`; creating an unknown tag is allowed (find-or-create handles it).
4. **`/tags` route** (new `tags` component + route): list of all tags with subject counts; inline create, rename (reuse `EditableText`), delete (confirm dialog). Selecting a tag shows its subjects as cards (same card style as People view) with a remove-from-tag action. Add a nav entry next to People.
5. **PhotoService**: add wrappers for the new commands; keep search flow API unchanged besides triggering the parallel `search_subjects` call and exposing its result as a signal.

## Error handling

- `rename_tag` collision → backend error string; frontend shows it inline and keeps the editor open.
- Empty/whitespace tag names rejected in the backend.
- Typeahead failures are silent (log only); search proper still works.

## Testing

- **Rust:** unit tests for `normalize()` (accents, case, hyphens); tag CRUD incl. find-or-create dedup and rename collision; `search_subjects` matching by name and by tag; pinned-search ordering by distinct-subject count with date tiebreaker; dedup of pinned vs semantic results; soft-deleted images excluded.
- **Frontend (vitest):** typeahead debounce + render; tag chip add/remove on subject detail; tags route list/rename/delete flows.

## Out of scope

- Image-level tags (follow-up task created in Notion).
- Fuzzy/typo-tolerant matching.
- SQL migrations (APP_DATA reset instead).
