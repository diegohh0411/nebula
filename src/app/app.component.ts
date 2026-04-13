import { Component, OnInit } from '@angular/core';
import { FolderManagerComponent } from './components/folder-manager/folder-manager.component';
import { EmbeddingStatusComponent } from './components/embedding-status/embedding-status.component';
import { SearchBarComponent } from './components/search-bar/search-bar.component';
import { ImageGridComponent } from './components/image-grid/image-grid.component';
import { TauriService, SearchResult } from './services/tauri.service';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [
    FolderManagerComponent,
    EmbeddingStatusComponent,
    SearchBarComponent,
    ImageGridComponent,
  ],
  templateUrl: './app.component.html',
  styleUrl: './app.component.css',
})
export class AppComponent implements OnInit {
  searchResults: SearchResult[] = [];
  searchLoading = false;
  currentQuery = '';

  constructor(private tauri: TauriService) {}

  ngOnInit(): void {
    this.tauri.startSidecar().catch(() => {});
  }

  async onSearch(query: string): Promise<void> {
    if (!query) {
      this.searchResults = [];
      this.currentQuery = '';
      return;
    }
    this.currentQuery = query;
    this.searchLoading = true;
    try {
      this.searchResults = await this.tauri.searchImages(query, 50);
    } catch (e) {
      console.error('Search failed:', e);
      this.searchResults = [];
    } finally {
      this.searchLoading = false;
    }
  }
}
