import { Component, ChangeDetectionStrategy, Input } from '@angular/core';

@Component({
  selector: 'app-sidebar-section',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  // The [sidebarSectionAction] slot lives inside the @if (title) block, so a
  // projected action is only shown when the section has a title. With no title,
  // Angular still claims action-tagged content for that (unrendered) slot, so it
  // simply does not appear — the default <ng-content> does not pick it up.
  template: `
    @if (divider) {
      <div class="sidebar-section-divider"></div>
    }
    @if (title) {
      <div class="sidebar-section-header">
        <span class="sidebar-section-title">{{ title }}</span>
        <ng-content select="[sidebarSectionAction]"></ng-content>
      </div>
    }
    <ng-content></ng-content>
  `,
  styleUrl: './sidebar-section.component.css',
})
export class SidebarSectionComponent {
  @Input() title?: string;
  @Input() divider = false;
}
