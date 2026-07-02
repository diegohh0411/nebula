import {
  Component, ChangeDetectionStrategy, inject, input, output, signal, OnInit,
} from '@angular/core';
import { Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch } from '../../models/models';

@Component({
  selector: 'app-subject-person-card',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './subject-person-card.component.html',
  styleUrl: './subject-person-card.component.css',
})
export class SubjectPersonCardComponent implements OnInit {
  private photos = inject(PhotoService);
  private router = inject(Router);

  readonly match = input.required<SubjectMatch>();
  readonly removable = input(false);
  readonly remove = output<number>();
  readonly subtitle = input<string>();

  protected readonly cropUrl = signal<string | null>(null);

  async ngOnInit(): Promise<void> {
    const subject = this.match().subject;
    if (!subject.thumbnail_face_id) return;
    try {
      const path = await this.photos.getFaceCrop(subject.thumbnail_face_id);
      this.cropUrl.set(this.photos.thumbnailUrl(path));
    } catch {
      /* fall back to placeholder */
    }
  }

  protected get displayName(): string {
    return this.match().subject.name ?? 'Unnamed';
  }

  protected navigate(): void {
    void this.router.navigate(['/subject', this.match().subject.id]);
  }

  protected onRemove(event: Event): void {
    event.stopPropagation();
    this.remove.emit(this.match().subject.id);
  }
}
