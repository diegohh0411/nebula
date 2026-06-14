import { Component, Input, HostBinding } from '@angular/core';
import { RouterLink, RouterLinkActive } from '@angular/router';
import { NgTemplateOutlet } from '@angular/common';

@Component({
  selector: 'app-sidebar-item',
  standalone: true,
  imports: [RouterLink, RouterLinkActive, NgTemplateOutlet],
  // A single <ng-content> lives in #inner and is rendered into whichever host
  // element is active. Two separate <ng-content> slots (one per branch) would
  // leave the inactive branch's slot empty — Angular only projects into one —
  // which previously blanked out the routerLink (anchor) items.
  template: `
    @if (routerLink) {
      <a
        [routerLink]="routerLink"
        routerLinkActive="folder-item--active"
        class="folder-item"
        [class.folder-item--active]="isActive"
      >
        <ng-container [ngTemplateOutlet]="inner"></ng-container>
      </a>
    } @else {
      <button
        type="button"
        class="folder-item"
        [class.folder-item--active]="isActive"
      >
        <ng-container [ngTemplateOutlet]="inner"></ng-container>
      </button>
    }
    <ng-template #inner><ng-content></ng-content></ng-template>
  `,
  styleUrl: './sidebar-item.component.css'
})
export class SidebarItemComponent {
  @HostBinding('class.app-sidebar-item') appSidebarItem = true;
  @Input() isActive: boolean = false;
  @Input() routerLink: string | any[] | null = null;
}
