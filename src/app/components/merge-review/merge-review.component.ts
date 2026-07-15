import {
  Component,
  Input,
  Output,
  EventEmitter,
  inject,
  signal,
  computed,
  effect,
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
import { EditableTextComponent } from '../editable-text/editable-text.component';
import { prefersReducedMotion } from '../../utils/motion';

interface MergeTarget {
  target: Subject;
  source: Subject;
}

@Component({
  selector: 'app-merge-review',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, MergePhotoGridComponent, CdkTrapFocus, EditableTextComponent],
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
    this.targetOverride.set(null);
    this.redirectSource.set(null);
    this.showRedirectPicker.set(false);
    this.redirectColumn.set(null);
    this.redirectQuery.set('');
    this.redirectHighlight.set(0);
    this.nameErrorA.set(null);
    this.nameErrorB.set(null);
    this.redirectGoneError.set(null);
    void this.loadPhotos(value);
  }
  get suggestion(): MergeSuggestion | null { return this._suggestion; }

  @Input() canDismiss = true;

  @Output() confirmed = new EventEmitter<number>();
  @Output() dismissed = new EventEmitter<void>();
  @Output() closed = new EventEmitter<void>();

  @ViewChild('colA') colARef?: ElementRef<HTMLElement>;
  @ViewChild('colB') colBRef?: ElementRef<HTMLElement>;
  @ViewChild('redirectInput') redirectInputRef?: ElementRef<HTMLInputElement>;

  private photoService = inject(PhotoService);

  subjectA = signal<Subject | null>(null);
  subjectB = signal<Subject | null>(null);
  photosA = signal<SubjectPhotoFace[]>([]);
  photosB = signal<SubjectPhotoFace[]>([]);
  protected loading = signal(false);
  protected submitting = signal(false);
  protected nameErrorA = signal<{ message: string; conflict: Subject } | null>(null);
  protected nameErrorB = signal<{ message: string; conflict: Subject } | null>(null);
  protected showExitConfirm = signal(false);
  protected showRedirectPicker = signal(false);
  protected targetOverride = signal<Subject | null>(null);
  protected redirectSource = signal<Subject | null>(null);
  protected redirectColumn = signal<'a' | 'b' | null>(null);
  protected redirectQuery = signal('');
  protected redirectHighlight = signal(0);
  protected redirectGoneError = signal<string | null>(null);

  protected redirectCandidates = computed<Subject[]>(() => {
    const query = this.redirectQuery().trim().toLowerCase();
    const merge = this.mergeTarget;
    const sourceId = merge?.source.id;
    const targetId = merge?.target.id; // the subject currently being kept (override when a redirect is active)
    return this.photoService.subjects().filter((s) => {
      if (!s.name) return false;
      if (s.id === sourceId) return false;
      if (s.id === targetId) return false; // never offer the subject already being kept
      if (!query) return true;
      return s.name.toLowerCase().includes(query);
    });
  });

  protected redirectAvatarUrls = signal<Map<number, string | null>>(new Map());

  constructor() {
    effect(() => {
      if (this.showRedirectPicker()) {
        this.loadRedirectAvatars(this.redirectCandidates());
      }
    });
  }

  private loadRedirectAvatars(candidates: Subject[]): void {
    const known = this.redirectAvatarUrls();
    const missing = candidates.filter((c) => !known.has(c.id));
    for (const candidate of missing) {
      if (!candidate.thumbnail_face_id) {
        this.redirectAvatarUrls.update((m) => new Map(m).set(candidate.id, null));
        continue;
      }
      this.photoService.getFaceCrop(candidate.thumbnail_face_id)
        .then((path) => {
          this.redirectAvatarUrls.update((m) => new Map(m).set(candidate.id, this.photoService.thumbnailUrl(path)));
        })
        .catch(() => {
          this.redirectAvatarUrls.update((m) => new Map(m).set(candidate.id, null));
        });
    }
  }

  protected namesIdentical = computed(() => {
    const a = this.subjectA()?.name?.trim().toLowerCase();
    const b = this.subjectB()?.name?.trim().toLowerCase();
    return !!a && !!b && a === b;
  });

  /** The duplicate-name nudge pulses the Merge button, but not when the user prefers reduced motion. */
  protected shouldPulse = computed(() => this.namesIdentical() && !prefersReducedMotion());

  /** Display name for a column, accounting for an active redirect into this slot. */
  protected columnDisplayName(which: 'a' | 'b'): string | null {
    if (this.targetOverride() && this.redirectColumn() === which) {
      return this.targetOverride()!.name;
    }
    return which === 'a' ? this.subjectA()?.name ?? null : this.subjectB()?.name ?? null;
  }

  /** Whether the `keep` badge belongs on this column, accounting for an active redirect. */
  protected columnIsKeep(which: 'a' | 'b'): boolean {
    if (this.targetOverride()) return this.redirectColumn() === which;
    const id = which === 'a' ? this.subjectA()?.id : this.subjectB()?.id;
    return this.mergeTarget?.target?.id === id;
  }

  get mergeTarget(): MergeTarget | null {
    const override = this.targetOverride();
    if (override) {
      const source = this.redirectSource();
      if (!source) return null;
      return { target: override, source };
    }
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

  protected async onNameCommit(which: 'a' | 'b', rawValue: string): Promise<void> {
    const subjSig = which === 'a' ? this.subjectA : this.subjectB;
    const otherSig = which === 'a' ? this.subjectB : this.subjectA;
    const errorSig = which === 'a' ? this.nameErrorA : this.nameErrorB;
    const subject = subjSig();
    if (!subject) return;
    errorSig.set(null);

    const typed = rawValue.trim();
    const newName = typed || null;

    // Case 3: matches a DIFFERENT existing subject (not either column in this modal) → block.
    if (typed) {
      const other = otherSig();
      const conflict = this.photoService.subjects().find(
        (s) =>
          s.id !== subject.id &&
          s.id !== other?.id &&
          (s.name ?? '').toLowerCase() === typed.toLowerCase(),
      );
      if (conflict) {
        errorSig.set({ message: `A subject named "${typed}" already exists.`, conflict });
        return; // no backend write; EditableText re-displays the unchanged signal value
      }
    }

    try {
      await this.photoService.nameSubject(subject.id, newName);
      subjSig.set({ ...subject, name: newName });
    } catch (e) {
      console.error('MergeReview: rename failed', e);
    }
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

  /** Which photo signal (A or B) is the redirect *target* slot — always the column OPPOSITE
   *  the merge source, so the source subject keeps its own column and the picked target
   *  replaces the non-participating column. This matters for the Part-2 collision entry point
   *  on the *kept* column: there the source is the kept column itself, so loading the target
   *  into the opposite column keeps the source (the subject actually being merged) visible and
   *  drops the bystander column, instead of overwriting the source and leaving an irrelevant
   *  subject on screen.
   *  Decided once, on the FIRST redirect in this modal session, from the *original*
   *  (pre-any-redirect) source, and reused unchanged by every subsequent pick — `redirectSource`
   *  is stable across re-picks, so recomputing would give the same column, but pinning it also
   *  documents the invariant. */
  private photosSignalFor(sourceId: number): typeof this.photosA {
    const column = this.redirectColumn() ?? (this._suggestion?.subject_a.id === sourceId ? 'b' : 'a');
    if (this.redirectColumn() === null) this.redirectColumn.set(column);
    return column === 'a' ? this.photosA : this.photosB;
  }

  /** Re-target the merge to `picked` instead of the original keep subject (or, when
   *  `explicitSource` is given, instead of whichever subject is explicitly passed — used by
   *  the Part 2 collision entry point, where the colliding rename may have happened on
   *  either column, not necessarily the current `mergeTarget.source`). Does not merge — the
   *  user must still click "Merge as {picked.name}" to confirm (see confirm()). */
  protected async applyRedirect(picked: Subject, explicitSource?: Subject): Promise<void> {
    const originalTarget = this.mergeTarget; // pre-redirect target/source, before override is set
    if (!originalTarget && !explicitSource) return;

    const source = explicitSource ?? originalTarget!.source;

    this.redirectSource.set(source);
    this.targetOverride.set(picked);
    this.showRedirectPicker.set(false);
    this.nameErrorA.set(null);
    this.nameErrorB.set(null);

    const gen = ++this._loadGen;
    const photosSig = this.photosSignalFor(source.id);
    try {
      const photos = await this.photoService.getSubjectPhotosWithFaces(picked.id);
      if (gen !== this._loadGen) return; // stale, discard
      photosSig.set(photos);
    } catch (e) {
      console.error('MergeReview: failed to load redirected subject faces', e);
    }
  }

  /** Update the filter query and reset the highlight — a narrowed list must never keep a
   *  now-out-of-range highlight index (Enter would otherwise be a silent no-op until an arrow
   *  key nudges it back into range). */
  protected onRedirectQueryInput(value: string): void {
    this.redirectQuery.set(value);
    this.redirectHighlight.set(0);
  }

  protected openRedirectPicker(): void {
    this.redirectQuery.set('');
    this.redirectHighlight.set(0);
    this.redirectGoneError.set(null);
    this.showRedirectPicker.set(true);
    queueMicrotask(() => this.redirectInputRef?.nativeElement.focus());
  }

  protected onRedirectKeydown(event: KeyboardEvent): void {
    const candidates = this.redirectCandidates();
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      this.redirectHighlight.update((i) => Math.min(i + 1, Math.max(candidates.length - 1, 0)));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      this.redirectHighlight.update((i) => Math.max(i - 1, 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const picked = candidates[this.redirectHighlight()];
      if (picked) void this.applyRedirect(picked);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation(); // must not bubble to @HostListener('document:keydown.escape')
      this.showRedirectPicker.set(false);
    }
  }

  protected pickRedirectCandidate(subject: Subject): void {
    void this.applyRedirect(subject);
  }

  async confirm() {
    const target = this.mergeTarget;
    if (!target || this.submitting()) return;

    const override = this.targetOverride();
    if (override && !this.photoService.subjects().some((s) => s.id === override.id)) {
      this.redirectGoneError.set(`${override.name} is no longer available — pick another subject.`);
      this.showRedirectPicker.set(true);
      return;
    }

    this.submitting.set(true);
    try {
      await this.runMergeAnimation(target);
      await this.photoService.mergeSubjects(target.target.id, target.source.id);
      this.confirmed.emit(target.target.id);
    } catch (e) {
      console.error('MergeReview: merge failed', e);
    } finally {
      this.submitting.set(false);
    }
  }

  async dismiss() {
    if (this.canDismiss && this.namesIdentical() && !this.submitting()) {
      this.showExitConfirm.set(true);
      return;
    }
    await this.doDismiss();
  }

  private async doDismiss() {
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
    if (this.canDismiss && this.namesIdentical() && !this.submitting()) {
      this.showExitConfirm.set(true);
      return;
    }
    this.doClose();
  }

  private doClose() {
    if (!this._suggestion || this.submitting()) return;
    this.closed.emit();
  }

  /** Exit-guard "Keep separate": abandon the merge WITHOUT writing a cannot_link mark. */
  protected keepSeparate() {
    this.showExitConfirm.set(false);
    this.doClose();
  }

  protected confirmFromGuard() {
    this.showExitConfirm.set(false);
    void this.confirm();
  }

  @HostListener('document:keydown.escape')
  protected onEscape() {
    if (!this._suggestion) return;
    this.close();
  }

  protected async runMergeAnimation(target: MergeTarget) {
    if (prefersReducedMotion() || this.targetOverride()) {
      return; // accepted v1 simplification: no animation for a redirected confirm (see spec)
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
