import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { AppSnapshot, Config } from './types';

export const isNative = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export function getSnapshot(): Promise<AppSnapshot> {
	return invoke<AppSnapshot>('get_snapshot');
}

export function listMicrophones(): Promise<string[]> {
	return invoke<string[]>('list_microphones');
}

export function saveConfig(config: Config): Promise<AppSnapshot> {
	return invoke<AppSnapshot>('save_config', { config });
}

export function startRecording(): Promise<void> {
	return invoke('start_recording');
}

export function stopRecording(): Promise<void> {
	return invoke('stop_recording');
}

export function unloadModel(): Promise<void> {
	return invoke('unload_model');
}

export function onDictationState(handler: (snapshot: AppSnapshot) => void): Promise<UnlistenFn> {
	return listen<AppSnapshot>('dictation-state', (event) => handler(event.payload));
}