import type { AppSnapshot } from './types';

class AppState {
	snapshot = $state<AppSnapshot | null>(null);
	loadError = $state<string | null>(null);

	apply(next: AppSnapshot) {
		this.snapshot = next;
		this.loadError = null;
	}
}

export const appState = new AppState();