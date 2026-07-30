import { TestBed } from '@angular/core/testing';
import { importProvidersFrom } from '@angular/core';
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
});
