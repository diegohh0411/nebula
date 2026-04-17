import { Injectable, inject, signal, computed } from '@angular/core';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import {
  DayGroup,
  EmbedStatus,
  Folder,
  Image,
  SearchResult,
  VirtualRow,
  Subject,
  Face,
  MergeSuggestion,
  NameSubjectResult,
  SubjectDetail,
} from '../models/models';
import { TauriEventsService } from './tauri-events.service';
import { buildJustifiedRows } from '../utils/justified-layout';

@Injectable({ providedIn: 'root' })
export class PhotoService {
  private events = inject(TauriEventsService);

  // ---- Reactive state (Angular signals) ----
  readonly viewportWidth = signal<number>(1000);
  readonly targetRowHeight = signal<number>(220);
  readonly folders = signal<Folder[]>([]);
  readonly subjects = signal<Subject[]>([]);
  readonly images = signal<Image[]>([]);
  readonly searchResults = signal<SearchResult[] | null>(null); // null = not in search mode
  readonly embedStatus = signal<EmbedStatus>({ pending: 0, done: 0 });
  readonly selectedFolderId = signal<number | null>(null);
  readonly isSearching = signal(false);
  readonly searchError = signal<string | null>(null);
  readonly searchImage = signal<{ thumbnailUrl: string; type: 'library' | 'external' } | null>(null);
  readonly searchText = signal<string>('');
  readonly apiKey = signal<string | null>(null);
  readonly showApiKeyInput = signal(false);

  // ---- Lightbox state ----
  readonly selectedImage = signal<Image | SearchResult | null>(null);
  readonly transitioningImageId = signal<number | null>(null);
  readonly selectedImageIds = signal<Set<number>>(new Set());

  toggleSelection(id: number): void {
    const next = new Set(this.selectedImageIds());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    this.selectedImageIds.set(next);
  }

  setSelection(ids: number[]): void {
    this.selectedImageIds.set(new Set(ids));
  }

  clearSelection(): void {
    this.selectedImageIds.set(new Set());
  }

  openLightbox(img: Image | SearchResult): void {
    this.transitioningImageId.set('id' in img ? img.id : img.image_id);
    this.selectedImage.set(img);
  }

  closeLightbox(): void {
    this.selectedImage.set(null);
    // Note: transitioningImageId stays set during the transition back, then cleared in the component.
  }

  navigateLightbox(direction: number): void {
    const current = this.selectedImage();
    if (!current) return;

    // Use search results if available, otherwise full gallery
    const allImages: (Image | SearchResult)[] = this.searchResults() ?? this.images();
    const currentId = 'id' in current ? current.id : current.image_id;
    const idx = allImages.findIndex((i) => ('id' in i ? i.id : i.image_id) === currentId);
    if (idx === -1) return;

    const nextIdx = (idx + direction + allImages.length) % allImages.length;
    const nextImg = allImages[nextIdx];
    this.selectedImage.set(nextImg);
    this.transitioningImageId.set('id' in nextImg ? nextImg.id : nextImg.image_id);
  }

  /** Day-grouped images for the gallery. Uses search results when available. */
  readonly dayGroups = computed<DayGroup[]>(() => {
    const results = this.searchResults();
    if (results) {
      // Use a single group for search results to show them sorted by similarity
      return [
        {
          label: 'Search Results',
          date: 'search',
          images: results,
        },
      ];
    }
    return groupByDay(this.images());
  });

  /** Flat virtual scroll rows: interleaved headers + justified rows */
  readonly virtualRows = computed<VirtualRow[]>(() => {
    return flattenToVirtualRowsJustified(
      this.dayGroups(),
      this.viewportWidth(),
      this.targetRowHeight()
    );
  });

  /** Total photo count across all folders, independent of selection. */
  readonly totalPhotoCount = computed<number>(() =>
    this.folders().reduce((sum, f) => sum + f.photo_count, 0)
  );

  constructor() {
    this.events.embedProgress$.subscribe((e) => {
      this.embedStatus.set(e);
    });

    this.events.imageAdded$.subscribe(() => {
      void this.refreshImages();
    });

    this.events.imageUpdated$.subscribe(() => {
      void this.refreshImages();
    });

    this.events.imageRemoved$.subscribe(() => {
      void this.refreshImages();
      void this.loadFolders();
    });
  }

  // ---- Commands ----

  async loadFolders(): Promise<void> {
    const folders = await invoke<Folder[]>('list_folders');
    this.folders.set(folders);
  }

  async loadSubjects(): Promise<void> {
    const subjects = await invoke<Subject[]>('list_subjects');
    this.subjects.set(subjects);
  }

  async nameSubject(id: number, name: string | null): Promise<NameSubjectResult> {
    const result = await invoke<NameSubjectResult>('name_subject', { id, name });
    await this.loadSubjects();
    return result;
  }

  async loadFaces(subjectId: number): Promise<Face[]> {
    return await invoke<Face[]>('list_faces', { subjectId });
  }

  async loadFacesForImage(imageId: number): Promise<Face[]> {
    return await invoke<Face[]>('list_faces_for_image', { imageId });
  }

  async getSubjectDetail(subjectId: number): Promise<SubjectDetail> {
    return await invoke<SubjectDetail>('get_subject_detail', { subjectId });
  }

  async getSubjectPhotos(subjectId: number): Promise<SearchResult[]> {
    return await invoke<SearchResult[]>('get_subject_photos', { subjectId });
  }

  async setSubjectThumbnail(subjectId: number, faceId: number): Promise<void> {
    await invoke('set_subject_thumbnail', { subjectId, faceId });
  }

  async getFaceCrop(faceId: number): Promise<string> {
    return await invoke<string>('get_face_crop', { faceId });
  }

  async addFolder(path: string): Promise<void> {
    await invoke<Folder>('add_folder', { path });
    await this.loadFolders();
    await this.refreshImages();
  }

  async removeFolder(id: number): Promise<void> {
    await invoke('remove_folder', { id });
    if (this.selectedFolderId() === id) {
      this.selectedFolderId.set(null);
    }
    await this.loadFolders();
    await this.refreshImages();
  }

  async refreshImages(): Promise<void> {
    const folderId = this.selectedFolderId();
    const imgs = await invoke<Image[]>('list_images', {
      folderId: folderId ?? null,
    });
    this.images.set(imgs);
  }

  async refreshEmbedStatus(): Promise<void> {
    const status = await invoke<EmbedStatus>('get_embed_status');
    this.embedStatus.set(status);
  }

  async searchByText(query: string): Promise<void> {
    if (!query.trim()) {
      this.clearSearch();
      return;
    }
    this.revokeExternalImage();
    this.searchText.set(query);
    this.searchImage.set(null);
    this.isSearching.set(true);
    this.searchError.set(null);
    try {
      const results = await invoke<SearchResult[]>('search', { query: { type: 'text', query } });
      this.searchResults.set(results);
    } catch (e: unknown) {
      const msg =
        typeof e === 'string' && e.includes('connection')
          ? e
          : 'Search requires a connection — try again when online.';
      this.searchError.set(msg);
      this.searchResults.set(null);
    } finally {
      this.isSearching.set(false);
    }
  }

  async searchByImage(image: Image | SearchResult): Promise<void> {
    const id = 'id' in image ? image.id : image.image_id;
    const thumbUrl = this.thumbnailUrl(image.thumbnail_path);
    this.revokeExternalImage();
    this.searchImage.set(thumbUrl ? { thumbnailUrl: thumbUrl, type: 'library' } : null);
    this.searchText.set('');
    this.isSearching.set(true);
    this.searchError.set(null);
    try {
      const results = await invoke<SearchResult[]>('search', { query: { type: 'imageId', image_id: id } });
      this.searchResults.set(results);
    } catch (e: unknown) {
      this.searchError.set(typeof e === 'string' ? e : 'Visual search failed.');
      this.searchResults.set(null);
    } finally {
      this.isSearching.set(false);
    }
  }

  async searchByExternalImage(base64Data: string, mimeType: string, objectUrl: string): Promise<void> {
    this.revokeExternalImage();
    this.searchImage.set({ thumbnailUrl: objectUrl, type: 'external' });
    this.searchText.set('');
    this.isSearching.set(true);
    this.searchError.set(null);
    try {
      const results = await invoke<SearchResult[]>('search', { query: { type: 'imageBytes', data: base64Data, mime_type: mimeType } });
      this.searchResults.set(results);
    } catch (e: unknown) {
      this.searchError.set(typeof e === 'string' ? e : 'Image search failed.');
      this.searchResults.set(null);
    } finally {
      this.isSearching.set(false);
    }
  }

  clearSearch(): void {
    this.revokeExternalImage();
    this.searchResults.set(null);
    this.searchError.set(null);
    this.searchImage.set(null);
    this.searchText.set('');
  }

  selectFolder(id: number | null): void {
    this.selectedFolderId.set(id);
    this.clearSearch();
    void this.refreshImages();
  }

  async loadApiKey(): Promise<void> {
    const key = await invoke<string | null>('get_api_key');
    this.apiKey.set(key);
  }

  async saveApiKey(key: string): Promise<void> {
    await invoke('set_api_key', { key });
    this.apiKey.set(key);
  }

  async regenerateThumbnails(): Promise<void> {
    await invoke('regenerate_all_thumbnails');
    // The image_updated events from Rust will trigger refreshing the grid automatically
  }

  async reclusterFaces(): Promise<{ clusters: number; noise: number; merged: number; deleted: number }> {
    return await invoke<{ clusters: number; noise: number; merged: number; deleted: number }>('recluster_faces');
  }

  async getMergeSuggestions(): Promise<MergeSuggestion[]> {
    return await invoke<MergeSuggestion[]>('get_merge_suggestions');
  }

  async mergeSubjects(targetId: number, sourceId: number): Promise<void> {
    await invoke('merge_subjects', { targetId, sourceId });
    await this.loadSubjects();
  }

  async dismissMergeSuggestion(id: number): Promise<void> {
    await invoke('dismiss_merge_suggestion', { id });
  }

  async assignFaceToSubject(faceId: number, subjectId: number): Promise<void> {
    await invoke('assign_face_to_subject', { faceId, subjectId });
  }

  async createSubjectForFace(faceId: number, name?: string): Promise<Subject> {
    const subject = await invoke<Subject>('create_subject_for_face', {
      faceId,
      name: name ?? null,
    });
    this.subjects.update(subjects => [...subjects, subject]);
    return subject;
  }

  private revokeExternalImage(): void {
    const img = this.searchImage();
    if (img?.type === 'external' && img.thumbnailUrl) {
      URL.revokeObjectURL(img.thumbnailUrl);
    }
  }

  /** Convert an absolute path to a Tauri asset URL for use in <img src>. */
  thumbnailUrl(thumbPath: string | null): string | null {
    if (!thumbPath) return null;
    return convertFileSrc(thumbPath);
  }

  /** Convert an absolute path to the original full-res image to a Tauri asset URL. */
  originalUrl(imagePath: string): string {
    return convertFileSrc(imagePath);
  }
}

// ---- Utility functions ----

function getTimestamp(img: Image | SearchResult): number {
  return img.date_taken ?? img.date_file;
}

function groupByDay(images: (Image | SearchResult)[]): DayGroup[] {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);

  const map = new Map<string, DayGroup>();

  for (const img of images) {
    const ts = getTimestamp(img);
    const d = new Date(ts * 1000);
    d.setHours(0, 0, 0, 0);
    const key = d.toISOString().slice(0, 10);

    if (!map.has(key)) {
      let label: string;
      if (d.getTime() === today.getTime()) {
        label = 'Today';
      } else if (d.getTime() === yesterday.getTime()) {
        label = 'Yesterday';
      } else {
        label = d.toLocaleDateString('en-US', {
          year: 'numeric',
          month: 'long',
          day: 'numeric',
        });
      }
      map.set(key, { label, date: key, images: [] });
    }
    map.get(key)!.images.push(img);
  }

  return Array.from(map.values()).sort((a, b) => b.date.localeCompare(a.date));
}

/**
 * Flatten day groups into a flat array of virtual rows using justified layout.
 */
function flattenToVirtualRowsJustified(
  groups: DayGroup[],
  containerWidth: number,
  targetHeight: number
): VirtualRow[] {
  const rows: VirtualRow[] = [];
  for (const group of groups) {
    rows.push({ type: 'header', label: group.label, date: group.date });
    const justifiedRows = buildJustifiedRows(group.images, containerWidth, targetHeight, 4);
    for (const row of justifiedRows) {
      rows.push({ type: 'row', images: row.images, rowHeight: row.rowHeight });
    }
  }
  return rows;
}
