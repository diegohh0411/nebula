import { Component, inject, OnInit, AfterViewInit, OnDestroy, effect, ElementRef, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';
import { MergeSuggestion, Subject } from '../../models/models';
import { RouterLink } from '@angular/router';
import { MergeReviewComponent } from '../merge-review/merge-review.component';
import { EditableTextComponent } from '../editable-text/editable-text.component';
import { PageHeaderComponent } from '../page-header/page-header.component';

const FACE_CROP_CACHE_CAP = 200;

@Component({
  selector: 'app-people-view',
  standalone: true,
  imports: [CommonModule, RouterLink, MergeReviewComponent, EditableTextComponent, PageHeaderComponent],
  templateUrl: './people-view.component.html',
  styleUrl: './people-view.component.css'
})
export class PeopleViewComponent implements OnInit, AfterViewInit, OnDestroy {
  protected photoService = inject(PhotoService);
  // Map preserves insertion order, enabling O(1) recency-based (LRU) eviction.
  protected faceCropUrls = signal<Map<number, string>>(new Map());
  protected mergeSuggestions = signal<MergeSuggestion[]>([]);
  protected suggestionCropUrls = signal<Record<number, string>>({});
  protected reviewingSuggestion = signal<MergeSuggestion | null>(null);

  /** Tracks which subject should enter edit mode next (used for Tab chaining). */
  editingSubjectId = signal<number | null>(null);
  protected namingConflict = signal<MergeSuggestion | null>(null);

  private _originalSubjects = new Map<number, Subject>();
  private host = inject(ElementRef<HTMLElement>);
  private observer?: IntersectionObserver;

  constructor() {
    // Re-observe cards whenever the subjects list changes (initial load, post-merge reload, etc.)
    effect(() => {
      this.photoService.subjects();
      setTimeout(() => this.observeCards(), 0);
    });
  }

  async ngOnInit() {
    await this.photoService.loadSubjects();
    void this.loadMergeSuggestions();
  }

  ngAfterViewInit(): void {
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const subjectId = Number((e.target as HTMLElement).dataset['subjectId']);
          if (!Number.isNaN(subjectId)) {
            void this.loadFaceCropForSubject(subjectId);
            this.observer?.unobserve(e.target);
          }
        }
      },
      { root: null, rootMargin: '300px', threshold: 0.01 }
    );
  }

  ngOnDestroy(): void {
    this.observer?.disconnect();
  }

  private observeCards(): void {
    if (!this.observer) return;
    this.observer.disconnect();
    const loaded = this.faceCropUrls();
    const cards = this.host.nativeElement.querySelectorAll('[data-subject-id]') as NodeListOf<HTMLElement>;
    cards.forEach((el: HTMLElement) => {
      const subjectId = Number(el.dataset['subjectId']);
      if (!loaded.has(subjectId)) this.observer!.observe(el);
    });
  }

  private async loadFaceCropForSubject(subjectId: number): Promise<void> {
    if (this.faceCropUrls().has(subjectId)) return;
    const subject = this.photoService.subjects().find(s => s.id === subjectId);
    if (!subject?.thumbnail_face_id) return;
    try {
      const path = await this.photoService.getFaceCrop(subject.thumbnail_face_id);
      const url = this.photoService.thumbnailUrl(path);
      if (url) {
        this.faceCropUrls.update(urls => {
          const next = new Map(urls);
          next.delete(subjectId);   // re-insert so it becomes most-recently-used
          next.set(subjectId, url);
          return this.withCap(next);
        });
      }
    } catch (e) {
      console.error(`Failed to load thumbnail for subject ${subjectId}`, e);
    }
  }

  private withCap(urls: Map<number, string>): Map<number, string> {
    // Evict oldest entries (front of insertion order) until within cap.
    while (urls.size > FACE_CROP_CACHE_CAP) {
      const oldest = urls.keys().next().value;
      if (oldest === undefined) break;
      urls.delete(oldest);
    }
    return urls;
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

  protected openReview(suggestion: MergeSuggestion) {
    this.reviewingSuggestion.set(suggestion);
  }

  async onConfirmed() {
    this.reviewingSuggestion.set(null);
    this.faceCropUrls.set(new Map());
    await Promise.all([this.photoService.loadSubjects(), this.loadMergeSuggestions()]);
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
    return this.suggestionCropUrls()[subject.thumbnail_face_id] ?? this.faceCropUrls().get(subject.id) ?? null;
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
    this.faceCropUrls.set(new Map());
    void Promise.all([this.photoService.loadSubjects(), this.loadMergeSuggestions()]);
  }

  protected onConflictDismissed(): void {
    this.namingConflict.set(null);
  }
}
