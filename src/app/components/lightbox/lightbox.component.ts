import {
  Component,
  Input,
  Output,
  EventEmitter,
  ChangeDetectionStrategy,
  HostListener,
  inject,
  signal,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { LucideAngularModule } from 'lucide-angular';
import { Image, SearchResult } from '../../models/models';
import { PhotoService } from '../../services/photo.service';
import { startViewTransition } from '../../utils/view-transition';

@Component({
  selector: 'app-lightbox',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, LucideAngularModule],
  templateUrl: './lightbox.component.html',
  styleUrl: './lightbox.component.css',
})
export class LightboxComponent {
  @Input() image: Image | SearchResult | null = null;
  
  protected photos = inject(PhotoService);
  protected showSidebar = signal(false);

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
    // Clear after closing transition finishes to minimize tracked elements
    this.photos.transitioningImageId.set(null);
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
    this.close();
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
