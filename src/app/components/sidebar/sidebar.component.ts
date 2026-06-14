import {
  Component,
  ChangeDetectionStrategy,
  inject,
} from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { open } from '@tauri-apps/plugin-dialog';
import { LucideAngularModule } from 'lucide-angular';
import { PhotoService } from '../../services/photo.service';
import { SidebarItemComponent } from '../ui/sidebar-item/sidebar-item.component';

@Component({
  selector: 'app-sidebar',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [RouterLink, LucideAngularModule, SidebarItemComponent],
  templateUrl: './sidebar.component.html',
  styleUrl: './sidebar.component.css',
})
export class SidebarComponent {
  protected photos = inject(PhotoService);
  protected router = inject(Router);

  protected folderBasename(path: string): string {
    return path.replace(/\\/g, '/').split('/').filter(Boolean).pop() ?? path;
  }

  protected async addFolder(): Promise<void> {
    const selected = await open({ directory: true, multiple: false });
    if (selected && typeof selected === 'string') {
      await this.photos.addFolder(selected);
    }
  }

  protected async removeFolder(id: number, event: MouseEvent): Promise<void> {
    event.stopPropagation();
    await this.photos.removeFolder(id);
  }

  protected selectFolder(id: number | null): void {
    if (this.router.url !== '/') {
      void this.router.navigate(['/']);
    }
    this.photos.selectFolder(id);
  }

  protected isGalleryActive(): boolean {
    return this.router.url === '/' || this.router.url === '';
  }

  protected isPeopleActive(): boolean {
    return this.router.url === '/people';
  }

  protected isTagsActive(): boolean {
    return this.router.url === '/tags';
  }

  protected isSettingsActive(): boolean {
    return this.router.url === '/settings';
  }
}
