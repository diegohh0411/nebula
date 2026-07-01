# Folder Coverage Report & Saved Reports Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the Folder Coverage feature by adding a Saved Reports system (backend DB + commands) and the full frontend UI (list of reports, report detail view with grids of missing/present people).

**Architecture:** We add `saved_reports` and `saved_report_tags` to the SQLite DB. Backend exposes commands to CRUD reports. Frontend Angular app adds a `/reports` route and service methods, mapping `SubjectCoverage` results to the existing `SubjectPersonCardComponent`.

**Tech Stack:** Rust, sqlx, Tauri, Angular, TypeScript

---

### Task 1: Backend Database Changes

**Files:**
- Modify: `src-tauri/src/db/mod.rs`

- [ ] **Step 1: Update BASE_SCHEMA**

Add the `saved_reports` and `saved_report_tags` tables to `BASE_SCHEMA` string in `src-tauri/src/db/mod.rs`.

```rust
CREATE TABLE IF NOT EXISTS saved_reports (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT NOT NULL,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    added_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS saved_report_tags (
    report_id INTEGER NOT NULL REFERENCES saved_reports(id) ON DELETE CASCADE,
    tag_id    INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (report_id, tag_id)
);
```

- [ ] **Step 2: Add Versioned Migration**

In `VERSIONED_MIGRATIONS` inside `src-tauri/src/db/mod.rs`, add version `3`.

```rust
    (
        3,
        "CREATE TABLE IF NOT EXISTS saved_reports (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE, added_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS saved_report_tags (report_id INTEGER NOT NULL REFERENCES saved_reports(id) ON DELETE CASCADE, tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE, PRIMARY KEY (report_id, tag_id));"
    ),
```

- [ ] **Step 3: Run Compiler**

Run: `cd src-tauri && cargo check`

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/db/mod.rs
git commit -m "feat(db): add saved_reports schema and migration"
```

---

### Task 2: Backend Models & Repo

**Files:**
- Modify: `src-tauri/src/people/models.rs`
- Modify: `src-tauri/src/people/repo.rs`
- Modify: `src-tauri/src/db/tests.rs`

- [ ] **Step 1: Add SavedReport struct**

In `src-tauri/src/people/models.rs`, add:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SavedReport {
    pub id: i64,
    pub name: String,
    pub folder_id: i64,
    pub tag_ids: Vec<i64>,
}
```

- [ ] **Step 2: Add Repo Methods**

In `src-tauri/src/people/repo.rs`, add:

```rust
use crate::people::models::SavedReport;
use std::time::{SystemTime, UNIX_EPOCH};

pub async fn create_saved_report(
    pool: &sqlx::SqlitePool,
    name: &str,
    folder_id: i64,
    tag_ids: &[i64],
) -> anyhow::Result<SavedReport> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let mut tx = pool.begin().await?;

    let report_id = sqlx::query("INSERT INTO saved_reports (name, folder_id, added_at) VALUES (?, ?, ?)")
        .bind(name)
        .bind(folder_id)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();

    for tag_id in tag_ids {
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
        tag_ids: tag_ids.to_vec(),
    })
}

pub async fn list_saved_reports(pool: &sqlx::SqlitePool) -> anyhow::Result<Vec<SavedReport>> {
    let rows = sqlx::query("SELECT id, name, folder_id FROM saved_reports ORDER BY added_at DESC")
        .fetch_all(pool)
        .await?;

    let mut reports = Vec::new();
    for row in rows {
        let id: i64 = row.get("id");
        let name: String = row.get("name");
        let folder_id: i64 = row.get("folder_id");

        let tag_rows = sqlx::query("SELECT tag_id FROM saved_report_tags WHERE report_id = ?")
            .bind(id)
            .fetch_all(pool)
            .await?;
            
        let tag_ids = tag_rows.iter().map(|r| r.get("tag_id")).collect();

        reports.push(SavedReport {
            id,
            name,
            folder_id,
            tag_ids,
        });
    }

    Ok(reports)
}

pub async fn delete_saved_report(pool: &sqlx::SqlitePool, report_id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM saved_reports WHERE id = ?")
        .bind(report_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 3: Run Compiler**

Run: `cd src-tauri && cargo check`

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/people/models.rs src/people/repo.rs
git commit -m "feat(repo): add repo methods for saved reports"
```

---

### Task 3: Tauri Commands

**Files:**
- Modify: `src-tauri/src/people/commands.rs`
- Modify: `src-tauri/src/app/mod.rs`

- [ ] **Step 1: Add commands to people/commands.rs**

Add:
```rust
use crate::people::models::SavedReport;

#[tauri::command]
pub async fn create_saved_report(
    name: String,
    folder_id: i64,
    tag_ids: Vec<i64>,
    state: tauri::State<'_, AppState>,
) -> Result<SavedReport, String> {
    repo::create_saved_report(&state.pool, &name, folder_id, &tag_ids)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn list_saved_reports(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SavedReport>, String> {
    repo::list_saved_reports(&state.pool)
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn delete_saved_report(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    repo::delete_saved_report(&state.pool, id)
        .await
        .map_err(map_err)
}
```

- [ ] **Step 2: Register commands in app/mod.rs**

In `src-tauri/src/app/mod.rs`, add them to the `generate_handler!` macro:
```rust
            crate::people::commands::create_saved_report,
            crate::people::commands::list_saved_reports,
            crate::people::commands::delete_saved_report,
```

- [ ] **Step 3: Check build**

Run: `cd src-tauri && cargo check`

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/people/commands.rs src/app/mod.rs
git commit -m "feat(tauri): expose saved reports commands"
```

---

### Task 4: Frontend Types & PhotoService

**Files:**
- Modify: `src/app/models/models.ts`
- Modify: `src/app/services/photo.service.ts`

- [ ] **Step 1: Add types to models.ts**

Add at the end of `src/app/models/models.ts`:
```typescript
export interface CoverageSummary {
  total_targets: number;
  present_targets: number;
}
export interface SubjectCoverage {
  subject_id: number;
  name: string;
  frequency: number;
}
export interface CoverageReport {
  summary: CoverageSummary;
  missing_targets: SubjectCoverage[];
  present_targets: SubjectCoverage[];
  others_found: SubjectCoverage[];
}
export interface SavedReport {
  id: number;
  name: string;
  folder_id: number;
  tag_ids: number[];
}
```

- [ ] **Step 2: Add service methods to photo.service.ts**

Import the new interfaces at the top and add these methods to `PhotoService`:
```typescript
  async getFolderCoverage(folderId: number, tagIds: number[]): Promise<CoverageReport> {
    return await invoke<CoverageReport>('get_folder_coverage', { folderId, tagIds });
  }
  async createSavedReport(name: string, folderId: number, tagIds: number[]): Promise<SavedReport> {
    return await invoke<SavedReport>('create_saved_report', { name, folderId, tagIds });
  }
  async listSavedReports(): Promise<SavedReport[]> {
    return await invoke<SavedReport[]>('list_saved_reports');
  }
  async deleteSavedReport(id: number): Promise<void> {
    await invoke('delete_saved_report', { id });
  }
```

- [ ] **Step 3: Commit**

```bash
git add src/app/models/models.ts src/app/services/photo.service.ts
git commit -m "feat(frontend): add photo.service methods for reports"
```

---

### Task 5: Frontend Reports List Component

**Files:**
- Create: `src/app/components/reports/reports.component.ts`
- Create: `src/app/components/reports/reports.component.html`
- Create: `src/app/components/reports/reports.component.css`

- [ ] **Step 1: Write component TS**

`src/app/components/reports/reports.component.ts`:
```typescript
import { Component, OnInit, inject, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SavedReport, TagWithCount, Folder } from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';
import { FormsModule } from '@angular/forms';

@Component({
  selector: 'app-reports',
  standalone: true,
  imports: [RouterLink, LucideAngularModule, FormsModule],
  templateUrl: './reports.component.html',
  styleUrl: './reports.component.css',
})
export class ReportsComponent implements OnInit {
  protected photos = inject(PhotoService);
  private router = inject(Router);

  protected reports = signal<SavedReport[]>([]);
  protected tags = signal<TagWithCount[]>([]);
  
  protected isCreating = signal(false);
  protected newName = signal('');
  protected newFolderId = signal<number | null>(null);
  protected newTagId = signal<number | null>(null); // Simplified single tag for now

  async ngOnInit() {
    await this.loadData();
  }

  async loadData() {
    const [reps, tgs] = await Promise.all([
      this.photos.listSavedReports(),
      this.photos.listTags()
    ]);
    this.reports.set(reps);
    this.tags.set(tgs);
  }

  protected async deleteReport(id: number, e: Event) {
    e.stopPropagation();
    await this.photos.deleteSavedReport(id);
    await this.loadData();
  }

  protected getFolderName(id: number): string {
    const folder = this.photos.folders().find(f => f.id === id);
    if (!folder) return 'Unknown Folder';
    return folder.path.replace(/\\/g, '/').split('/').filter(Boolean).pop() ?? folder.path;
  }

  protected getTagsDesc(tagIds: number[]): string {
    const allTags = this.tags();
    return tagIds.map(id => allTags.find(t => t.id === id)?.name ?? 'Unknown').join(', ');
  }

  protected async createReport() {
    const fId = this.newFolderId();
    const tId = this.newTagId();
    const name = this.newName().trim();
    if (!fId || !tId || !name) return;

    const rep = await this.photos.createSavedReport(name, fId, [tId]);
    this.isCreating.set(false);
    this.newName.set('');
    void this.router.navigate(['/reports', rep.id]);
  }
}
```

- [ ] **Step 2: Write component HTML**

`src/app/components/reports/reports.component.html`:
```html
<div class="reports-container">
  <div class="header">
    <h2>Saved Reports</h2>
    <button class="btn-primary" (click)="isCreating.set(true)">
      <lucide-icon name="plus" size="16"></lucide-icon> New Report
    </button>
  </div>

  @if (isCreating()) {
    <div class="create-form">
      <input type="text" placeholder="Report Name" [ngModel]="newName()" (ngModelChange)="newName.set($event)">
      <select [ngModel]="newFolderId()" (ngModelChange)="newFolderId.set($event * 1)">
        <option [value]="null">Select Folder...</option>
        @for (f of photos.folders(); track f.id) {
          <option [value]="f.id">{{ getFolderName(f.id) }}</option>
        }
      </select>
      <select [ngModel]="newTagId()" (ngModelChange)="newTagId.set($event * 1)">
        <option [value]="null">Select Tag (Roster)...</option>
        @for (t of tags(); track t.id) {
          <option [value]="t.id">{{ t.name }}</option>
        }
      </select>
      <button class="btn-primary" (click)="createReport()">Save & Run</button>
      <button class="btn-secondary" (click)="isCreating.set(false)">Cancel</button>
    </div>
  }

  <div class="grid">
    @for (rep of reports(); track rep.id) {
      <div class="card" [routerLink]="['/reports', rep.id]">
        <div class="card-header">
          <h3>{{ rep.name }}</h3>
          <button class="btn-icon" (click)="deleteReport(rep.id, $event)">
            <lucide-icon name="trash-2" size="16"></lucide-icon>
          </button>
        </div>
        <div class="card-body">
          <p><strong>Folder:</strong> {{ getFolderName(rep.folder_id) }}</p>
          <p><strong>Tags:</strong> {{ getTagsDesc(rep.tag_ids) }}</p>
        </div>
      </div>
    }
  </div>
</div>
```

- [ ] **Step 3: Write CSS**

`src/app/components/reports/reports.component.css`:
```css
.reports-container { padding: 24px; max-width: 1000px; margin: 0 auto; }
.header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px; }
h2 { font-size: 24px; font-weight: 600; color: #fff; margin: 0; }
.btn-primary { background: #3b82f6; color: white; border: none; padding: 8px 16px; border-radius: 6px; display: flex; align-items: center; gap: 8px; cursor: pointer; }
.btn-primary:hover { background: #2563eb; }
.btn-secondary { background: transparent; color: #94a3b8; border: 1px solid #475569; padding: 8px 16px; border-radius: 6px; cursor: pointer; }
.btn-icon { background: transparent; border: none; color: #ef4444; cursor: pointer; opacity: 0.7; }
.btn-icon:hover { opacity: 1; }
.create-form { background: #1e293b; padding: 16px; border-radius: 8px; display: flex; gap: 12px; margin-bottom: 24px; align-items: center; border: 1px solid #334155; }
.create-form input, .create-form select { background: #0f172a; border: 1px solid #334155; color: white; padding: 8px; border-radius: 4px; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 16px; }
.card { background: #1e293b; border: 1px solid #334155; border-radius: 8px; padding: 16px; cursor: pointer; transition: border-color 0.2s; }
.card:hover { border-color: #3b82f6; }
.card-header { display: flex; justify-content: space-between; margin-bottom: 12px; }
.card-header h3 { margin: 0; font-size: 16px; color: #f8fafc; }
.card-body p { margin: 4px 0; font-size: 14px; color: #94a3b8; }
.card-body strong { color: #cbd5e1; }
```

- [ ] **Step 4: Commit**

```bash
git add src/app/components/reports/
git commit -m "feat(frontend): add Reports list component"
```

---

### Task 6: Frontend Report Detail Component

**Files:**
- Create: `src/app/components/report-detail/report-detail.component.ts`
- Create: `src/app/components/report-detail/report-detail.component.html`
- Create: `src/app/components/report-detail/report-detail.component.css`

- [ ] **Step 1: Write component TS**

`src/app/components/report-detail/report-detail.component.ts`:
```typescript
import { Component, OnInit, inject, signal } from '@angular/core';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SavedReport, CoverageReport, SubjectCoverage, SubjectMatch, Tag } from '../../models/models';
import { SubjectPersonCardComponent } from '../subject-person-card/subject-person-card.component';
import { LucideAngularModule } from 'lucide-angular';

@Component({
  selector: 'app-report-detail',
  standalone: true,
  imports: [SubjectPersonCardComponent, LucideAngularModule, RouterLink],
  templateUrl: './report-detail.component.html',
  styleUrl: './report-detail.component.css',
})
export class ReportDetailComponent implements OnInit {
  private route = inject(ActivatedRoute);
  protected photos = inject(PhotoService);

  protected report = signal<SavedReport | null>(null);
  protected coverage = signal<CoverageReport | null>(null);
  
  protected missingMatches = signal<SubjectMatch[]>([]);
  protected presentMatches = signal<SubjectMatch[]>([]);
  protected othersMatches = signal<SubjectMatch[]>([]);

  async ngOnInit() {
    const id = Number(this.route.snapshot.paramMap.get('id'));
    if (!id) return;

    const reports = await this.photos.listSavedReports();
    const rep = reports.find(r => r.id === id);
    if (!rep) return;
    this.report.set(rep);

    await this.photos.loadSubjects();

    const cov = await this.photos.getFolderCoverage(rep.folder_id, rep.tag_ids);
    this.coverage.set(cov);

    this.missingMatches.set(this.mapToMatches(cov.missing_targets));
    this.presentMatches.set(this.mapToMatches(cov.present_targets));
    this.othersMatches.set(this.mapToMatches(cov.others_found));
  }

  private mapToMatches(covList: SubjectCoverage[]): SubjectMatch[] {
    const allSubjects = this.photos.subjects();
    return covList.map(item => {
      let subject = allSubjects.find(s => s.id === item.subject_id);
      if (!subject) {
        // Fallback for missing subjects not loaded in cache
        subject = { id: item.subject_id, name: item.name, thumbnail_face_id: null, type: 'person', added_at: 0 };
      }
      const fakeTag: Tag = { id: -1, name: `${item.frequency} photos`, added_at: 0 };
      return { subject, tags: [fakeTag] };
    });
  }
}
```

- [ ] **Step 2: Write component HTML**

`src/app/components/report-detail/report-detail.component.html`:
```html
<div class="report-detail">
  <div class="header">
    <a routerLink="/reports" class="back-link"><lucide-icon name="arrow-left" size="16"></lucide-icon> Back</a>
    @if (report(); as rep) {
      <h2>{{ rep.name }}</h2>
    }
  </div>

  @if (coverage(); as cov) {
    <div class="summary">
      <strong>Coverage Summary:</strong> {{ cov.summary.present_targets }} of {{ cov.summary.total_targets }} target subjects present.
    </div>

    <div class="sections">
      <div class="section">
        <h3>Missing Targets ({{ cov.missing_targets.length }})</h3>
        <div class="cards-grid">
          @for (match of missingMatches(); track match.subject.id) {
            <app-subject-person-card [match]="match" />
          }
        </div>
      </div>

      <div class="section">
        <h3>Present Targets ({{ cov.present_targets.length }})</h3>
        <div class="cards-grid">
          @for (match of presentMatches(); track match.subject.id) {
            <app-subject-person-card [match]="match" />
          }
        </div>
      </div>

      <div class="section">
        <h3>Others Found ({{ cov.others_found.length }})</h3>
        <div class="cards-grid">
          @for (match of othersMatches(); track match.subject.id) {
            <app-subject-person-card [match]="match" />
          }
        </div>
      </div>
    </div>
  } @else {
    <div class="loading">Generating report...</div>
  }
</div>
```

- [ ] **Step 3: Write CSS**

`src/app/components/report-detail/report-detail.component.css`:
```css
.report-detail { padding: 24px; max-width: 1200px; margin: 0 auto; }
.header { display: flex; align-items: center; gap: 16px; margin-bottom: 24px; }
.back-link { display: flex; align-items: center; gap: 4px; color: #94a3b8; text-decoration: none; font-size: 14px; }
.back-link:hover { color: #f8fafc; }
h2 { margin: 0; font-size: 24px; color: #fff; }
.summary { background: #1e293b; border: 1px solid #334155; padding: 16px; border-radius: 8px; margin-bottom: 24px; color: #e2e8f0; font-size: 16px; }
.section { margin-bottom: 32px; }
h3 { font-size: 18px; color: #cbd5e1; margin-bottom: 16px; padding-bottom: 8px; border-bottom: 1px solid #334155; }
.cards-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 16px; }
.loading { color: #94a3b8; font-size: 16px; }
```

- [ ] **Step 4: Commit**

```bash
git add src/app/components/report-detail/
git commit -m "feat(frontend): add Report Detail component"
```

---

### Task 7: Frontend Routing & Sidebar Update

**Files:**
- Modify: `src/app/app.routes.ts`
- Modify: `src/app/components/sidebar/sidebar.component.ts`
- Modify: `src/app/components/sidebar/sidebar.component.html`

- [ ] **Step 1: Update app.routes.ts**

Import the components and add the routes:
```typescript
import { Routes } from "@angular/router";
import { GalleryComponent } from "./components/gallery/gallery.component";
import { PeopleViewComponent } from "./components/people-view/people-view.component";
import { TagsViewComponent } from "./components/tags-view/tags-view.component";
import { SubjectDetailComponent } from "./components/subject-detail/subject-detail.component";
import { FacePickerComponent } from "./components/face-picker/face-picker.component";
import { SettingsComponent } from "./components/settings/settings.component";
import { ReportsComponent } from "./components/reports/reports.component";
import { ReportDetailComponent } from "./components/report-detail/report-detail.component";

export const routes: Routes = [
  { path: "", component: GalleryComponent },
  { path: "people", component: PeopleViewComponent },
  { path: "tags", component: TagsViewComponent },
  { path: "subject/:id", component: SubjectDetailComponent },
  { path: "subject/:id/face-picker", component: FacePickerComponent },
  { path: "reports", component: ReportsComponent },
  { path: "reports/:id", component: ReportDetailComponent },
  { path: "settings", component: SettingsComponent },
];
```

- [ ] **Step 2: Update Sidebar Component TS**

In `src/app/components/sidebar/sidebar.component.ts`, add `isReportsActive()`:
```typescript
  protected isReportsActive(): boolean {
    return this.router.url.startsWith('/reports');
  }
```

- [ ] **Step 3: Update Sidebar HTML**

In `src/app/components/sidebar/sidebar.component.html`, add the Reports link after Tags:
```html
    <app-sidebar-item
      [isActive]="isTagsActive()"
      routerLink="/tags"
    >
      <span class="folder-icon">
        <lucide-icon name="tag" size="14"></lucide-icon>
      </span>
      <span class="folder-name">Tags</span>
    </app-sidebar-item>

    <app-sidebar-item
      [isActive]="isReportsActive()"
      routerLink="/reports"
    >
      <span class="folder-icon">
        <lucide-icon name="file-text" size="14"></lucide-icon>
      </span>
      <span class="folder-name">Reports</span>
    </app-sidebar-item>
```

- [ ] **Step 4: Commit**

```bash
git add src/app/app.routes.ts src/app/components/sidebar/
git commit -m "feat(frontend): add reports to sidebar and routing"
```
