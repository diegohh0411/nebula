import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { SidebarSectionComponent } from './sidebar-section.component';

@Component({
  standalone: true,
  imports: [SidebarSectionComponent],
  template: `
    <app-sidebar-section [title]="title" [divider]="divider">
      <button sidebarSectionAction class="act">add</button>
      <span class="body">section-body</span>
    </app-sidebar-section>
  `,
})
class HostComponent {
  title?: string;
  divider = false;
}

describe('SidebarSectionComponent', () => {
  async function render(setup: (h: HostComponent) => void) {
    TestBed.resetTestingModule();
    await TestBed.configureTestingModule({ imports: [HostComponent] }).compileComponents();
    const fixture = TestBed.createComponent(HostComponent);
    setup(fixture.componentInstance);
    fixture.detectChanges();
    return fixture.nativeElement as HTMLElement;
  }

  it('renders the title header when title is provided', async () => {
    const el = await render((h) => (h.title = 'Folders'));
    const title = el.querySelector('.sidebar-section-title');
    expect(title).toBeTruthy();
    expect(title!.textContent).toContain('Folders');
  });

  it('omits the header when no title is provided', async () => {
    const el = await render(() => {});
    expect(el.querySelector('.sidebar-section-header')).toBeNull();
    expect(el.querySelector('.sidebar-section-title')).toBeNull();
  });

  it('projects the action slot into the header when a title exists', async () => {
    const el = await render((h) => (h.title = 'Folders'));
    const header = el.querySelector('.sidebar-section-header');
    expect(header).toBeTruthy();
    expect(header!.querySelector('.act')).toBeTruthy();
  });

  it('does not render the action slot when no title exists', async () => {
    const el = await render(() => {});
    expect(el.querySelector('.act')).toBeNull();
  });

  it('always projects default content', async () => {
    const el = await render(() => {});
    expect(el.querySelector('.body')).toBeTruthy();
    expect(el.querySelector('.body')!.textContent).toContain('section-body');
  });

  it('renders the divider only when divider is true', async () => {
    const withDivider = await render((h) => (h.divider = true));
    expect(withDivider.querySelector('.sidebar-section-divider')).toBeTruthy();

    const withoutDivider = await render((h) => (h.divider = false));
    expect(withoutDivider.querySelector('.sidebar-section-divider')).toBeNull();
  });
});
