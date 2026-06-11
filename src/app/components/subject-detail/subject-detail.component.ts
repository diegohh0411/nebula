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
import { SearchResult, VirtualRow, SubjectDetail, MergeSuggestion, Tag, TagWithCount } from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { buildJustifiedRows } from '../../utils/justified-layout';
import { LightboxComponent } from '../lightbox/lightbox.component';
import { EditableTextComponent } from '../editable-text/editable-text.component';

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

  protected isMenuOpen = signal(false);

  protected similarSubjects = signal<MergeSuggestion[]>([]);
  protected similarCropUrls = signal<Record<number, string>>({});
  protected showNameConflict = signal(false);
  protected conflictingSubjectId = signal<number | null>(null);

  protected tags = signal<Tag[]>([]);
  protected allTags = signal<TagWithCount[]>([]);
  protected newTagName = signal('');
  protected tagError = signal<string | null>(null);

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

      if (detail.subject.thumbnail_face_id) {
        const path = await this.photos.getFaceCrop(detail.subject.thumbnail_face_id);
        this.faceCropUrl.set(this.photos.thumbnailUrl(path));
      }

      const photos = await this.photos.getSubjectPhotos(id);
      this.subjectPhotos.set(photos);

      void this.loadSimilarSubjects(id);
      const tags = await this.photos.getSubjectTags(id);
      this.tags.set(tags);
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

  protected async saveName(value: string): Promise<void> {
    const id = this.subjectId();
    if (id === null) return;
    const name = value || null;
    try {
      const result = await this.photos.nameSubject(id, name);
      this.detail.update((d) => {
        if (d) d.subject.name = name;
        return d;
      });
      if (result.duplicate_subject_id) {
        this.conflictingSubjectId.set(result.duplicate_subject_id);
        this.showNameConflict.set(true);
      }
    } catch (e) {
      console.error('Failed to save name', e);
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

  protected async onTagFocus(): Promise<void> {
    try {
      const all = await this.photos.listTags();
      this.allTags.set(all);
    } catch { /* ignore */ }
  }

  protected async addTag(): Promise<void> {
    const id = this.subjectId();
    const name = this.newTagName().trim();
    if (!name || id === null) return;
    try {
      this.tagError.set(null);
      await this.photos.addSubjectTag(id, name);
      this.newTagName.set('');
      const tags = await this.photos.getSubjectTags(id);
      this.tags.set(tags);
    } catch (e: unknown) {
      this.tagError.set(typeof e === 'string' ? e : 'Failed to add tag');
    }
  }

  protected async removeTag(tagId: number): Promise<void> {
    const id = this.subjectId();
    if (id === null) return;
    try {
      await this.photos.removeSubjectTag(id, tagId);
      this.tags.update((ts) => ts.filter((t) => t.id !== tagId));
    } catch { /* ignore */ }
  }
}
