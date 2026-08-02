import { TestBed } from '@angular/core/testing';
import { importProvidersFrom, signal } from '@angular/core';
import { provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { LucideAngularModule } from 'lucide-angular';
import { APP_ICONS } from '../../app-icons';
import { PhotoService } from '../../services/photo.service';
import { ReportDetailComponent } from './report-detail.component';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue([]),
  convertFileSrc: vi.fn((p: string) => p),
}));

describe('ReportDetailComponent — subject cards', () => {
  class ReportPhotoServiceStub {
    getSavedReport = vi.fn().mockResolvedValue({
      id: 1,
      name: 'Camp Roster',
      folder_ids: [1],
      tag_ids: [1],
    });
    loadSubjects = vi.fn().mockResolvedValue(undefined);
    subjects = () => [
      { id: 10, name: 'Maria', thumbnail_face_id: null, type: 'person', added_at: 0 },
    ];
    folders = () => [{ id: 1, path: '/photos/camp', added_at: 0, photo_count: 3 }];
    getFolderCoverage = vi.fn().mockResolvedValue({
      summary: { total_targets: 1, present_targets: 1 },
      missing_targets: [],
      present_targets: [{ subject_id: 10, name: 'Maria', frequency: 2 }],
      others_found: [],
    });
    getSubjectTags = vi.fn().mockResolvedValue([{ id: 1, name: 'Cabaña-21', added_at: 0 }]);
    listTags = vi.fn().mockResolvedValue([]);
    pipelineStats = signal({ total_pending: 5, images_per_sec: 1 });
    getReportProcessingProgress = vi.fn().mockResolvedValue({ total: 4, done: 2 });
  }

  let stub: ReportPhotoServiceStub;

  beforeEach(() => {
    stub = new ReportPhotoServiceStub();
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'reports/:id', component: ReportDetailComponent }]),
        importProvidersFrom(LucideAngularModule.pick(APP_ICONS)),
        { provide: PhotoService, useValue: stub },
      ],
    });
  });

  it("renders each subject's tags on its card without needing a click-through", async () => {
    const harness = await RouterTestingHarness.create('/reports/1');
    // Component init chains several awaits (report → coverage → per-subject
    // tags) before the cards even mount and seed their chips; settle until
    // the async chain has fully flushed.
    for (let i = 0; i < 5; i++) {
      harness.detectChanges();
      await harness.fixture.whenStable();
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    harness.detectChanges();

    const text = harness.routeNativeElement?.textContent ?? '';
    expect(text).toContain('Maria');
    expect(stub.getSubjectTags).toHaveBeenCalledWith(10);
    expect(text).toContain('Cabaña-21');
  });

  async function settle(harness: RouterTestingHarness) {
    for (let i = 0; i < 5; i++) {
      harness.detectChanges();
      await harness.fixture.whenStable();
      await new Promise(resolve => setTimeout(resolve, 0));
    }
    harness.detectChanges();
  }

  it('renders the processing progress bar with counts and percentage', async () => {
    const harness = await RouterTestingHarness.create('/reports/1');
    await settle(harness);

    const text = harness.routeNativeElement?.textContent ?? '';
    expect(text).toContain('2 of 4 images processed (50%)');
    const fill = harness.routeNativeElement?.querySelector('.progress-fill') as HTMLElement;
    expect(fill.style.width).toBe('50%');
  });

  it('re-fetches progress when a pipeline stats tick arrives', async () => {
    const harness = await RouterTestingHarness.create('/reports/1');
    await settle(harness);
    const callsAfterInit = stub.getReportProcessingProgress.mock.calls.length;

    stub.getReportProcessingProgress.mockResolvedValue({ total: 4, done: 4 });
    stub.pipelineStats.set({ total_pending: 1, images_per_sec: 1 });
    await settle(harness);

    expect(stub.getReportProcessingProgress.mock.calls.length).toBeGreaterThan(callsAfterInit);
    const text = harness.routeNativeElement?.textContent ?? '';
    expect(text).toContain('4 of 4 images processed (100%)');
  });
});
