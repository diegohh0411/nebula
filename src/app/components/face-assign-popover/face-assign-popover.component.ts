import {
  Component,
  ChangeDetectionStrategy,
  input,
  output,
  inject,
  signal,
  computed,
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { LucideAngularModule } from 'lucide-angular';
import { BrnPopoverImports } from '@spartan-ng/brain/popover';
import { BrnCommandImports } from '@spartan-ng/brain/command';
import { HlmPopoverImports } from '@spartan-ng/helm/popover';
import { HlmCommandImports } from '@spartan-ng/helm/command';
import { PhotoService } from '../../services/photo.service';
import { Face, Subject } from '../../models/models';

@Component({
  selector: 'app-face-assign-popover',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [
    CommonModule,
    LucideAngularModule,
    BrnPopoverImports,
    BrnCommandImports,
    HlmPopoverImports,
    HlmCommandImports,
  ],
  templateUrl: './face-assign-popover.component.html',
  styleUrl: './face-assign-popover.component.css',
})
export class FaceAssignPopoverComponent {
  readonly face = input.required<Face>();
  readonly assigned = output<{ face: Face; subject: Subject }>();

  protected photos = inject(PhotoService);
  protected query = signal('');
  protected isOpen = signal(false);

  protected filteredSubjects = computed(() => {
    const q = this.query().toLowerCase().trim();
    if (!q) return this.photos.subjects();
    return this.photos.subjects().filter(s =>
      s.name?.toLowerCase().includes(q) ?? false
    );
  });

  async open() {
    this.isOpen.set(true);
    this.query.set('');
  }

  close() {
    this.isOpen.set(false);
    this.query.set('');
  }

  async selectSubject(subject: Subject) {
    await this.photos.assignFaceToSubject(this.face().id, subject.id);
    this.assigned.emit({ face: this.face(), subject });
    this.close();
  }

  async createSubject() {
    const name = this.query().trim() || undefined;
    const subject = await this.photos.createSubjectForFace(this.face().id, name);
    this.assigned.emit({ face: this.face(), subject });
    this.close();
  }
}
