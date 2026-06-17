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
  signal,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { LucideAngularModule } from 'lucide-angular';
import { SubjectPhotoFace, SearchResult } from '../../models/models';
import { PhotoService } from '../../services/photo.service';

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

  /** Resolved face-crop image URLs, keyed by face id. */
  protected cropUrls = signal<Map<number, string>>(new Map());

  @Input() images: SubjectPhotoFace[] = [];

  ngOnChanges(changes: SimpleChanges): void {
    if (changes['images'] && !changes['images'].firstChange) {
      this.cropUrls.set(new Map());
      queueMicrotask(() => this.observeCells());
    }
  }

  ngAfterViewInit(): void {
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const faceId = Number((e.target as HTMLElement).dataset['id']);
          if (Number.isNaN(faceId)) continue;
          void this.loadCrop(faceId);
          this.observer?.unobserve(e.target);
        }
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

  /** Fetch (and cache) the real face crop for a face, then expose its URL to the template. */
  private async loadCrop(faceId: number): Promise<void> {
    if (this.cropUrls().has(faceId)) return;
    try {
      const path = await this.photos.getFaceCrop(faceId);
      const url = this.photos.thumbnailUrl(path);
      if (!url) return;
      this.cropUrls.update((urls) => {
        const next = new Map(urls);
        next.set(faceId, url);
        return next;
      });
    } catch (e) {
      console.error(`MergePhotoGrid: failed to load face crop ${faceId}`, e);
    }
  }

  protected cropUrl(img: SubjectPhotoFace): string | null {
    return this.cropUrls().get(img.face_id) ?? null;
  }

  ngOnDestroy(): void {
    this.observer?.disconnect();
  }

  protected onClick(img: SubjectPhotoFace): void {
    const list = this.images.map((i) => this.toLightboxImage(i));
    const clicked = list.find((i) => i.image_id === img.image_id) ?? this.toLightboxImage(img);
    this.photos.openLightbox(clicked, list);
  }

  private toLightboxImage(img: SubjectPhotoFace): SearchResult {
    return {
      image_id: img.image_id,
      path: img.path,
      thumbnail_path: img.thumbnail_path,
      preview_path: img.preview_path,
      score: 0,
      date_taken: img.date_taken,
      mtime: img.mtime,
      semantic_analysis_done: true,
      subject_analysis_done: true,
    };
  }
}
