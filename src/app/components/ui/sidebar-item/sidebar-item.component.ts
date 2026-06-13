import { Component, Input, HostBinding } from '@angular/core';
import { RouterLink, RouterLinkActive } from '@angular/router';
import { NgIf } from '@angular/common';

@Component({
  selector: 'app-sidebar-item',
  standalone: true,
  imports: [RouterLink, RouterLinkActive, NgIf],
  template: `
    <ng-container *ngIf="routerLink; else buttonTpl">
      <a
        [routerLink]="routerLink"
        routerLinkActive="folder-item--active"
        class="folder-item"
        [class.folder-item--active]="isActive"
      >
        <ng-content></ng-content>
      </a>
    </ng-container>
    <ng-template #buttonTpl>
      <button
        type="button"
        class="folder-item"
        [class.folder-item--active]="isActive"
      >
        <ng-content></ng-content>
      </button>
    </ng-template>
  `,
  styleUrl: './sidebar-item.component.css'
})
export class SidebarItemComponent {
  @HostBinding('class.app-sidebar-item') appSidebarItem = true;
  @Input() isActive: boolean = false;
  @Input() routerLink: string | any[] | null = null;
}
