import {
  Component,
  Input,
  ChangeDetectionStrategy,
  inject,
} from '@angular/core';
import { Image, SearchResult } from '../../models/models';
import { PhotoService } from '../../services/photo.service';

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
