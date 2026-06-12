import '@analogjs/vitest-angular/setup-zone';
import { setupTestBed } from '@analogjs/vitest-angular/setup-testbed';

// jsdom provides neither IntersectionObserver nor ResizeObserver; several
// components (people-view, gallery, photo-grid, lightbox) construct them in
// lifecycle hooks. Provide inert stubs so component mounts don't throw.
class NoopObserver {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
  takeRecords(): [] { return []; }
}

if (!('IntersectionObserver' in globalThis)) {
  (globalThis as unknown as { IntersectionObserver: unknown }).IntersectionObserver = NoopObserver;
}
if (!('ResizeObserver' in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = NoopObserver;
}

setupTestBed({ zoneless: false });
