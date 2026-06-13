import {
  Component,
  ChangeDetectionStrategy,
  inject,
  signal,
  effect,
  computed,
  OnDestroy,
} from '@angular/core';
import { DecimalPipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { LucideAngularModule } from 'lucide-angular';
import { Router } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SubjectMatch, formatEta } from '../../models/models';

type BadgeState = 'active' | 'completing' | 'idle';

@Component({
  selector: 'app-search-bar',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, DecimalPipe, LucideAngularModule],
  templateUrl: './search-bar.component.html',
  styleUrl: './search-bar.component.css',
})
export class SearchBarComponent implements OnDestroy {
  protected photos = inject(PhotoService);
  private router = inject(Router);
  protected query = signal('');
  protected isDragOver = signal(false);
  protected badgeState = signal<BadgeState>('idle');
  protected readonly formatEta = formatEta;
  protected typeaheadMatches = signal<SubjectMatch[]>([]);
  protected typeaheadOpen = signal(false);

  private completingTimer: ReturnType<typeof setTimeout> | null = null;
  private typeaheadTimer: ReturnType<typeof setTimeout> | null = null;
  private typeaheadSeq = 0;

  constructor() {
    effect(() => {
      const stats = this.photos.pipelineStats();
      if (stats.total_pending > 0) {
        if (this.completingTimer !== null) {
          clearTimeout(this.completingTimer);
          this.completingTimer = null;
        }
        this.badgeState.set('active');
      } else if (this.badgeState() === 'active') {
        this.badgeState.set('completing');
        this.completingTimer = setTimeout(() => {
          this.badgeState.set('idle');
          this.completingTimer = null;
        }, 2500);
      }
    });
  }

  ngOnDestroy(): void {
    if (this.completingTimer !== null) {
      clearTimeout(this.completingTimer);
    }
    if (this.typeaheadTimer !== null) {
      clearTimeout(this.typeaheadTimer);
    }
  }

  protected onQueryInput(event: Event): void {
    const q = (event.target as HTMLInputElement).value.trim();
    this.query.set((event.target as HTMLInputElement).value);
    if (this.typeaheadTimer !== null) clearTimeout(this.typeaheadTimer);
    if (q.length < 2) {
      this.typeaheadOpen.set(false);
      this.typeaheadMatches.set([]);
      return;
    }
    this.typeaheadTimer = setTimeout(() => {
      const seq = ++this.typeaheadSeq;
      this.photos.searchSubjects(q).then((m) => {
        if (seq !== this.typeaheadSeq) return;
        this.typeaheadMatches.set(m);
        this.typeaheadOpen.set(m.length > 0);
      }).catch(() => { /* typeahead failures are silent */ });
    }, 200);
  }

  protected onTypeaheadSelect(match: SubjectMatch): void {
    this.typeaheadOpen.set(false);
    void this.router.navigate(['/subject', match.subject.id]);
  }

  protected closeTypeahead(): void {
    this.typeaheadOpen.set(false);
  }

  protected onSearch(): void {
    this.typeaheadOpen.set(false);
    void this.photos.searchByText(this.query());
  }

  protected onClear(): void {
    this.query.set('');
    this.typeaheadOpen.set(false);
    this.typeaheadMatches.set([]);
    this.photos.clearSearch();
  }

  protected onDragOver(event: DragEvent): void {
    event.preventDefault();
    this.isDragOver.set(true);
  }

  protected onDragLeave(event: DragEvent): void {
    this.isDragOver.set(false);
  }

  protected onDrop(event: DragEvent): void {
    event.preventDefault();
    this.isDragOver.set(false);
    const file = event.dataTransfer?.files[0];
    if (!file || !file.type.startsWith('image/')) return;
    const reader = new FileReader();
    reader.onload = () => {
      const base64 = (reader.result as string).split(',')[1];
      const objectUrl = URL.createObjectURL(file);
      void this.photos.searchByExternalImage(base64, file.type, objectUrl);
    };
    reader.readAsDataURL(file);
  }

  protected onPaste(event: ClipboardEvent): void {
    const items = event.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        const file = item.getAsFile();
        if (!file) continue;
        const reader = new FileReader();
        reader.onload = () => {
          const base64 = (reader.result as string).split(',')[1];
          const objectUrl = URL.createObjectURL(file);
          void this.photos.searchByExternalImage(base64, file.type, objectUrl);
        };
        reader.readAsDataURL(file);
        break;
      }
    }
  }

  protected clearImageSearch(): void {
    this.query.set('');
    this.photos.clearSearch();
  }
}
