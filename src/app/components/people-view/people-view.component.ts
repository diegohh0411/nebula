import { Component, inject, OnInit, signal, ViewChildren, QueryList, ElementRef } from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';
import { MergeSuggestion, Subject } from '../../models/models';
import { RouterLink } from '@angular/router';
import { MergeReviewComponent } from '../merge-review/merge-review.component';

@Component({
  selector: 'app-people-view',
  standalone: true,
  imports: [CommonModule, RouterLink, MergeReviewComponent],
  templateUrl: './people-view.component.html',
  styleUrl: './people-view.component.css'
})
export class PeopleViewComponent implements OnInit {
  protected photoService = inject(PhotoService);
  protected faceCropUrls = signal<Record<number, string>>({});
  protected mergeSuggestions = signal<MergeSuggestion[]>([]);
  protected suggestionCropUrls = signal<Record<number, string>>({});
  protected reviewingSuggestion = signal<MergeSuggestion | null>(null);

  editingSubjectId = signal<number | null>(null);
  editingName = signal<string>('');
  protected namingConflict = signal<MergeSuggestion | null>(null);

  @ViewChildren('nameInput') private nameInputRefs!: QueryList<ElementRef<HTMLInputElement>>;

  async ngOnInit() {
    await this.photoService.loadSubjects();
    void this.loadMergeSuggestions();
    void this.loadThumbnails();
  }

  private async loadMergeSuggestions() {
    try {
      const suggestions = await this.photoService.getMergeSuggestions(3);
      this.mergeSuggestions.set(suggestions);
      void this.loadSuggestionCrops(suggestions);
    } catch (e) {
      console.error('Failed to load merge suggestions', e);
    }
  }

  private async loadSuggestionCrops(suggestions: MergeSuggestion[]) {
    const ids = new Set<number>();
    for (const s of suggestions) {
      if (s.subject_a.thumbnail_face_id) ids.add(s.subject_a.thumbnail_face_id);
      if (s.subject_b.thumbnail_face_id) ids.add(s.subject_b.thumbnail_face_id);
    }
    const urls: Record<number, string> = {};
    await Promise.all(
      [...ids].map(async (faceId) => {
        try {
          const path = await this.photoService.getFaceCrop(faceId);
          const url = this.photoService.thumbnailUrl(path);
          if (url) urls[faceId] = url;
        } catch {}
      })
    );
    this.suggestionCropUrls.set(urls);
  }

  private async loadThumbnails() {
    const subjects = this.photoService.subjects();
    const urls: Record<number, string> = {};

    await Promise.all(subjects.map(async (s) => {
      if (s.thumbnail_face_id) {
        try {
          const path = await this.photoService.getFaceCrop(s.thumbnail_face_id);
          const url = this.photoService.thumbnailUrl(path);
          if (url) urls[s.id] = url;
        } catch (e) {
          console.error(`Failed to load thumbnail for subject ${s.id}`, e);
        }
      }
    }));

    this.faceCropUrls.set(urls);
  }

  protected openReview(suggestion: MergeSuggestion) {
    this.reviewingSuggestion.set(suggestion);
  }

  async onConfirmed() {
    this.reviewingSuggestion.set(null);
    await Promise.all([this.loadThumbnails(), this.loadMergeSuggestions()]);
  }

  async onDismissed() {
    const current = this.reviewingSuggestion();
    if (current) {
      this.mergeSuggestions.update((list) => list.filter((s) => s.id !== current.id));
    }
    this.reviewingSuggestion.set(null);
  }

  onClosed() {
    this.reviewingSuggestion.set(null);
  }

  protected getThumbUrl(subject: Subject): string | null {
    if (!subject.thumbnail_face_id) return null;
    return this.suggestionCropUrls()[subject.thumbnail_face_id] ?? this.faceCropUrls()[subject.id] ?? null;
  }

  protected startEditing(subject: Subject, event: Event): void {
    event.stopPropagation();
    this.editingSubjectId.set(subject.id);
    this.editingName.set('');
  }

  protected async commitName(subject: Subject): Promise<void> {
    if (this.editingSubjectId() !== subject.id) return;
    const name = this.editingName().trim();
    if (!name) { this.cancelEditing(); return; }

    this.photoService.subjects.update(subjects =>
      subjects.map(s => s.id === subject.id ? { ...s, name } : s)
    );
    this.editingSubjectId.set(null);
    this.editingName.set('');

    const result = await this.photoService.nameSubject(subject.id, name);

    if (result.duplicate_subject_id) {
      const duplicate = this.photoService.subjects().find(s => s.id === result.duplicate_subject_id);
      if (duplicate) {
        const current = this.photoService.subjects().find(s => s.id === subject.id) ?? { ...subject, name };
        this.namingConflict.set({ id: -1, subject_a: duplicate, subject_b: current, score: 1.0 });
      }
    }
  }

  protected cancelEditing(): void {
    this.editingSubjectId.set(null);
    this.editingName.set('');
  }

  protected onKeydown(event: KeyboardEvent, subject: Subject): void {
    if (event.key === 'Enter') {
      void this.commitName(subject);
    } else if (event.key === 'Escape') {
      this.cancelEditing();
    } else if (event.key === 'Tab') {
      event.preventDefault();
      const subjects = this.photoService.subjects();
      const idx = subjects.findIndex(s => s.id === subject.id);
      const nextUnnamed = subjects.slice(idx + 1).find(s => !s.name) ?? null;
      void this.commitName(subject);
      if (nextUnnamed) {
        this.editingSubjectId.set(nextUnnamed.id);
        this.editingName.set('');
        setTimeout(() => this.nameInputRefs.first?.nativeElement.focus(), 0);
      }
    }
  }

  protected onConflictConfirmed(): void {
    this.namingConflict.set(null);
    void Promise.all([this.loadThumbnails(), this.loadMergeSuggestions()]);
  }

  protected onConflictDismissed(): void {
    this.namingConflict.set(null);
  }
}
