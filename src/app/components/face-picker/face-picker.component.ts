import {
  Component,
  OnInit,
  inject,
  signal,
  ChangeDetectionStrategy,
} from '@angular/core';
import { CommonModule, Location } from '@angular/common';
import { ActivatedRoute } from '@angular/router';
import { PhotoService } from '../../services/photo.service';
import { Face } from '../../models/models';
import { LucideAngularModule } from 'lucide-angular';

interface FaceCrop {
  face: Face;
  url: string;
}

@Component({
  selector: 'app-face-picker',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CommonModule, LucideAngularModule],
  templateUrl: './face-picker.component.html',
  styleUrl: './face-picker.component.css',
})
export class FacePickerComponent implements OnInit {
  private route = inject(ActivatedRoute);
  private location = inject(Location);
  protected photos = inject(PhotoService);

  protected subjectId = signal<number | null>(null);
  protected currentThumbnailFaceId = signal<number | null>(null);
  protected faceCrops = signal<FaceCrop[]>([]);
  protected isLoading = signal(true);

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
    this.isLoading.set(true);
    try {
      const detail = await this.photos.getSubjectDetail(id);
      this.currentThumbnailFaceId.set(detail.subject.thumbnail_face_id);

      const faces = await this.photos.loadFaces(id);
      const crops: FaceCrop[] = [];
      
      for (const face of faces) {
        const path = await this.photos.getFaceCrop(face.id);
        crops.push({
          face,
          url: this.photos.thumbnailUrl(path) || '',
        });
      }
      this.faceCrops.set(crops);
    } catch (e) {
      console.error('Failed to load face crops', e);
    } finally {
      this.isLoading.set(false);
    }
  }

  protected goBack() {
    this.location.back();
  }

  protected async selectFace(faceId: number) {
    const sid = this.subjectId();
    if (sid !== null) {
      await this.photos.setSubjectThumbnail(sid, faceId);
      this.location.back();
    }
  }
}
