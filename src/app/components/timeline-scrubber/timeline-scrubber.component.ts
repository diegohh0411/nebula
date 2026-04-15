import {
  Component,
  EventEmitter,
  Output,
  ChangeDetectionStrategy,
  inject,
  computed,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';

@Component({
  selector: 'app-timeline-scrubber',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule],
  templateUrl: './timeline-scrubber.component.html',
  styleUrl: './timeline-scrubber.component.css',
})
export class TimelineScrubberComponent {
  @Output() dateSelected = new EventEmitter<string>();

  protected photos = inject(PhotoService);

  private isStartOfMonth(group: any): boolean {
    const parts = group.date.split('-');
    if (parts.length !== 3) return false;

    const day = Number(parts[2]);
    return Number.isInteger(day) && day === 1;
  }

  protected markers = computed(() => {
    const groups = this.photos.dayGroups();
    // Only show markers for significant jumps or start of months
    return groups.filter(
      (g, i) => i === 0 || this.isStartOfMonth(g) || i === groups.length - 1
    );
  });

  onScrub(event: MouseEvent) {
    const target = event.currentTarget as HTMLElement;
    const rect = target.getBoundingClientRect();
    const percent = (event.clientY - rect.top) / rect.height;
    
    const groups = this.photos.dayGroups();
    if (groups.length === 0) return;
    
    const idx = Math.floor(percent * groups.length);
    const safeIdx = Math.max(0, Math.min(idx, groups.length - 1));
    this.dateSelected.emit(groups[safeIdx].date);
  }
}
