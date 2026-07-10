import {
  Component,
  Input,
  Output,
  EventEmitter,
  inject,
  signal,
  ChangeDetectionStrategy,
  ViewChild,
  ElementRef,
  HostListener,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { CdkTrapFocus } from '@angular/cdk/a11y';
import { PhotoService } from '../../services/photo.service';
import { MergeSuggestion, SubjectPhotoFace, Subject } from '../../models/models';
import { MergePhotoGridComponent } from '../merge-photo-grid/merge-photo-grid.component';
import { prefersReducedMotion } from '../../utils/motion';

interface MergeTarget {
  target: Subject;
  source: Subject;
}

@Component({
  selector: 'app-merge-review',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, MergePhotoGridComponent, CdkTrapFocus],
  templateUrl: './merge-review.component.html',
  styleUrl: './merge-review.component.css',
})
export class MergeReviewComponent {
  private _suggestion: MergeSuggestion | null = null;
  private _loadGen = 0;

  @Input()
  set suggestion(value: MergeSuggestion | null) {
    this._suggestion = value;
    this.subjectA.set(value?.subject_a ?? null);
    this.subjectB.set(value?.subject_b ?? null);
    void this.loadPhotos(value);
  }
  get suggestion(): MergeSuggestion | null { return this._suggestion; }

  @Input() canDismiss = true;

  @Output() confirmed = new EventEmitter<void>();
  @Output() dismissed = new EventEmitter<void>();
  @Output() closed = new EventEmitter<void>();

  @ViewChild('colA') colARef?: ElementRef<HTMLElement>;
  @ViewChild('colB') colBRef?: ElementRef<HTMLElement>;

  private photoService = inject(PhotoService);

  subjectA = signal<Subject | null>(null);
  subjectB = signal<Subject | null>(null);
  photosA = signal<SubjectPhotoFace[]>([]);
  photosB = signal<SubjectPhotoFace[]>([]);
  protected loading = signal(false);
  protected submitting = signal(false);

  get mergeTarget(): MergeTarget | null {
    const subjectA = this.subjectA();
    const subjectB = this.subjectB();
    if (!subjectA || !subjectB) return null;
    const aName = !!subjectA.name;
    const bName = !!subjectB.name;
    if (aName && !bName) return { target: subjectA, source: subjectB };
    if (bName && !aName) return { target: subjectB, source: subjectA };
    // Both named or both unnamed: lower id wins
    return subjectA.id <= subjectB.id
      ? { target: subjectA, source: subjectB }
      : { target: subjectB, source: subjectA };
  }

  protected onFaceRemovedA(faceId: number): void {
    this.photosA.update((list) => list.filter((f) => f.face_id !== faceId));
  }

  protected onFaceRemovedB(faceId: number): void {
    this.photosB.update((list) => list.filter((f) => f.face_id !== faceId));
  }

  private async loadPhotos(value: MergeSuggestion | null) {
    const gen = ++this._loadGen;
    if (!value) {
      this.photosA.set([]);
      this.photosB.set([]);
      this.loading.set(false);
      return;
    }
    this.loading.set(true);
    try {
      const [photosA, photosB] = await Promise.all([
        this.photoService.getSubjectPhotosWithFaces(value.subject_a.id),
        this.photoService.getSubjectPhotosWithFaces(value.subject_b.id),
      ]);
      if (gen !== this._loadGen) return; // stale, discard
      this.photosA.set(photosA);
      this.photosB.set(photosB);
    } finally {
      if (gen === this._loadGen) this.loading.set(false);
    }
  }

  async confirm() {
    const target = this.mergeTarget;
    if (!target || this.submitting()) return;
    this.submitting.set(true);
    try {
      await this.runMergeAnimation(target);
      await this.photoService.mergeSubjects(target.target.id, target.source.id);
      this.confirmed.emit();
    } catch (e) {
      console.error('MergeReview: merge failed', e);
    } finally {
      this.submitting.set(false);
    }
  }

  async dismiss() {
    if (!this._suggestion || this.submitting()) return;
    if (!this.canDismiss) {
      this.dismissed.emit();
      return;
    }
    this.submitting.set(true);
    try {
      await this.photoService.dismissMergeSuggestion(this._suggestion.id);
      this.dismissed.emit();
    } catch (e) {
      console.error('MergeReview: dismiss failed', e);
    } finally {
      this.submitting.set(false);
    }
  }

  close() {
    if (!this._suggestion || this.submitting()) return;
    this.closed.emit();
  }

  @HostListener('document:keydown.escape')
  protected onEscape() {
    if (!this._suggestion) return;
    this.close();
  }

  protected async runMergeAnimation(target: MergeTarget) {
    if (prefersReducedMotion()) {
      return;
    }
    const colA = this.colARef?.nativeElement;
    const colB = this.colBRef?.nativeElement;
    if (!colA || !colB) return;

    const { gsap } = await import('gsap');

    const isTargetA = target.target.id === this._suggestion!.subject_a.id;
    const sourceEl = isTargetA ? colB : colA;
    const targetEl = isTargetA ? colA : colB;

    const sourceRect = sourceEl.getBoundingClientRect();
    const targetRect = targetEl.getBoundingClientRect();
    const dx = targetRect.left - sourceRect.left;

    await gsap.timeline()
      .to(sourceEl, { x: dx, opacity: 0, duration: 0.35, ease: 'power2.in' })
      .to(targetEl, { scale: 1.04, duration: 0.15, ease: 'power1.out' }, '<0.2')
      .to(targetEl, { scale: 1, duration: 0.15, ease: 'power1.in' })
      .then();

    gsap.set([sourceEl, targetEl], { clearProps: 'all' });
  }
}
