import { Component, OnInit, inject, signal } from '@angular/core';
import { ActivatedRoute, RouterLink, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { PhotoService } from '../../services/photo.service';
import { SavedReport, CoverageReport, SubjectCoverage, SubjectMatch } from '../../models/models';
import { SubjectPersonCardComponent } from '../subject-person-card/subject-person-card.component';
import { LucideAngularModule } from 'lucide-angular';

export interface ReportMatch {
  match: SubjectMatch;
  frequency: number;
}

@Component({
  selector: 'app-report-detail',
  standalone: true,
  imports: [SubjectPersonCardComponent, LucideAngularModule, RouterLink, FormsModule],
  templateUrl: './report-detail.component.html',
  styleUrl: './report-detail.component.css',
})
export class ReportDetailComponent implements OnInit {
  private route = inject(ActivatedRoute);
  protected photos = inject(PhotoService);
  private router = inject(Router);

  protected isEditingName = signal(false);
  protected editNameValue = signal('');

  protected startEditName() {
    const rep = this.report();
    if (!rep) return;
    this.editNameValue.set(rep.name);
    this.isEditingName.set(true);
  }

  protected cancelEditName() {
    this.isEditingName.set(false);
  }

  protected async confirmEditName() {
    const rep = this.report();
    const newName = this.editNameValue().trim();
    if (!rep || !newName) return;

    try {
      await this.photos.updateSavedReportName(rep.id, newName);
      this.report.set({ ...rep, name: newName });
      this.isEditingName.set(false);
    } catch (err: any) {
      console.error('Failed to rename report:', err);
      alert('Failed to rename report');
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


  protected report = signal<SavedReport | null>(null);
  protected coverage = signal<CoverageReport | null>(null);
  protected error = signal<string | null>(null);

  protected missingMatches = signal<ReportMatch[]>([]);
  protected presentMatches = signal<ReportMatch[]>([]);
  protected othersMatches = signal<ReportMatch[]>([]);

  protected isPrioritizing = signal(false);
  protected prioritizedCount = signal<number | null>(null);

  protected getFolderNames(): string {
    const rep = this.report();
    if (!rep) return '';
    const folders = this.photos.folders();
    return rep.folder_ids
      .map(id => {
        const folder = folders.find(f => f.id === id);
        if (!folder) return 'Unknown Folder';
        return folder.path.replace(/\\/g, '/').split('/').filter(Boolean).pop() ?? folder.path;
      })
      .join(', ');
  }

  protected async prioritizeProcessing() {
    const rep = this.report();
    if (!rep || this.isPrioritizing()) return;
    this.isPrioritizing.set(true);
    try {
      const count = await this.photos.prioritizeReportProcessing(rep.id);
      this.prioritizedCount.set(count);
    } catch (err) {
      console.error('Failed to prioritize report processing:', err);
      alert('Failed to prioritize processing');
    } finally {
      this.isPrioritizing.set(false);
    }
  }

  async ngOnInit() {
    try {
      const id = Number(this.route.snapshot.paramMap.get('id'));
      if (!id) {
        this.error.set('Report not found');
        return;
      }

      const [rep] = await Promise.all([
        this.photos.getSavedReport(id),
        this.photos.loadSubjects(),
      ]);
      if (!rep) {
        this.error.set('Report not found');
        return;
      }
      this.report.set(rep);

      const cov = await this.photos.getFolderCoverage(rep.folder_ids, rep.tag_ids);
      this.coverage.set(cov);

      this.missingMatches.set(this.mapToMatches(cov.missing_targets));
      this.presentMatches.set(this.mapToMatches(cov.present_targets));
      this.othersMatches.set(this.mapToMatches(cov.others_found));
    } catch (err: any) {
      console.error('Failed to load report detail:', err);
      this.error.set(err.message || 'An error occurred loading the report');
    }
  }

  private mapToMatches(covList: SubjectCoverage[]): ReportMatch[] {
    const allSubjects = this.photos.subjects();
    return covList.map(item => {
      let subject = allSubjects.find(s => s.id === item.subject_id);
      if (!subject) {
        // Fallback for missing subjects not loaded in cache
        subject = { id: item.subject_id, name: item.name, thumbnail_face_id: null, type: 'person', added_at: 0 };
      }
      return { match: { subject, tags: [] }, frequency: item.frequency };
    });
  }
}
