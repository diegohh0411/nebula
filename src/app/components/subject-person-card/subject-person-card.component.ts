import {
  Component, ChangeDetectionStrategy, inject, input, output, signal, OnInit,
} from '@angular/core';
import { Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch, Tag, TagWithCount } from '../../models/models';
import { EditableTextComponent } from '../editable-text/editable-text.component';
import { ConfirmMergeDialogComponent } from '../confirm-merge-dialog/confirm-merge-dialog.component';
import { HlmInput } from '@spartan-ng/helm/input';

@Component({
  selector: 'app-subject-person-card',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [EditableTextComponent, ConfirmMergeDialogComponent, HlmInput],
  templateUrl: './subject-person-card.component.html',
  styleUrl: './subject-person-card.component.css',
})
export class SubjectPersonCardComponent implements OnInit {
  private photos = inject(PhotoService);
  private router = inject(Router);

  readonly match = input.required<SubjectMatch>();

  readonly tagAdded = output<Tag>();
  readonly tagRemoved = output<number>();
  readonly merged = output<void>();

  protected readonly cropUrl = signal<string | null>(null);
  protected readonly name = signal<string | null>(null);
  protected readonly tags = signal<Tag[]>([]);
  protected readonly allTags = signal<TagWithCount[]>([]);
  protected readonly newTagName = signal('');
  protected readonly tagError = signal<string | null>(null);
  protected readonly showNameConflict = signal(false);
  protected readonly conflictingSubjectId = signal<number | null>(null);

  async ngOnInit(): Promise<void> {
    const subject = this.match().subject;
    this.name.set(subject.name);
    this.tags.set(this.match().tags);
    if (!subject.thumbnail_face_id) return;
    try {
      const path = await this.photos.getFaceCrop(subject.thumbnail_face_id);
      this.cropUrl.set(this.photos.thumbnailUrl(path));
    } catch {
      /* fall back to placeholder */
    }
  }

  protected navigate(): void {
    void this.router.navigate(['/subject', this.match().subject.id]);
  }

  protected async saveName(value: string): Promise<void> {
    const id = this.match().subject.id;
    const name = value || null;
    try {
      const result = await this.photos.nameSubject(id, name);
      this.name.set(name);
      if (result.duplicate_subject_id) {
        this.conflictingSubjectId.set(result.duplicate_subject_id);
        this.showNameConflict.set(true);
      }
    } catch (e) {
      console.error('Failed to save name', e);
    }
  }

  protected async confirmMerge(): Promise<void> {
    const id = this.match().subject.id;
    const conflictId = this.conflictingSubjectId();
    if (conflictId === null) return;
    await this.photos.mergeSubjects(id, conflictId);
    this.showNameConflict.set(false);
    this.conflictingSubjectId.set(null);
    this.merged.emit();
  }

  protected cancelMerge(): void {
    this.showNameConflict.set(false);
    this.conflictingSubjectId.set(null);
  }

  protected async onTagFocus(): Promise<void> {
    try {
      this.allTags.set(await this.photos.listTags());
    } catch { /* ignore */ }
  }

  protected async addTag(): Promise<void> {
    const id = this.match().subject.id;
    const name = this.newTagName().trim();
    if (!name) return;
    try {
      this.tagError.set(null);
      const tag = await this.photos.addSubjectTag(id, name);
      this.newTagName.set('');
      this.tags.set(await this.photos.getSubjectTags(id));
      this.tagAdded.emit(tag);
    } catch (e: unknown) {
      this.tagError.set(typeof e === 'string' ? e : 'Failed to add tag');
    }
  }

  protected async removeTag(tagId: number): Promise<void> {
    const id = this.match().subject.id;
    try {
      await this.photos.removeSubjectTag(id, tagId);
      this.tags.update((ts) => ts.filter((t) => t.id !== tagId));
      this.tagRemoved.emit(tagId);
    } catch { /* ignore */ }
  }
}
