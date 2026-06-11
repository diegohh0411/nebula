import { Component, inject, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';
import { MergeSuggestion, Subject } from '../../models/models';
import { RouterLink } from '@angular/router';
import { MergeReviewComponent } from '../merge-review/merge-review.component';
import { EditableTextComponent } from '../editable-text/editable-text.component';

@Component({
  selector: 'app-people-view',
  standalone: true,
  imports: [CommonModule, RouterLink, MergeReviewComponent, EditableTextComponent],
  templateUrl: './people-view.component.html',
  styleUrl: './people-view.component.css'
})
export class PeopleViewComponent implements OnInit {
  protected photoService = inject(PhotoService);
  protected faceCropUrls = signal<Record<number, string>>({});
  protected mergeSuggestions = signal<MergeSuggestion[]>([]);
  protected suggestionCropUrls = signal<Record<number, string>>({});
  protected reviewingSuggestion = signal<MergeSuggestion | null>(null);

  /** Tracks which subject should enter edit mode next (used for Tab chaining). */
  editingSubjectId = signal<number | null>(null);
  protected namingConflict = signal<MergeSuggestion | null>(null);

  private _originalSubjects = new Map<number, Subject>();

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

  protected async onNameCommit(subject: Subject, value: string): Promise<void> {
    const name = value || null;

    this.editingSubjectId.set(null);
    this._originalSubjects.set(subject.id, { ...subject });
    this.photoService.subjects.update(subjects =>
      subjects.map(s => s.id === subject.id ? { ...s, name } : s)
    );

    let result: { duplicate_subject_id: number | null };
    try {
      result = await this.photoService.nameSubject(subject.id, name);
    } catch (e) {
      console.error('nameSubject failed, reverting', e);
      const original = this._originalSubjects.get(subject.id);
      this._originalSubjects.delete(subject.id);
      if (original) {
        this.photoService.subjects.update(subjects =>
          subjects.map(s => s.id === original.id ? original : s)
        );
      }
      return;
    }

    if (result.duplicate_subject_id) {
      const duplicate = this.photoService.subjects().find(s => s.id === result.duplicate_subject_id);
      if (duplicate) {
        const currentSubject = this.photoService.subjects().find(s => s.id === subject.id) ?? { ...subject };
        const currentWithName: Subject = { ...currentSubject, name };
        this.namingConflict.set({ id: -1, subject_a: duplicate, subject_b: currentWithName, score: 1.0 });
      } else {
        this._originalSubjects.delete(subject.id);
      }
    } else {
      this._originalSubjects.delete(subject.id);
    }
  }

  protected onNameTab(subject: Subject): void {
    const subjects = this.photoService.subjects();
    const idx = subjects.findIndex(s => s.id === subject.id);
    const nextUnnamed = subjects.slice(idx + 1).find(s => !s.name) ?? null;
    if (nextUnnamed) {
      this.editingSubjectId.set(nextUnnamed.id);
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
