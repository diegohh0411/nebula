import {
  Component,
  Input,
  ChangeDetectionStrategy,
  HostListener,
  inject,
  signal,
  OnChanges,
  OnDestroy,
  AfterViewInit,
  ElementRef,
  ViewChild,
  computed,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { Router } from '@angular/router';
import { LucideAngularModule } from 'lucide-angular';
import { Image, SearchResult, Face } from '../../models/models';
import { PhotoService } from '../../services/photo.service';
import { startViewTransition } from '../../utils/view-transition';

interface FaceOverlayStyle {
  left: number;
  top: number;
  width: number;
  height: number;
}

@Component({
  selector: 'app-lightbox',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, LucideAngularModule],
  templateUrl: './lightbox.component.html',
  styleUrl: './lightbox.component.css',
})
export class LightboxComponent implements OnChanges, AfterViewInit, OnDestroy {
  @Input() image: Image | SearchResult | null = null;
  @ViewChild('mainImg') imgRef?: ElementRef<HTMLImageElement>;
  @ViewChild('imgContainer') containerRef?: ElementRef<HTMLDivElement>;

  protected photos = inject(PhotoService);
  protected router = inject(Router);
  protected showSidebar = signal(false);

  faces = signal<Face[]>([]);
  activeFaceId = signal<number | null>(null);

  private naturalW = 0;
  private naturalH = 0;
  private containerW = 0;
  private containerH = 0;
  private resizeObserver?: ResizeObserver;

  private imgLayout = signal<{ offsetX: number; offsetY: number; renderedW: number; renderedH: number; containerW: number; containerH: number } | null>(null);

  protected faceOverlayStyles = computed(() => {
    const layout = this.imgLayout();
    if (!layout) return new Map<number, FaceOverlayStyle>();

    const { offsetX, offsetY, renderedW, renderedH, containerW, containerH } = layout;
    const styles = new Map<number, FaceOverlayStyle>();
    for (const face of this.faces()) {
      styles.set(face.id, {
        left: ((offsetX + face.bbox_x * renderedW) / containerW) * 100,
        top: ((offsetY + face.bbox_y * renderedH) / containerH) * 100,
        width: (face.bbox_w * renderedW / containerW) * 100,
        height: (face.bbox_h * renderedH / containerH) * 100,
      });
    }
    return styles;
  });

  ngOnChanges() {
    if (this.image) {
      const id = 'id' in this.image ? this.image.id : this.image.image_id;
      this.photos.loadFacesForImage(id).then(f => this.faces.set(f));
    } else {
      this.faces.set([]);
    }
    this.recalcLayout();
  }

  ngAfterViewInit() {
    const container = this.containerRef?.nativeElement;
    if (container) {
      this.containerW = container.clientWidth;
      this.containerH = container.clientHeight;
    }
    this.initResizeObserver();
    this.recalcLayout();
  }

  ngOnDestroy() {
    this.resizeObserver?.disconnect();
  }

  protected onImageLoad() {
    const img = this.imgRef?.nativeElement;
    if (img) {
      this.naturalW = img.naturalWidth;
      this.naturalH = img.naturalHeight;
      this.recalcLayout();
    }
  }

  protected initResizeObserver() {
    const container = this.containerRef?.nativeElement;
    if (!container) return;
    this.resizeObserver?.disconnect();
    this.resizeObserver = new ResizeObserver(() => {
      this.containerW = container.clientWidth;
      this.containerH = container.clientHeight;
      this.recalcLayout();
    });
    this.resizeObserver.observe(container);
  }

  private recalcLayout() {
    if (!this.naturalW || !this.naturalH || !this.containerW || !this.containerH) {
      this.imgLayout.set(null);
      return;
    }
    const scaleX = this.containerW / this.naturalW;
    const scaleY = this.containerH / this.naturalH;
    const scale = Math.min(scaleX, scaleY);
    const renderedW = this.naturalW * scale;
    const renderedH = this.naturalH * scale;
    const offsetX = (this.containerW - renderedW) / 2;
    const offsetY = (this.containerH - renderedH) / 2;
    this.imgLayout.set({ offsetX, offsetY, renderedW, renderedH, containerW: this.containerW, containerH: this.containerH });
  }

  getSubjectName(subjectId: number | null): string {
    if (!subjectId) return 'Unnamed Subject';
    const sub = this.photos.subjects().find(s => s.id === subjectId);
    return sub?.name || 'Unnamed Subject';
  }

  setActiveFace(id: number | null) {
    this.activeFaceId.set(id);
  }

  @HostListener('window:keydown', ['$event'])
  handleKeyDown(event: KeyboardEvent) {
    if (event.key === 'Escape') this.close();
    if (event.key === 'ArrowRight') this.next();
    if (event.key === 'ArrowLeft') this.prev();
  }

  async close() {
    await startViewTransition(() => {
      this.photos.closeLightbox();
    });
    this.photos.transitioningImageId.set(null);
  }

  async navigateToSubject(subjectId: number | null) {
    if (!subjectId) return;
    await this.close();
    void this.router.navigate(['/subject', subjectId]);
  }

  async next() {
    await startViewTransition(() => {
      this.photos.navigateLightbox(1);
    });
  }

  async prev() {
    await startViewTransition(() => {
      this.photos.navigateLightbox(-1);
    });
  }

  toggleSidebar() {
    this.showSidebar.update(s => !s);
  }

  async findSimilar() {
    if (!this.image) return;
    await this.photos.searchByImage(this.image);
    await this.close();
  }

  protected thumbUrl(img: Image | SearchResult): string | null {
    return this.photos.thumbnailUrl(img.thumbnail_path);
  }

  protected originalUrl(img: Image | SearchResult): string {
    return this.photos.originalUrl(img.path);
  }

  protected filename(img: Image | SearchResult): string {
    const p = img.path.replace(/\\/g, '/');
    return p.split('/').pop() ?? p;
  }
}
