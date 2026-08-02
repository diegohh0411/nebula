import {
  Component,
  OnInit,
  inject,
  signal,
  computed,
  ChangeDetectionStrategy,
} from '@angular/core';
import { CommonModule, Location } from '@angular/common';
import { ActivatedRoute, RouterLink, Router } from '@angular/router';
import { open } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { PhotoService } from '../../services/photo.service';
import {
  SearchResult,
  VirtualRow,
  SubjectDetail,
  MergeSuggestion,
  ExportSubjectProgress,
} from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { buildJustifiedRows } from '../../utils/justified-layout';
import { LightboxComponent } from '../lightbox/lightbox.component';
import { EditableTextComponent } from '../editable-text/editable-text.component';
import { ConfirmMergeDialogComponent } from '../confirm-merge-dialog/confirm-merge-dialog.component';
import { MergeReviewComponent } from '../merge-review/merge-review.component';
import { injectSubjectTagging } from '../../composables/subject-tagging.composable';
import { HlmInput } from '@spartan-ng/helm/input';
import { GridControlsComponent } from '../grid-controls/grid-controls.component';
import { createImageCollection } from '../../composables/image-collection.composable';

@Component({
  selector: 'app-subject-detail',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CommonModule,
    RouterLink,
    LucideAngularModule,
    PhotoGridComponent,
    LightboxComponent,
    EditableTextComponent,
    ConfirmMergeDialogComponent,
    MergeReviewComponent,
    HlmInput,
    GridControlsComponent,
  ],
  templateUrl: './subject-detail.component.html',
  styleUrl: './subject-detail.component.css',
})
export class SubjectDetailComponent implements OnInit {
  private route = inject(ActivatedRoute);
  private location = inject(Location);
  private router = inject(Router);
  protected photos = inject(PhotoService);

  protected subjectId = signal<number | null>(null);
  protected detail = signal<SubjectDetail | null>(null);
  protected subjectPhotos = signal<SearchResult[]>([]);
  protected readonly collection = createImageCollection(this.subjectPhotos, {
    sortKeys: ['dateTaken', 'relevance'],
    defaultSort: { key: 'dateTaken', direction: 'desc' },
    dateRangeFilter: true,
  });
  protected faceCropUrl = signal<string | null>(null);

  protected isMenuOpen = signal(false);
  protected exporting = signal(false);
  protected exportProgress = signal<ExportSubjectProgress | null>(null);
  protected exportStatus = signal<string | null>(null);

  protected similarSubjects = signal<MergeSuggestion[]>([]);
  protected similarCropUrls = signal<Record<number, string>>({});
  protected reviewingSuggestion = signal<MergeSuggestion | null>(null);

  protected readonly tagging = injectSubjectTagging(this.subjectId, {
    onNameSaved: (name) =>
      this.detail.update((d) => (d ? { ...d, subject: { ...d.subject, name } } : d)),
    onMerged: (id) => {
      this.router.navigate(['/subject', id]);
    },
  });

  protected readonly virtualRows = computed<VirtualRow[]>(() => {
    const images = this.collection.view();
    const width = this.photos.viewportWidth();
    const targetRowHeight = this.photos.targetRowHeight();

    const rows: VirtualRow[] = [];
    const justifiedRows = buildJustifiedRows(images, width, targetRowHeight, 4);
    for (const row of justifiedRows) {
      rows.push({ type: 'row', images: row.images, rowHeight: row.rowHeight });
    }
    return rows;
  });

  ngOnInit() {
    this.route.params.subscribe((params) => {
      const id = Number(params['id']);
      if (!isNaN(id)) {
        this.subjectId.set(id);
        void this.loadData(id);
      }
    });
  }

  private async loadData(id: number) {
    try {
      const detail = await this.photos.getSubjectDetail(id);
      this.detail.set(detail);

      if (detail.subject.thumbnail_face_id) {
        const path = await this.photos.getFaceCrop(detail.subject.thumbnail_face_id);
        this.faceCropUrl.set(this.photos.thumbnailUrl(path));
      }

      const photos = await this.photos.getSubjectPhotos(id);
      this.subjectPhotos.set(photos);

      void this.loadSimilarSubjects(id);
      const tags = await this.photos.getSubjectTags(id);
      this.tagging.tags.set(tags);
    } catch (e) {
      console.error('Failed to load subject detail', e);
      this.location.back();
    }
  }

  private async loadSimilarSubjects(id: number) {
    try {
      const all = await this.photos.getMergeSuggestions();
      const related = all.filter(
        (s) => s.subject_a.id === id || s.subject_b.id === id
      );
      this.similarSubjects.set(related);
      void this.loadSimilarCrops(related);
    } catch (e) {
      console.error('Failed to load similar subjects', e);
    }
  }

  private async loadSimilarCrops(suggestions: MergeSuggestion[]) {
    const ids = new Set<number>();
    for (const s of suggestions) {
      if (s.subject_a.thumbnail_face_id) ids.add(s.subject_a.thumbnail_face_id);
      if (s.subject_b.thumbnail_face_id) ids.add(s.subject_b.thumbnail_face_id);
    }
    const urls: Record<number, string> = {};
    await Promise.all(
      [...ids].map(async (faceId) => {
        try {
          const path = await this.photos.getFaceCrop(faceId);
          const url = this.photos.thumbnailUrl(path);
          if (url) urls[faceId] = url;
        } catch {}
      })
    );
    this.similarCropUrls.set(urls);
  }

  protected goBack() {
    this.location.back();
  }

  protected openReview(suggestion: MergeSuggestion) {
    this.reviewingSuggestion.set(suggestion);
  }

  protected onReviewConfirmed(survivingId: number) {
    const current = this.subjectId();
    this.reviewingSuggestion.set(null);
    if (current === null) return;
    if (survivingId === current) {
      void this.loadData(current);
    } else {
      void this.router.navigate(['/subject', survivingId]);
    }
  }

  protected onReviewDismissed() {
    const current = this.reviewingSuggestion();
    if (current) {
      this.similarSubjects.update((list) => list.filter((s) => s.id !== current.id));
    }
    this.reviewingSuggestion.set(null);
  }

  protected onReviewClosed() {
    this.reviewingSuggestion.set(null);
  }

  protected getOtherSubject(suggestion: MergeSuggestion) {
    const id = this.subjectId();
    return suggestion.subject_a.id === id
      ? suggestion.subject_b
      : suggestion.subject_a;
  }

  protected getSimilarThumbUrl(subject: { thumbnail_face_id: number | null }): string | null {
    if (!subject.thumbnail_face_id) return null;
    return this.similarCropUrls()[subject.thumbnail_face_id] ?? null;
  }

  protected toggleMenu() {
    this.isMenuOpen.update((v) => !v);
  }

  protected closeMenu() {
    this.isMenuOpen.set(false);
  }

  protected async onCopyAll(): Promise<void> {
    if (this.exporting()) return;
    const id = this.subjectId();
    if (id === null) return;

    const selected = await open({ directory: true, multiple: false });
    if (!selected || typeof selected !== 'string') return;

    this.exporting.set(true);
    this.exportProgress.set({ current: 0, total: 0 });
    this.exportStatus.set(null);

    let unlisten: UnlistenFn | undefined;
    try {
      unlisten = await listen<ExportSubjectProgress>('export_subject_progress', (e) => {
        this.exportProgress.set(e.payload);
      });

      const result = await this.photos.exportSubjectPhotos(id, selected);

      const parts = [`Copied ${result.copied} photo${result.copied === 1 ? '' : 's'}`];
      if (result.skipped_missing > 0) {
        parts.push(`${result.skipped_missing} missing`);
      }
      if (result.skipped_errors > 0) {
        parts.push(`${result.skipped_errors} failed`);
      }
      this.exportStatus.set(parts.join(' · '));

      try {
        await openPath(result.dest_dir);
      } catch (openErr) {
        console.error('Failed to open export folder', openErr);
        this.exportStatus.set(`${parts.join(' · ')} (could not open folder)`);
      }
    } catch (e) {
      console.error('Export failed', e);
      const msg = e instanceof Error ? e.message : String(e);
      this.exportStatus.set(msg || 'Export failed');
    } finally {
      if (unlisten) unlisten();
      this.exporting.set(false);
      this.exportProgress.set(null);
    }
  }
}
