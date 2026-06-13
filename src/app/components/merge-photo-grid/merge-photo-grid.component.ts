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

const PADDING = 0.2;

function focusPercent(bbox: { x: number; y: number; w: number; h: number }): { x: string; y: string } {
  const padW = bbox.w * PADDING;
  const padH = bbox.h * PADDING;
  const x0 = Math.max(0, bbox.x - padW);
  const y0 = Math.max(0, bbox.y - padH);
  const x1 = Math.min(1, bbox.x + bbox.w + padW);
  const y1 = Math.min(1, bbox.y + bbox.h + padH);
  const cx = Math.round(Math.max(0, Math.min(100, (x0 + x1) * 50)) * 10) / 10;
  const cy = Math.round(Math.max(0, Math.min(100, (y0 + y1) * 50)) * 10) / 10;
  return {
    x: `${cx}%`,
    y: `${cy}%`,
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
    return this.photos.thumbnailUrl(img.preview_path ?? img.thumbnail_path);
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
