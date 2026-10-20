export type ActivationMode = 'hold' | 'toggle';

export type CleanupMode = 'raw' | 'light' | 'polished';

export type Phase = 'idle' | 'starting' | 'recording' | 'transcribing' | 'inserting' | 'error';

export interface Replacement {
	from: string;
	to: string;
}

export interface Config {
	microphone: string | null;
	shortcut: string;
	activationMode: ActivationMode;
	runtimePath: string;
	modelPath: string;
	device: string;
	replacements: Replacement[];
	unloadAfterSeconds: number;
	cleanupMode: CleanupMode;
	cleanupRuntimePath: string;
	cleanupModelPath: string;
}

export interface AppSnapshot {
	config: Config;
	phase: Phase;
	error: string | null;
	lastTranscript: string | null;
	liveTranscript: string | null;
	modelLoaded: boolean;
}

export const phaseLabels: Record<Phase, string> = {
	idle: 'Idle',
	starting: 'Starting',
	recording: 'Recording',
	transcribing: 'Transcribing',
	inserting: 'Inserting',
	error: 'Error'
};

export function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}