import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { SidebarItemComponent } from './sidebar-item.component';

@Component({
  standalone: true,
  imports: [SidebarItemComponent],
  template: `
    <app-sidebar-item><span class="proj">click-content</span></app-sidebar-item>
    <app-sidebar-item routerLink="/somewhere"><span class="proj">link-content</span></app-sidebar-item>
  `,
})
class HostComponent {}

describe('SidebarItemComponent', () => {
  async function render() {
    await TestBed.configureTestingModule({
      imports: [HostComponent],
      providers: [provideRouter([])],
    }).compileComponents();
    const fixture = TestBed.createComponent(HostComponent);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  }

  it('projects content in button (click) mode', async () => {
    const el = await render();
    const button = el.querySelector('button.folder-item');
    expect(button).toBeTruthy();
    expect(button!.textContent).toContain('click-content');
  });

  it('projects content in routerLink (anchor) mode', async () => {
    const el = await render();
    const anchor = el.querySelector('a.folder-item');
    expect(anchor).toBeTruthy();
    // Regression: with two <ng-content> slots the anchor rendered empty.
    expect(anchor!.textContent).toContain('link-content');
  });
});
