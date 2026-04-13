import { Component, Input } from '@angular/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { SearchResult } from '../../services/tauri.service';

@Component({
  selector: 'app-image-grid',
  standalone: true,
  imports: [],
  templateUrl: './image-grid.component.html',
  styleUrl: './image-grid.component.css',
})
export class ImageGridComponent {
  @Input() results: SearchResult[] = [];
  @Input() loading = false;
  @Input() query = '';

  getAssetUrl(filePath: string): string {
    return convertFileSrc(filePath);
  }

  getSimilarityLabel(score: number): string {
    return `${Math.round(score * 100)}%`;
  }

  async openImage(filePath: string): Promise<void> {
    try {
      const { openPath } = await import('@tauri-apps/plugin-opener');
      await openPath(filePath);
    } catch (e) {
      console.error('Failed to open image:', e);
    }
  }
}
