import {
  Component,
  OnInit,
  inject,
  signal,
  computed,
  ChangeDetectionStrategy,
} from '@angular/core';
import { CommonModule, Location } from '@angular/common';
import { ActivatedRoute, RouterLink } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { SearchResult, VirtualRow, SubjectDetail } from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';
import { PhotoGridComponent } from '../photo-grid/photo-grid.component';
import { FormsModule } from '@angular/forms';
import { buildJustifiedRows } from '../../utils/justified-layout';
import { LightboxComponent } from '../lightbox/lightbox.component';

@Component({
  selector: 'app-subject-detail',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CommonModule,
    RouterLink,
    LucideAngularModule,
    PhotoGridComponent,
    FormsModule,
    LightboxComponent,
  ],
  templateUrl: './subject-detail.component.html',
  styleUrl: './subject-detail.component.css',
})
export class SubjectDetailComponent implements OnInit {
  private route = inject(ActivatedRoute);
  private location = inject(Location);
  protected photos = inject(PhotoService);

  protected subjectId = signal<number | null>(null);
  protected detail = signal<SubjectDetail | null>(null);
  protected subjectPhotos = signal<SearchResult[]>([]);
  protected faceCropUrl = signal<string | null>(null);

  protected isEditingName = signal(false);
  protected editedName = signal('');
  protected isMenuOpen = signal(false);

  protected readonly virtualRows = computed<VirtualRow[]>(() => {
    const images = this.subjectPhotos();
    const width = this.photos.viewportWidth();
    const targetHeight = this.photos.targetRowHeight();
    
    const rows: VirtualRow[] = [];
    const justifiedRows = buildJustifiedRows(images, width, targetHeight, 4);
    for (const row of justifiedRows) {
      rows.push({ type: 'row', images: row.images, rowHeight: row.rowHeight });
    }
    return rows;
  });

  ngOnInit() {
    this.route.params.subscribe((params) => {
      const id = Number(params['id']);
      if (!isNaN(id)) {
        this.subjectId.set(id);
        void this.loadData(id);
      }
    });
  }

  private async loadData(id: number) {
    try {
      const detail = await this.photos.getSubjectDetail(id);
      this.detail.set(detail);
      this.editedName.set(detail.subject.name || '');

      if (detail.subject.thumbnail_face_id) {
        const path = await this.photos.getFaceCrop(detail.subject.thumbnail_face_id);
        this.faceCropUrl.set(this.photos.thumbnailUrl(path));
      }

      const photos = await this.photos.getSubjectPhotos(id);
      this.subjectPhotos.set(photos);
    } catch (e) {
      console.error('Failed to load subject detail', e);
      // Fallback or navigate back
      this.location.back();
    }
  }

  protected goBack() {
    this.location.back();
  }

  protected startEdit() {
    this.isEditingName.set(true);
  }

  protected cancelEdit() {
    this.isEditingName.set(false);
    this.editedName.set(this.detail()?.subject.name || '');
  }

  protected async saveName() {
    const id = this.subjectId();
    const name = this.editedName().trim();
    if (id !== null) {
      await this.photos.nameSubject(id, name || null);
      this.detail.update(d => {
        if (d) d.subject.name = name || null;
        return d;
      });
      this.isEditingName.set(false);
    }
  }

  protected toggleMenu() {
    this.isMenuOpen.update(v => !v);
  }

  protected closeMenu() {
    this.isMenuOpen.set(false);
  }
}
