import {
  Component, ChangeDetectionStrategy, computed, inject, input, output, signal, OnInit,
} from '@angular/core';
import { Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch, Tag } from '../../models/models';
import { EditableTextComponent } from '../editable-text/editable-text.component';
import { ConfirmMergeDialogComponent } from '../confirm-merge-dialog/confirm-merge-dialog.component';
import { injectSubjectTagging } from '../../composables/subject-tagging.composable';
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

  private readonly subjectId = computed(() => this.match().subject.id);
  protected readonly tagging = injectSubjectTagging(this.subjectId, {
    onMerged: () => this.merged.emit(),
    onTagAdded: (t) => this.tagAdded.emit(t),
    onTagRemoved: (id) => this.tagRemoved.emit(id),
  });

  async ngOnInit(): Promise<void> {
    const subject = this.match().subject;
    this.tagging.name.set(subject.name);
    this.tagging.tags.set(this.match().tags);
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
}
