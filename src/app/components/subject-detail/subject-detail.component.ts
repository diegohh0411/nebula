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
import { PhotoService } from '../../services/photo.service';
import { SearchResult, VirtualRow, SubjectDetail, MergeSuggestion } from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { FormsModule } from '@angular/forms';
import { buildJustifiedRows } from '../../utils/justified-layout';
import { LightboxComponent } from '../lightbox/lightbox.component';

@Component({
  selector: 'app-subject-detail',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CommonModule,
    RouterLink,
    LucideAngularModule,
    PhotoGridComponent,
    FormsModule,
    LightboxComponent,
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
  protected faceCropUrl = signal<string | null>(null);

  protected isEditingName = signal(false);
  protected editedName = signal('');
  protected isMenuOpen = signal(false);
  protected isSavingName = signal(false);

  protected similarSubjects = signal<MergeSuggestion[]>([]);
  protected similarCropUrls = signal<Record<number, string>>({});
  protected showNameConflict = signal(false);
  protected conflictingSubjectId = signal<number | null>(null);

  protected readonly virtualRows = computed<VirtualRow[]>(() => {
    const images = this.subjectPhotos();
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
      this.editedName.set(detail.subject.name || '');

      if (detail.subject.thumbnail_face_id) {
        const path = await this.photos.getFaceCrop(detail.subject.thumbnail_face_id);
        this.faceCropUrl.set(this.photos.thumbnailUrl(path));
      }

      const photos = await this.photos.getSubjectPhotos(id);
      this.subjectPhotos.set(photos);

      void this.loadSimilarSubjects(id);
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

  protected startEdit() {
    this.isEditingName.set(true);
  }

  protected cancelEdit() {
    this.isEditingName.set(false);
    this.editedName.set(this.detail()?.subject.name || '');
  }

  protected async saveName() {
    if (this.isSavingName() || !this.isEditingName()) return;
    const id = this.subjectId();
    const name = this.editedName().trim();
    if (id !== null) {
      this.isSavingName.set(true);
      try {
        const result = await this.photos.nameSubject(id, name || null);
        this.detail.update((d) => {
          if (d) d.subject.name = name || null;
          return d;
        });
        this.isEditingName.set(false);

        if (result.duplicate_subject_id) {
          this.conflictingSubjectId.set(result.duplicate_subject_id);
          this.showNameConflict.set(true);
        }
      } finally {
        this.isSavingName.set(false);
      }
    }
  }

  protected async confirmMerge() {
    const id = this.subjectId();
    const conflictId = this.conflictingSubjectId();
    if (id !== null && conflictId !== null) {
      await this.photos.mergeSubjects(id, conflictId);
      this.showNameConflict.set(false);
      this.conflictingSubjectId.set(null);
      this.router.navigate(['/subject', id]);
    }
  }

  protected cancelMerge() {
    this.showNameConflict.set(false);
    this.conflictingSubjectId.set(null);
  }

  protected async mergeSimilar(suggestion: MergeSuggestion) {
    const id = this.subjectId();
    if (id === null) return;
    const sourceId =
      suggestion.subject_a.id === id
        ? suggestion.subject_b.id
        : suggestion.subject_a.id;
    await this.photos.mergeSubjects(id, sourceId);
    void this.loadData(id);
  }

  protected async dismissSimilar(suggestion: MergeSuggestion) {
    await this.photos.dismissMergeSuggestion(suggestion.id);
    this.similarSubjects.update((list) =>
      list.filter((s) => s.id !== suggestion.id)
    );
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
}
