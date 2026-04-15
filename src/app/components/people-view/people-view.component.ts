import { Component, inject, OnInit } from '@angular/core';
import { CommonModule } from '@angular/common';
import { PhotoService } from '../../services/photo.service';
import { Subject } from '../../models/models';
import { FormsModule } from '@angular/forms';

@Component({
  selector: 'app-people-view',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './people-view.component.html',
  styleUrl: './people-view.component.css'
})
export class PeopleViewComponent implements OnInit {
  protected photoService = inject(PhotoService);
  editingId: number | null = null;
  editName: string = '';

  ngOnInit() {
    void this.photoService.loadSubjects();
  }

  startEdit(subject: Subject) {
    this.editingId = subject.id;
    this.editName = subject.name || '';
  }

  async saveEdit(subject: Subject) {
    if (this.editingId === subject.id) {
      await this.photoService.nameSubject(subject.id, this.editName);
      this.editingId = null;
    }
  }

  cancelEdit() {
    this.editingId = null;
  }
}
