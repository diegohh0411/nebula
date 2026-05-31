import {
  Component,
  Input,
  ChangeDetectionStrategy,
  inject,
  ElementRef,
  AfterViewInit,
  OnDestroy,
} from '@angular/core';
import { Image, SearchResult, ProcessingStage, getProcessingStage } from '../../models/models';
import { PhotoService } from '../../services/photo.service';
import { startViewTransition } from '../../utils/view-transition';

@Component({
  selector: 'app-photo-grid',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './photo-grid.component.html',
  styleUrl: './photo-grid.component.css',
})
export class PhotoGridComponent implements AfterViewInit, OnDestroy {
  private _images: (Image | SearchResult)[] = [];
  @Input() set images(value: (Image | SearchResult)[]) {
    this._images = value;
    setTimeout(() => this.observeCells(), 0);
  }
  get images() { return this._images; }

  @Input() rowHeight: number = 220;

  protected photos = inject(PhotoService);
  protected Math = Math;

  private host = inject(ElementRef<HTMLElement>);
  private observer?: IntersectionObserver;
  private visible = new Set<number>();
  private flushTimer: ReturnType<typeof setTimeout> | null = null;

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
    const cells = this.host.nativeElement.querySelectorAll('.photo-cell[data-id]');
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

  protected hasScore(img: Image | SearchResult): boolean {
    return 'score' in img && typeof img.score === 'number';
  }

  protected getScore(img: Image | SearchResult): number {
    return 'score' in img ? img.score : 0;
  }

  async onPhotoClick(img: Image | SearchResult) {
    this.photos.transitioningImageId.set(this.imageId(img));
    
    // Brief delay to let Angular apply the view-transition-name to the clicked thumb
    await new Promise(resolve => requestAnimationFrame(resolve));

    await startViewTransition(() => {
      this.photos.openLightbox(img);
    });
  }

  protected imageId(img: Image | SearchResult): number {
    return 'id' in img ? img.id : img.image_id;
  }

  protected thumbUrl(img: Image | SearchResult): string | null {
    return this.photos.thumbnailUrl(img.thumbnail_path ?? img.preview_path);
  }

  protected processingStage(img: Image | SearchResult): ProcessingStage {
    return getProcessingStage(img);
  }

  protected filename(img: Image | SearchResult): string {
    const p = img.path.replace(/\\/g, '/');
    return p.split('/').pop() ?? p;
  }
}
