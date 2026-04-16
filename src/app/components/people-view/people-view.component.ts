import { Component, inject, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';
import { Subject } from '../../models/models';
import { RouterLink } from '@angular/router';

@Component({
  selector: 'app-people-view',
  standalone: true,
  imports: [CommonModule, RouterLink],
  templateUrl: './people-view.component.html',
  styleUrl: './people-view.component.css'
})
export class PeopleViewComponent implements OnInit {
  protected photoService = inject(PhotoService);
  protected faceCropUrls = signal<Record<number, string>>({});

  async ngOnInit() {
    await this.photoService.loadSubjects();
    void this.loadThumbnails();
  }

  private async loadThumbnails() {
    const subjects = this.photoService.subjects();
    const urls: Record<number, string> = {};
    
    // Load crops in parallel (with some concurrency limit if needed, but here simple parallel is fine for now)
    await Promise.all(subjects.map(async (s) => {
      if (s.thumbnail_face_id) {
        try {
          const path = await this.photoService.getFaceCrop(s.thumbnail_face_id);
          const url = this.photoService.thumbnailUrl(path);
          if (url) urls[s.id] = url;
        } catch (e) {
          console.error(`Failed to load thumbnail for subject ${s.id}`, e);
        }
      }
    }));
    
    this.faceCropUrls.set(urls);
  }
}
