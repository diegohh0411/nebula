import { Injectable, OnDestroy } from '@angular/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { Subject } from 'rxjs';
import {
  ProcessingProgressEvent,
  ImageAddedEvent,
  ImageRemovedEvent,
  ImageUpdatedEvent,
  ModelDownloadEvent,
} from '../models/models';

@Injectable({ providedIn: 'root' })
export class TauriEventsService implements OnDestroy {
  readonly processingProgress$ = new Subject<ProcessingProgressEvent>();
  readonly imageAdded$ = new Subject<ImageAddedEvent>();
  readonly imageUpdated$ = new Subject<ImageUpdatedEvent>();
  readonly imageRemoved$ = new Subject<ImageRemovedEvent>();
  readonly modelDownloadProgress$ = new Subject<ModelDownloadEvent>();

  private unlisteners: UnlistenFn[] = [];

  constructor() {
    this.setupListeners();
  }

  private async setupListeners(): Promise<void> {
    this.unlisteners.push(
      await listen<ProcessingProgressEvent>('processing_progress', (e) =>
        this.processingProgress$.next(e.payload)
      ),
      await listen<ImageAddedEvent>('image_added', (e) =>
        this.imageAdded$.next(e.payload)
      ),
      await listen<ImageUpdatedEvent>('image_updated', (e) =>
        this.imageUpdated$.next(e.payload)
      ),
      await listen<ImageRemovedEvent>('image_removed', (e) =>
        this.imageRemoved$.next(e.payload)
      ),
      await listen<ModelDownloadEvent>('model_download_progress', (e) =>
        this.modelDownloadProgress$.next(e.payload)
      )
    );
  }

  ngOnDestroy(): void {
    this.unlisteners.forEach((fn) => fn());
  }
}
