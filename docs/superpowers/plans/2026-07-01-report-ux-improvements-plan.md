# Report UX Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement UX improvements for the Reports feature including updating the sidebar icon, adding dark mode support, adding CRUD UI actions to the Report detail view, and natively rendering the frequency count for each subject.

**Architecture:** We will replace the hardcoded colors in the reports CSS files with Tailwind semantic `@apply` directives, update the backend and frontend services to support renaming reports, update `SubjectPersonCardComponent` to accept a `subtitle` property for displaying the frequency count, and update `APP_ICONS` for Lucide.

**Tech Stack:** Angular, TailwindCSS, Tauri (Rust).

---

### Task 1: Update Sidebar Icon & Register Icons

**Files:**
- Modify: `src/app/components/sidebar/sidebar.component.html`
- Modify: `src/app/app-icons.ts`

- [ ] **Step 1: Register missing Lucide icons**
Modify `src/app/app-icons.ts` to include `FileBarChart` and `Trash2` (and make sure to export them in the `APP_ICONS` object).

```typescript
import {
  // ... other icons
  FileBarChart,
  Trash2,
} from 'lucide-angular';

export const APP_ICONS = {
  // ... other icons
  FileBarChart,
  Trash2,
};
```

- [ ] **Step 2: Update sidebar icon**
Modify `src/app/components/sidebar/sidebar.component.html` (around line 48) to change `name="file-text"` to `name="file-bar-chart"`.

```html
    <app-sidebar-item
      [isActive]="isReportsActive()"
      routerLink="/reports"
    >
      <span class="folder-icon">
        <lucide-icon name="file-bar-chart" size="14"></lucide-icon>
      </span>
      <span class="folder-name">Reports</span>
    </app-sidebar-item>
```

### Task 2: Implement Dark/Light Mode Theme Support

**Files:**
- Modify: `src/app/components/reports/reports.component.css`
- Modify: `src/app/components/report-detail/report-detail.component.css`

- [ ] **Step 1: Refactor reports list CSS**
Replace the hardcoded colors in `src/app/components/reports/reports.component.css` with Tailwind `@apply` using semantic tokens.

```css
.reports-container { @apply p-6 max-w-5xl mx-auto; }
.header { @apply flex justify-between items-center mb-6; }
h2 { @apply text-2xl font-semibold m-0 text-foreground; }
.btn-primary { @apply bg-primary text-primary-foreground border-none px-4 py-2 rounded-md flex items-center gap-2 cursor-pointer hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed; }
.btn-secondary { @apply bg-transparent text-muted-foreground border border-border px-4 py-2 rounded-md cursor-pointer hover:bg-muted; }
.btn-icon { @apply bg-transparent border-none text-destructive cursor-pointer opacity-70 hover:opacity-100 transition-opacity; }
.create-form { @apply bg-card p-4 rounded-lg flex gap-3 mb-6 items-center border border-border; }
.create-form input, .create-form select { @apply bg-background border border-border text-foreground p-2 rounded-md outline-none focus:ring-2 focus:ring-ring; }
.grid { @apply grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-4; }
.card { @apply bg-card border border-border rounded-lg p-4 cursor-pointer transition-colors hover:border-primary block no-underline text-foreground; }
.card-header { @apply flex justify-between mb-3 items-center; }
.card-header h3 { @apply m-0 text-base font-semibold text-foreground; }
.card-body p { @apply my-1 text-sm text-muted-foreground; }
.card-body strong { @apply text-foreground font-medium; }
```

- [ ] **Step 2: Refactor report detail CSS**
Replace the hardcoded colors in `src/app/components/report-detail/report-detail.component.css`.

```css
.report-detail { @apply p-6 max-w-6xl mx-auto; }
.header { @apply flex items-center gap-4 mb-6; }
.header-actions { @apply flex items-center gap-2 ml-auto; }
.btn { @apply bg-transparent text-muted-foreground border border-border px-3 py-1.5 rounded-md cursor-pointer hover:bg-muted hover:text-foreground text-sm transition-colors; }
.btn-danger { @apply bg-destructive/10 text-destructive border border-destructive/20 hover:bg-destructive/20; }
.back-link { @apply flex items-center gap-1 text-muted-foreground no-underline text-sm hover:text-foreground transition-colors; }
h2 { @apply m-0 text-2xl font-semibold text-foreground; }
.summary { @apply bg-card border border-border p-4 rounded-lg mb-6 text-foreground text-base; }
.section { @apply mb-8; }
h3 { @apply text-lg font-medium text-foreground mb-4 pb-2 border-b border-border; }
.cards-grid { @apply grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-4; }
.loading { @apply text-muted-foreground text-base flex items-center gap-2; }
.error-msg { @apply bg-destructive/10 border border-destructive/20 p-4 rounded-lg text-destructive mb-6 text-sm; }
```

### Task 3: Backend Support for Renaming Reports

**Files:**
- Modify: `src-tauri/src/people/repo.rs`
- Modify: `src-tauri/src/people/commands.rs`
- Modify: `src-tauri/src/app/mod.rs`

- [ ] **Step 1: Add update_saved_report_name to repo**
Add to `src-tauri/src/people/repo.rs`:

```rust
pub async fn update_saved_report_name(pool: &sqlx::SqlitePool, report_id: i64, new_name: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE saved_reports SET name = ? WHERE id = ?")
        .bind(new_name)
        .bind(report_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 2: Add tauri command**
Add to `src-tauri/src/people/commands.rs`:

```rust
#[tauri::command]
pub async fn update_saved_report_name(
    state: tauri::State<'_, crate::app::AppState>,
    id: i64,
    name: String,
) -> Result<(), String> {
    repo::update_saved_report_name(&state.pool, id, &name)
        .await
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Register command in app**
In `src-tauri/src/app/mod.rs`, add `crate::people::commands::update_saved_report_name` to the `invoke_handler(tauri::generate_handler![...])` list.

### Task 4: Frontend Support for Updating Reports & Report Detail CRUD UI

**Files:**
- Modify: `src/app/services/photo.service.ts`
- Modify: `src/app/components/report-detail/report-detail.component.ts`
- Modify: `src/app/components/report-detail/report-detail.component.html`

- [ ] **Step 1: Add updateSavedReportName to photo.service.ts**
```typescript
  async updateSavedReportName(id: number, name: string): Promise<void> {
    await invoke('update_saved_report_name', { id, name });
  }
```

- [ ] **Step 2: Add CRUD methods to ReportDetailComponent**
Inject `Router` in `ReportDetailComponent` (`private router = inject(Router);`).
Add methods for renaming and deleting:

```typescript
  protected async editReportName() {
    const rep = this.report();
    if (!rep) return;
    const newName = prompt('Enter new report name:', rep.name);
    if (newName !== null && newName.trim() !== '') {
      try {
        await this.photos.updateSavedReportName(rep.id, newName.trim());
        this.report.set({ ...rep, name: newName.trim() });
      } catch (err: any) {
        console.error('Failed to rename report:', err);
        alert('Failed to rename report');
      }
    }
  }

  protected async deleteReport() {
    const rep = this.report();
    if (!rep) return;
    if (confirm('Are you sure you want to delete this report?')) {
      try {
        await this.photos.deleteSavedReport(rep.id);
        void this.router.navigate(['/reports']);
      } catch (err: any) {
        console.error('Failed to delete report:', err);
        alert('Failed to delete report');
      }
    }
  }
```

- [ ] **Step 3: Update Report Detail HTML**
Modify `src/app/components/report-detail/report-detail.component.html` to add the buttons in the header:

```html
  <div class="header">
    <a routerLink="/reports" class="back-link"><lucide-icon name="arrow-left" size="16"></lucide-icon> Back</a>
    @if (report(); as rep) {
      <h2>{{ rep.name }}</h2>
      <div class="header-actions">
        <button class="btn" (click)="editReportName()">Edit Name</button>
        <button class="btn btn-danger" (click)="deleteReport()">Delete</button>
      </div>
    }
  </div>
```

### Task 5: Proper Subject Frequency Display

**Files:**
- Modify: `src/app/components/subject-person-card/subject-person-card.component.ts`
- Modify: `src/app/components/subject-person-card/subject-person-card.component.css`
- Modify: `src/app/components/subject-person-card/subject-person-card.component.html`
- Modify: `src/app/components/report-detail/report-detail.component.ts`
- Modify: `src/app/components/report-detail/report-detail.component.html`

- [ ] **Step 1: Add subtitle input to card**
In `src/app/components/subject-person-card/subject-person-card.component.ts`:
```typescript
  readonly subtitle = input<string>();
```

- [ ] **Step 2: Update card CSS**
In `src/app/components/subject-person-card/subject-person-card.component.css`, add the subtitle class:
```css
.person-card-subtitle {
  @apply text-[12px] text-white/70 tracking-tight truncate;
}
```

- [ ] **Step 3: Update card HTML**
In `src/app/components/subject-person-card/subject-person-card.component.html`, add the subtitle under `person-card-name`:
```html
    <span class="person-card-name">{{ displayName }}</span>
    @if (subtitle()) {
      <span class="person-card-subtitle">{{ subtitle() }}</span>
    }
```

- [ ] **Step 4: Pass subtitle and remove fakeTag**
In `src/app/components/report-detail/report-detail.component.ts`, change `SubjectMatch` tracking to pass frequency instead of fake tags. We will need to map `SubjectCoverage` along with the `SubjectMatch` or alter `SubjectMatch`. Since `SubjectMatch` expects `subject` and `tags`, we can just export an interface for the view or compute subtitle in the template.

Alternatively, since `mapToMatches` is only used here, we can return `{ match: SubjectMatch, frequency: number }`:

```typescript
// Add interface at the top or inside the component
interface ReportMatch {
  match: SubjectMatch;
  frequency: number;
}
```

Update the signals:
```typescript
  protected missingMatches = signal<ReportMatch[]>([]);
  protected presentMatches = signal<ReportMatch[]>([]);
  protected othersMatches = signal<ReportMatch[]>([]);
```

Update `mapToMatches`:
```typescript
  private mapToMatches(covList: SubjectCoverage[]): ReportMatch[] {
    const allSubjects = this.photos.subjects();
    return covList.map(item => {
      let subject = allSubjects.find(s => s.id === item.subject_id);
      if (!subject) {
        subject = { id: item.subject_id, name: item.name, thumbnail_face_id: null, type: 'person', added_at: 0 };
      }
      return { match: { subject, tags: [] }, frequency: item.frequency };
    });
  }
```

- [ ] **Step 5: Bind subtitle in HTML**
In `src/app/components/report-detail/report-detail.component.html`, update the loops and pass the subtitle:

```html
        <div class="cards-grid">
          @for (item of missingMatches(); track item.match.subject.id) {
            <app-subject-person-card [match]="item.match" [subtitle]="'Appears in ' + item.frequency + ' photo' + (item.frequency === 1 ? '' : 's')" />
          }
        </div>
```
Do the same for `presentMatches()` and `othersMatches()`.

- [ ] **Step 6: Build and Run Application**
Build the frontend using `npm run build` or verify with `ng lint` to make sure there are no errors in Angular templates, and `cargo check` in `src-tauri` to ensure the rust code is correct.
