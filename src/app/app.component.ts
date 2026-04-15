import {
  Component,
  OnInit,
  OnDestroy,
  inject,
  ChangeDetectionStrategy,
} from '@angular/core';
import { MediaMatcher } from '@angular/cdk/layout';
import { SidebarComponent } from './components/sidebar/sidebar.component';
import { GalleryComponent } from './components/gallery/gallery.component';
import { PeopleViewComponent } from './components/people-view/people-view.component';
import { SearchBarComponent } from './components/search-bar/search-bar.component';
import { PhotoService } from './services/photo.service';
import { TauriEventsService } from './services/tauri-events.service';

@Component({
  selector: 'app-root',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SidebarComponent, GalleryComponent, PeopleViewComponent, SearchBarComponent],
  template: `
    <div class="flex h-screen bg-background text-foreground overflow-hidden">
      <app-sidebar class="flex-shrink-0" />
      <div class="flex flex-col flex-1 min-w-0">
        @if (photos.currentView() === 'gallery') {
          <app-search-bar />
          <app-gallery class="flex-1 min-h-0" />
        } @else {
          <app-people-view class="flex-1 min-h-0" />
        }
      </div>
    </div>
  `,
  styles: [
    `
      :host {
        display: block;
        height: 100vh;
      }
    `,
  ],
})
export class AppComponent implements OnInit, OnDestroy {
  private media = inject(MediaMatcher);
  protected photos = inject(PhotoService);
  // Inject events service early so its constructor wires up listeners
  private _events = inject(TauriEventsService);

  private darkQuery = this.media.matchMedia('(prefers-color-scheme: dark)');
  private themeListener = (e: MediaQueryListEvent) => this.applyTheme(e.matches);

  ngOnInit(): void {
    this.applyTheme(this.darkQuery.matches);
    this.darkQuery.addEventListener('change', this.themeListener);

    // Bootstrap data
    void this.photos.loadFolders();
    void this.photos.refreshImages();
    void this.photos.refreshEmbedStatus();
    void this.photos.loadApiKey();
  }

  ngOnDestroy(): void {
    this.darkQuery.removeEventListener('change', this.themeListener);
  }

  private applyTheme(isDark: boolean): void {
    document.documentElement.classList.toggle('dark', isDark);
  }
}
