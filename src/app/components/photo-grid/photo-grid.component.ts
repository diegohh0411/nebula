import {
  Component,
  Input,
  ChangeDetectionStrategy,
  inject,
} from '@angular/core';
import { Image, SearchResult } from '../../models/models';
import { PhotoService } from '../../services/photo.service';
import { startViewTransition } from '../../utils/view-transition';

@Component({
  selector: 'app-photo-grid',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './photo-grid.component.html',
  styleUrl: './photo-grid.component.css',
})
export class PhotoGridComponent {
  @Input() images: (Image | SearchResult)[] = [];
  @Input() rowHeight: number = 220;

  protected photos = inject(PhotoService);
  protected Math = Math;

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
    return this.photos.thumbnailUrl(img.thumbnail_path);
  }

  protected embedStatus(img: Image | SearchResult): 'pending' | 'done' | 'failed' {
    return img.embed_status;
  }

  protected filename(img: Image | SearchResult): string {
    const p = img.path.replace(/\\/g, '/');
    return p.split('/').pop() ?? p;
  }
}
