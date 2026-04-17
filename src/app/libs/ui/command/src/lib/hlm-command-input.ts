import { ChangeDetectionStrategy, Component, input } from '@angular/core';
import { LucideAngularModule } from 'lucide-angular';
import { BrnCommandInput } from '@spartan-ng/brain/command';
import { classes } from '@spartan-ng/helm/utils';

@Component({
	selector: 'hlm-command-input',
	imports: [LucideAngularModule, BrnCommandInput],
	changeDetection: ChangeDetectionStrategy.OnPush,
	template: `
		<div class="flex items-center border-b px-3 gap-2">
			<lucide-icon name="search" [size]="16" class="shrink-0 opacity-50" />
			<input
				brnCommandInput
				data-slot="command-input"
				class="flex h-10 w-full bg-transparent py-3 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50"
				[id]="id()"
				[placeholder]="placeholder()"
			/>
		</div>
	`,
})
export class HlmCommandInput {
	public readonly id = input<string | undefined>();
	public readonly placeholder = input<string>('Search...');

	constructor() {
		classes(() => '');
	}
}
