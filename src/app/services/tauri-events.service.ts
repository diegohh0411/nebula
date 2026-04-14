import { Injectable, OnDestroy } from '@angular/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { Subject } from 'rxjs';
import {
  EmbedProgressEvent,
  ImageAddedEvent,
  ImageUpdatedEvent,
} from '../models/models';

@Injectable({ providedIn: 'root' })
export class TauriEventsService implements OnDestroy {
  readonly embedProgress$ = new Subject<EmbedProgressEvent>();
  readonly imageAdded$ = new Subject<ImageAddedEvent>();
  readonly imageUpdated$ = new Subject<ImageUpdatedEvent>();

  private unlisteners: UnlistenFn[] = [];

  constructor() {
    this.setupListeners();
  }

  private async setupListeners(): Promise<void> {
    this.unlisteners.push(
      await listen<EmbedProgressEvent>('embed_progress', (e) =>
        this.embedProgress$.next(e.payload)
      ),
      await listen<ImageAddedEvent>('image_added', (e) =>
        this.imageAdded$.next(e.payload)
      ),
      await listen<ImageUpdatedEvent>('image_updated', (e) =>
        this.imageUpdated$.next(e.payload)
      )
    );
  }

  ngOnDestroy(): void {
    this.unlisteners.forEach((fn) => fn());
  }
}
