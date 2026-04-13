import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface Folder {
  id: number;
  path: string;
  added_at: string;
}

export interface ImageRecord {
  id: number;
  folder_id: number;
  file_path: string;
  file_name: string;
  file_size: number | null;
  created_at: string | null;
  indexed_at: string;
  embedded: boolean;
}

export interface IndexingStatus {
  total: number;
  embedded: number;
}

export interface SearchResult {
  id: number;
  file_path: string;
  file_name: string;
  similarity: number;
}

export interface EmbeddingProgress {
  current: number;
  total: number;
}

@Injectable({ providedIn: 'root' })
export class TauriService {
  addFolder(path: string): Promise<Folder> {
    return invoke<Folder>('add_folder', { path });
  }

  removeFolder(id: number): Promise<void> {
    return invoke<void>('remove_folder', { id });
  }

  listFolders(): Promise<Folder[]> {
    return invoke<Folder[]>('list_folders');
  }

  getIndexingStatus(): Promise<IndexingStatus> {
    return invoke<IndexingStatus>('get_indexing_status');
  }

  getImages(offset: number, limit: number): Promise<ImageRecord[]> {
    return invoke<ImageRecord[]>('get_images', { offset, limit });
  }

  startSidecar(): Promise<void> {
    return invoke<void>('start_sidecar');
  }

  stopSidecar(): Promise<void> {
    return invoke<void>('stop_sidecar');
  }

  sidecarHealth(): Promise<boolean> {
    return invoke<boolean>('sidecar_health');
  }

  startEmbeddingJob(): Promise<void> {
    return invoke<void>('start_embedding_job');
  }

  searchImages(query: string, limit: number = 20): Promise<SearchResult[]> {
    return invoke<SearchResult[]>('search_images', { query, limit });
  }

  onEmbeddingProgress(
    callback: (progress: EmbeddingProgress) => void,
  ): Promise<UnlistenFn> {
    return listen<EmbeddingProgress>('embedding-progress', (event) =>
      callback(event.payload),
    );
  }

  onEmbeddingComplete(callback: () => void): Promise<UnlistenFn> {
    return listen('embedding-complete', () => callback());
  }
}
