# Face Labeling and Search Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Display face bounding boxes with subject info in the Lightbox, and integrate subject matching directly into the search results.

**Architecture:** We will add a backend command to retrieve faces for a specific image, and update the Lightbox component to overlay these faces on the image and list them in the sidebar. We'll update the `search_images` command to prioritize photos containing subjects that fuzzily match the search query before falling back to RAG embedding searches. We'll also link the People View directly to the Search View.

**Tech Stack:** Angular (TypeScript, HTML, CSS), Rust (Tauri), SQLite.

---

### Task 1: Backend - Add `list_faces_for_image` Command

**Files:**
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Add DB query to `db.rs`**

Add the following function to `src-tauri/src/db.rs`:

```rust
pub async fn list_faces_for_image(pool: &SqlitePool, image_id: i64) -> Result<Vec<Face>> {
    let rows = sqlx::query(
        "SELECT id, image_id, subject_id, bbox_x, bbox_y, bbox_w, bbox_h, embedding, added_at
         FROM faces WHERE image_id = ? ORDER BY added_at DESC",
    )
    .bind(image_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Face {
            id: r.get("id"),
            image_id: r.get("image_id"),
            subject_id: r.get("subject_id"),
            bbox_x: r.get("bbox_x"),
            bbox_y: r.get("bbox_y"),
            bbox_w: r.get("bbox_w"),
            bbox_h: r.get("bbox_h"),
            embedding: r.get("embedding"),
            added_at: r.get("added_at"),
        })
        .collect())
}
```

- [ ] **Step 2: Add Tauri command to `commands.rs`**

Add the following command to `src-tauri/src/commands.rs`:

```rust
#[tauri::command]
pub async fn list_faces_for_image(image_id: i64, state: tauri::State<'_, AppState>) -> Result<Vec<Face>, String> {
    db::list_faces_for_image(&state.pool, image_id).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register command in `lib.rs`**

Add `list_faces_for_image` to the `invoke_handler` in `src-tauri/src/lib.rs`.

```rust
            commands::list_faces_for_image,
```

- [ ] **Step 4: Update Angular PhotoService**

Add the API call to `src/app/services/photo.service.ts`:

```typescript
  async loadFacesForImage(imageId: number): Promise<Face[]> {
    return await invoke<Face[]>('list_faces_for_image', { imageId });
  }
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/commands.rs src-tauri/src/lib.rs src/app/services/photo.service.ts
git commit -m "feat(backend): add list_faces_for_image command"
```

### Task 2: Frontend - Lightbox Faces Overlay and Sidebar

**Files:**
- Modify: `src/app/components/lightbox/lightbox.component.ts`
- Modify: `src/app/components/lightbox/lightbox.component.html`
- Modify: `src/app/components/lightbox/lightbox.component.css`

- [ ] **Step 1: Load and expose faces in `lightbox.component.ts`**

Update `LightboxComponent` to load faces when the image changes and resolve subject names.

```typescript
  faces = signal<Face[]>([]);
  activeFaceId = signal<number | null>(null);

  ngOnChanges() {
    if (this.image) {
      const id = 'id' in this.image ? this.image.id : this.image.image_id;
      this.photos.loadFacesForImage(id).then(f => this.faces.set(f));
    } else {
      this.faces.set([]);
    }
  }

  getSubjectName(subjectId: number | null): string {
    if (!subjectId) return 'Unnamed Subject';
    const sub = this.photos.subjects().find(s => s.id === subjectId);
    return sub?.name || 'Unnamed Subject';
  }

  setActiveFace(id: number | null) {
    this.activeFaceId.set(id);
  }
```

- [ ] **Step 2: Add overlays and sidebar people list to `lightbox.component.html`**

Update the image container and sidebar in `lightbox.component.html`:

```html
        <div class="image-container" style="position: relative;">
          <img
            [src]="originalUrl(image)"
            [alt]="filename(image)"
            class="main-image"
            [style.view-transition-name]="'photo-' + ('id' in image ? image.id : image.image_id)"
            [style.background-image]="'url(' + thumbUrl(image) + ')'"
            style="background-size: contain; background-repeat: no-repeat; background-position: center;"
          />
          @if (showSidebar()) {
            @for (face of faces(); track face.id) {
              <div 
                class="face-overlay"
                [class.active]="activeFaceId() === face.id"
                [style.left.%]="face.bbox_x * 100"
                [style.top.%]="face.bbox_y * 100"
                [style.width.%]="face.bbox_w * 100"
                [style.height.%]="face.bbox_h * 100"
                (mouseenter)="setActiveFace(face.id)"
                (mouseleave)="setActiveFace(null)">
                <div class="face-label" *ngIf="activeFaceId() === face.id">{{ getSubjectName(face.subject_id) }}</div>
              </div>
            }
          }
        </div>
```

Append the People section to the `.sidebar-content`:

```html
          <div class="meta-section">
            <h3>People</h3>
            @if (faces().length === 0) {
              <div class="value">No people detected</div>
            } @else {
              <div class="people-list">
                @for (face of faces(); track face.id) {
                  <div 
                    class="person-item"
                    [class.active]="activeFaceId() === face.id"
                    (mouseenter)="setActiveFace(face.id)"
                    (mouseleave)="setActiveFace(null)">
                    <div class="person-avatar">👤</div>
                    <div class="person-name">{{ getSubjectName(face.subject_id) }}</div>
                  </div>
                }
              </div>
            }
          </div>
```

- [ ] **Step 3: Add CSS for overlays and list**

Add to `lightbox.component.css`:

```css
.face-overlay {
  position: absolute;
  border: 2px solid rgba(255, 255, 255, 0.5);
  box-shadow: 0 0 0 1px rgba(0,0,0,0.5), inset 0 0 0 1px rgba(0,0,0,0.5);
  border-radius: 4px;
  pointer-events: auto;
  cursor: crosshair;
  transition: all 0.2s ease;
}
.face-overlay:hover, .face-overlay.active {
  border-color: #0071e3;
  background: rgba(0, 113, 227, 0.2);
  z-index: 10;
}
.face-label {
  position: absolute;
  bottom: -24px;
  left: 50%;
  transform: translateX(-50%);
  background: rgba(0, 0, 0, 0.8);
  color: white;
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 12px;
  white-space: nowrap;
}
.meta-section { margin-top: 24px; }
.meta-section h3 { margin-bottom: 12px; font-size: 14px; font-weight: 600; color: #fff; }
.people-list { display: flex; flex-direction: column; gap: 8px; }
.person-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.2s ease;
}
.person-item:hover, .person-item.active { background: rgba(255, 255, 255, 0.1); }
.person-avatar { width: 32px; height: 32px; background: #333; border-radius: 50%; display: flex; align-items: center; justify-content: center; }
.person-name { font-size: 14px; font-weight: 500; }
```

- [ ] **Step 4: Commit**

```bash
git add src/app/components/lightbox/
git commit -m "feat(ui): add face bounding boxes and sidebar people list to lightbox"
```

### Task 3: Backend - Subject Search Integration

**Files:**
- Modify: `src-tauri/src/db.rs`
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Add subject search queries to `db.rs`**

```rust
pub async fn search_subjects_by_name(pool: &SqlitePool, query: &str) -> Result<Vec<Subject>> {
    let like_query = format!("%{}%", query);
    let rows = sqlx::query(
        "SELECT id, name, thumbnail_face_id, type, added_at 
         FROM subjects 
         WHERE name LIKE ? COLLATE NOCASE 
         ORDER BY added_at DESC"
    )
    .bind(like_query)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Subject {
            id: r.get("id"),
            name: r.get("name"),
            thumbnail_face_id: r.get("thumbnail_face_id"),
            subject_type: r.get("type"),
            added_at: r.get("added_at"),
        })
        .collect())
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
```

- [ ] **Step 2: Update `search_images` in `commands.rs`**

Rewrite the `search_images` function to merge fuzzy subject results with RAG results.

```rust
#[tauri::command]
pub async fn search_images(
    query: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let pool = &state.pool;

    // 1. Fuzzy Subject Search
    let matched_subjects = db::search_subjects_by_name(pool, &query).await.unwrap_or_default();
    let subject_ids: Vec<i64> = matched_subjects.iter().map(|s| s.id).collect();
    let subject_image_ids = db::get_image_ids_for_subjects(pool, &subject_ids).await.unwrap_or_default();

    let mut final_results = vec![];
    
    // Map explicit subject matches to SearchResult with a high score (1.0)
    for image_id in &subject_image_ids {
        if let Ok(Some(img)) = db::get_image_by_id(pool, *image_id).await {
            final_results.push(SearchResult {
                image_id: *image_id,
                path: img.path,
                thumbnail_path: img.thumbnail_path,
                score: 1.0,
                date_taken: img.date_taken,
                date_file: img.date_file,
                embed_status: img.embed_status,
            });
        }
    }

    // 2. RAG Search Fallback
    let api_key = {
        let lock = state.api_key.lock().await;
        lock.clone()
    };

    if let Some(api_key) = api_key {
        let client = Client::new();
        if let Ok(query_embedding) = crate::embedder::embed_text(&client, &api_key, &query).await {
            if let Ok(scored) = search::search_images(pool, query_embedding, 50).await {
                if let Ok(rag_results) = search::build_search_results(pool, scored).await {
                    for res in rag_results {
                        if !subject_image_ids.contains(&res.image_id) {
                            final_results.push(res);
                        }
                    }
                }
            }
        }
    }

    Ok(final_results)
}
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/db.rs src-tauri/src/commands.rs
git commit -m "feat(search): merge fuzzy subject search results with RAG embeddings"
```

### Task 4: Frontend - People View Search Navigation

**Files:**
- Modify: `src/app/components/people-view/people-view.component.ts`
- Modify: `src/app/components/people-view/people-view.component.html`
- Modify: `src/app/components/people-view/people-view.component.css`

- [ ] **Step 1: Add navigation method to `people-view.component.ts`**

```typescript
  async searchPerson(subject: Subject) {
    if (!subject.name) return; // Cannot search unnamed subjects via text yet
    this.photoService.currentView.set('gallery');
    await this.photoService.search(subject.name);
  }
```

- [ ] **Step 2: Update template to be clickable in `people-view.component.html`**

Update the card markup to handle clicks while preventing the edit button from triggering the search.

```html
        <div class="subject-info">
          @if (editingId === subject.id) {
            <div class="edit-mode">
              <input [(ngModel)]="editName" (keyup.enter)="saveEdit(subject)" (keyup.escape)="cancelEdit()" autofocus />
              <button (click)="saveEdit(subject)">Save</button>
              <button (click)="cancelEdit()">Cancel</button>
            </div>
          } @else {
            <div class="view-mode">
              <span class="name" [class.unnamed]="!subject.name" [class.clickable]="!!subject.name" (click)="searchPerson(subject)">
                {{ subject.name || 'Unnamed Subject' }}
              </span>
              <button class="edit-btn" (click)="$event.stopPropagation(); startEdit(subject)">✏️</button>
            </div>
          }
        </div>
```
*Also make the face thumbnail trigger the search:*
```html
        <div class="face-thumbnail" [class.clickable]="!!subject.name" (click)="searchPerson(subject)">
          <div class="placeholder-face">👤</div>
        </div>
```

- [ ] **Step 3: Add CSS for clickable items**

Add to `people-view.component.css`:

```css
.clickable {
  cursor: pointer;
}
.clickable:hover {
  text-decoration: underline;
  opacity: 0.8;
}
```

- [ ] **Step 4: Commit**

```bash
git add src/app/components/people-view/
git commit -m "feat(ui): allow clicking a person to search for their photos"
```
