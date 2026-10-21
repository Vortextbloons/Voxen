import { getVersion } from '@tauri-apps/api/app';
import { check, type Update } from '@tauri-apps/plugin-updater';

export type { Update };

export function currentVersion(): Promise<string> {
	return getVersion();
}

export function findUpdate(): Promise<Update | null> {
	return check();
}

export async function installUpdate(
	update: Update,
	onProgress: (percent: number) => void
): Promise<void> {
	let downloaded = 0;
	let total = 0;
	await update.downloadAndInstall((event) => {
		if (event.event === 'Started') {
			total = event.data.contentLength ?? 0;
			onProgress(0);
		} else if (event.event === 'Progress') {
			downloaded += event.data.chunkLength;
			onProgress(total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0);
		} else {
			onProgress(100);
		}
	});
}
