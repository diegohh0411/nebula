import {
  Component,
  Input,
  ChangeDetectionStrategy,
  inject,
  ElementRef,
  AfterViewInit,
  OnDestroy,
  OnChanges,
  SimpleChanges,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { LucideAngularModule } from 'lucide-angular';
import { SubjectPhotoFace } from '../../models/models';
import { PhotoService } from '../../services/photo.service';

function focusPercent(bbox: { x: number; y: number; w: number; h: number }): { x: string; y: string } {
  const cx = bbox.x + bbox.w / 2;
  const cy = bbox.y + bbox.h / 2;
  return {
    x: `${Math.max(0, Math.min(100, cx * 100))}%`,
    y: `${Math.max(0, Math.min(100, cy * 100))}%`,
  };
}

@Component({
  selector: 'app-merge-photo-grid',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, LucideAngularModule],
  templateUrl: './merge-photo-grid.component.html',
  styleUrl: './merge-photo-grid.component.css',
})
export class MergePhotoGridComponent implements AfterViewInit, OnDestroy, OnChanges {
  private photos = inject(PhotoService);
  private host = inject(ElementRef<HTMLElement>);

  private observer?: IntersectionObserver;
  private visible = new Set<number>();
  private flushTimer: ReturnType<typeof setTimeout> | null = null;

  @Input() images: SubjectPhotoFace[] = [];

  ngOnChanges(changes: SimpleChanges): void {
    if (changes['images'] && !changes['images'].firstChange) {
      this.visible.clear();
      this.observer?.disconnect();
      queueMicrotask(() => this.observeCells());
    }
  }

  ngAfterViewInit(): void {
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const id = Number((e.target as HTMLElement).dataset['id']);
          if (Number.isNaN(id)) continue;
          if (e.isIntersecting) this.visible.add(id);
          else this.visible.delete(id);
        }
        this.scheduleFlush();
      },
      { root: null, rootMargin: '400px', threshold: 0.01 }
    );
    this.observeCells();
  }

  private observeCells(): void {
    if (!this.observer) return;
    this.observer.disconnect();
    const cells = this.host.nativeElement.querySelectorAll('.merge-photo-cell[data-id]');
    cells.forEach((el: Element) => this.observer!.observe(el));
  }

  private scheduleFlush(): void {
    if (this.flushTimer) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      if (this.visible.size > 0) {
        this.photos.prioritizePreviews([...this.visible]);
      }
    }, 100);
  }

  ngOnDestroy(): void {
    this.observer?.disconnect();
    if (this.flushTimer) clearTimeout(this.flushTimer);
  }

  protected thumbUrl(img: SubjectPhotoFace): string | null {
    return this.photos.thumbnailUrl(img.thumbnail_path ?? img.preview_path);
  }

  protected focus(img: SubjectPhotoFace): { x: string; y: string } {
    return focusPercent(img.face_bbox);
  }

  protected onClick(img: SubjectPhotoFace): void {
    this.photos.openLightbox({
      image_id: img.image_id,
      path: img.path,
      thumbnail_path: img.thumbnail_path,
      preview_path: img.preview_path,
      score: 0,
      date_taken: img.date_taken,
      mtime: img.mtime,
      semantic_analysis_done: true,
      subject_analysis_done: true,
    });
  }
}
