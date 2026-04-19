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
import { Image, SearchResult, Face, Subject, getProcessingStage, ProcessingStage } from '../../models/models';
import { PhotoService } from '../../services/photo.service';
import { FaceAssignPopoverComponent } from '../face-assign-popover/face-assign-popover.component';
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
  imports: [CommonModule, LucideAngularModule, FaceAssignPopoverComponent],
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

  showFacesToAdd = signal(false);

  protected assignedFaces = computed(() => this.faces().filter(f => f.subject_id !== null));
  protected unassignedFaces = computed(() => this.faces().filter(f => f.subject_id === null));

  protected localSubjects = signal<Subject[]>([]);

  private resizeObserver?: ResizeObserver;

  private imgLayout = signal<{ offsetX: number; offsetY: number; renderedW: number; renderedH: number; containerW: number; containerH: number } | null>(null);

  protected get stage(): ProcessingStage {
    return this.image ? getProcessingStage(this.image) : 'ready';
  }

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
      if (this.photos.subjects().length === 0) {
        void this.photos.loadSubjects();
      }
      this.localSubjects.set(this.photos.subjects());
      this.showFacesToAdd.set(false);
    } else {
      this.faces.set([]);
      this.localSubjects.set([]);
    }
    this.imgLayout.set(null);
  }

  ngAfterViewInit() {
    this.tryInitResizeObserver();
  }

  ngOnDestroy() {
    this.resizeObserver?.disconnect();
  }

  protected onImageLoad() {
    this.updateLayoutFromDOM();
    this.tryInitResizeObserver();
  }

  private tryInitResizeObserver() {
    const container = this.containerRef?.nativeElement;
    if (!container || this.resizeObserver) return;
    this.resizeObserver = new ResizeObserver(() => this.updateLayoutFromDOM());
    this.resizeObserver.observe(container);
  }

  private updateLayoutFromDOM() {
    const img = this.imgRef?.nativeElement;
    const container = this.containerRef?.nativeElement;
    if (!img || !container) {
      this.imgLayout.set(null);
      return;
    }
    const imgRect = img.getBoundingClientRect();
    const containerRect = container.getBoundingClientRect();
    this.imgLayout.set({
      offsetX: imgRect.left - containerRect.left,
      offsetY: imgRect.top - containerRect.top,
      renderedW: imgRect.width,
      renderedH: imgRect.height,
      containerW: containerRect.width,
      containerH: containerRect.height,
    });
  }

  onFaceAssigned(event: { face: Face; subject: Subject }) {
    this.faces.update(faces =>
      faces.map(f => f.id === event.face.id ? { ...f, subject_id: event.subject.id } : f)
    );
    this.localSubjects.update(subjects => {
      if (subjects.find(s => s.id === event.subject.id)) return subjects;
      return [...subjects, event.subject];
    });
    if (this.unassignedFaces().length === 0) {
      this.showFacesToAdd.set(false);
    }
  }

  onFaceRemoved(event: { face: Face }) {
    this.faces.update(faces =>
      faces.map(f => f.id === event.face.id ? { ...f, subject_id: null } : f)
    );
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
