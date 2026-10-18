<script lang="ts">
	import { onMount } from 'svelte';
	import {
		isNative,
		getSnapshot,
		listMicrophones,
		saveConfig,
		setShortcutCapture,
		startRecording,
		stopRecording,
		unloadModel,
		onDictationState
	} from './lib/tauri';
	import { appState } from './lib/state.svelte';
	import { errorMessage, phaseLabels, type Config } from './lib/types';
	import Indicator from './lib/Indicator.svelte';
	import Replacements from './lib/Replacements.svelte';

	const indicatorMode = new URLSearchParams(window.location.search).has('indicator');

	let draft = $state<Config | null>(null);
	let microphones = $state<string[]>([]);
	let micError = $state<string | null>(null);
	let saving = $state(false);
	let saved = $state(false);
	let saveError = $state<string | null>(null);
	let actionError = $state<string | null>(null);
	let busyAction = $state<'start' | 'stop' | 'unload' | null>(null);
	let shortcutCapturing = $state(false);
	let shortcutPreparing = $state(false);
	let shortcutChosen = $state(false);
	let previousShortcut = '';
	let shortcutCaptureRequest: Promise<void> | undefined;

	async function beginShortcutCapture(event: FocusEvent) {
		if (!draft) return;
		const input = event.currentTarget as HTMLInputElement;
		previousShortcut = draft.shortcut;
		shortcutChosen = false;
		shortcutPreparing = true;
		actionError = null;
		shortcutCaptureRequest = setShortcutCapture(true);
		try {
			await shortcutCaptureRequest;
			shortcutCapturing = document.activeElement === input;
		} catch (error) {
			actionError = errorMessage(error);
		} finally {
			shortcutPreparing = false;
		}
	}

	async function endShortcutCapture() {
		shortcutCapturing = false;
		if (!shortcutCaptureRequest) return;
		try {
			await shortcutCaptureRequest;
			await setShortcutCapture(false);
		} catch (error) {
			actionError = errorMessage(error);
		} finally {
			shortcutCaptureRequest = undefined;
		}
	}

	function captureShortcut(event: KeyboardEvent) {
		if (event.key === 'Tab') return;
		event.preventDefault();
		if (!draft || !shortcutCapturing || event.repeat || event.isComposing) return;
		const input = event.currentTarget as HTMLInputElement;
		if (event.key === 'Escape') {
			draft.shortcut = previousShortcut;
			input.blur();
			return;
		}
		if (!event.code || ['Control', 'Shift', 'Alt', 'Meta', 'AltGraph'].includes(event.key)) return;
		const keys = [];
		if (event.ctrlKey) keys.push('Ctrl');
		if (event.altKey) keys.push('Alt');
		if (event.shiftKey) keys.push('Shift');
		if (event.metaKey) keys.push('Super');
		keys.push(event.code.replace(/^(Key|Digit)/, ''));
		draft.shortcut = keys.join('+');
		shortcutChosen = true;
		saved = false;
	}

	function releaseShortcut(event: KeyboardEvent) {
		if (shortcutChosen && !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) {
			(event.currentTarget as HTMLInputElement).blur();
		}
	}

	function toDraft(config: Config): Config {
		return {
			...config,
			microphone: config.microphone ?? '',
			replacements: config.replacements.map((row) => ({ ...row }))
		};
	}

	onMount(() => {
		if (!isNative || indicatorMode) return;
		let disposed = false;
		let unlisten: (() => void) | undefined;
		(async () => {
			try {
				const fn = await onDictationState((snapshot) => appState.apply(snapshot));
				if (disposed) fn();
				else unlisten = fn;
			} catch (error) {
				actionError = errorMessage(error);
				return;
			}
			if (disposed) return;
			try {
				const snapshot = await getSnapshot();
				appState.apply(snapshot);
				draft = toDraft(snapshot.config);
			} catch (error) {
				appState.loadError = errorMessage(error);
			}
			try {
				microphones = await listMicrophones();
			} catch (error) {
				micError = errorMessage(error);
			}
		})();
		return () => {
			disposed = true;
			unlisten?.();
		};
	});

	const phase = $derived(appState.snapshot?.phase ?? 'idle');
	const modelLoaded = $derived(appState.snapshot?.modelLoaded ?? false);
	const pathsReady = $derived(
		appState.snapshot !== null &&
			appState.snapshot.config.runtimePath.trim() !== '' &&
			appState.snapshot.config.modelPath.trim() !== ''
	);
	const recordDisabled = $derived(
		!pathsReady || phase === 'transcribing' || phase === 'inserting' || busyAction !== null
	);

	const recordLabel = $derived(
		phase === 'recording'
			? 'Stop test'
			: phase === 'transcribing'
				? 'Transcribing…'
				: phase === 'inserting'
					? 'Inserting…'
					: 'Test microphone'
	);

	async function toggleRecording() {
		const stopping = phase === 'recording';
		busyAction = stopping ? 'stop' : 'start';
		actionError = null;
		try {
			if (stopping) await stopRecording();
			else await startRecording();
		} catch (error) {
			actionError = errorMessage(error);
		} finally {
			busyAction = null;
		}
	}

	async function unload() {
		busyAction = 'unload';
		actionError = null;
		try {
			await unloadModel();
		} catch (error) {
			actionError = errorMessage(error);
		} finally {
			busyAction = null;
		}
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (!draft) return;
		await endShortcutCapture();
		saving = true;
		saved = false;
		saveError = null;
		const config: Config = {
			...draft,
			microphone: draft.microphone === '' ? null : draft.microphone,
			unloadAfterSeconds: Number(draft.unloadAfterSeconds)
		};
		try {
			const snapshot = await saveConfig(config);
			appState.apply(snapshot);
			draft = toDraft(snapshot.config);
			saved = true;
		} catch (error) {
			saveError = errorMessage(error);
		} finally {
			saving = false;
		}
	}
</script>

<svelte:window onblur={() => {
	if (shortcutCaptureRequest) (document.getElementById('shortcut') as HTMLInputElement | null)?.blur();
}} />

{#if indicatorMode}
	<Indicator />
{:else if !isNative}
	<main class="browser">
		<div class="card">
			<h1>Voxen</h1>
			<p>Voxen is a native desktop app for voice dictation. This page only shows its interface — open the Voxen app to view and change settings.</p>
			<p class="hint">Run <code>npm run tauri dev</code> to start the native app.</p>
		</div>
	</main>
{:else}
	<div class="shell">
		<aside class="sidebar">
			<div class="brand">Voxen</div>
			<div class="status">
				<span class="chip" data-phase={phase}>
					<span class="dot" aria-hidden="true"></span>
					{phaseLabels[phase]}
				</span>
				<span class="model" data-loaded={modelLoaded}>
					{modelLoaded ? 'Model loaded' : 'Model not loaded'}
				</span>
			</div>
			<button
				type="button"
				class="record"
				data-recording={phase === 'recording'}
				disabled={recordDisabled}
				onclick={toggleRecording}
			>
				{recordLabel}
			</button>
			<p class="hint">Focus Voxen to test the microphone — the transcript stays here and inserts nothing. To dictate, focus the target app and use the shortcut.</p>
			{#if appState.snapshot?.error}
				<p class="error" role="alert">{appState.snapshot.error}</p>
			{/if}
			{#if actionError}
				<p class="error" role="alert">{actionError}</p>
			{/if}
			{#if appState.snapshot?.liveTranscript}
				<section class="transcript">
					<h2>Live</h2>
					<p>{appState.snapshot.liveTranscript}</p>
				</section>
			{/if}
			{#if appState.snapshot?.lastTranscript}
				<section class="transcript">
					<h2>Last transcription</h2>
					<p>{appState.snapshot.lastTranscript}</p>
				</section>
			{/if}
		</aside>
		<main class="content">
			{#if draft}
				<form onsubmit={save} oninput={() => (saved = false)}>
					<section class="group">
						<h2>Microphone</h2>
						{#if micError}
							<p class="error" role="alert">Could not load microphones: {micError}</p>
						{/if}
						<div class="field">
							<label for="microphone">Input device</label>
							<select id="microphone" bind:value={draft.microphone}>
								<option value="">System default</option>
								{#each microphones as mic (mic)}
									<option value={mic}>{mic}</option>
								{/each}
							</select>
						</div>
					</section>

					<section class="group">
						<h2>Activation</h2>
						<div class="field">
							<label for="shortcut">Shortcut</label>
							<input id="shortcut" type="text" readonly
								value={shortcutPreparing ? 'Preparing...' : shortcutCapturing && !shortcutChosen ? 'Press your shortcut...' : draft.shortcut}
								aria-describedby="shortcut-hint"
								onfocus={beginShortcutCapture} onblur={endShortcutCapture}
								onkeydown={captureShortcut} onkeyup={releaseShortcut} />
							<p id="shortcut-hint" class="hint">Click the field and press your shortcut. Release the keys, then save. Esc cancels.</p>
						</div>
						<fieldset class="field">
							<legend>Mode</legend>
							<label class="choice">
								<input type="radio" name="activationMode" value="hold" bind:group={draft.activationMode} />
								<span>Hold<small>Record while the shortcut is held down.</small></span>
							</label>
							<label class="choice">
								<input type="radio" name="activationMode" value="toggle" bind:group={draft.activationMode} />
								<span>Toggle<small>Press once to start, press again to stop.</small></span>
							</label>
						</fieldset>
					</section>

					<section class="group">
						<h2>Local model</h2>
						<p class="intro">Speech recognition runs locally on this machine. Point Voxen at your speech runtime and a model file on disk.</p>
						<div class="field">
							<label for="runtimePath">Runtime path</label>
							<input id="runtimePath" type="text" bind:value={draft.runtimePath} />
							<p class="hint">Path to the NeMo SDK root directory or nemo_speech_asr_c.dll.</p>
						</div>
						<div class="field">
							<label for="modelPath">Model path</label>
							<input id="modelPath" type="text" bind:value={draft.modelPath} />
							<p class="hint">Path to a local model file.</p>
						</div>
						<div class="field">
							<label for="device">Device</label>
							<input id="device" type="text" bind:value={draft.device} />
							<p class="hint">Device identifier used by the runtime, for example cpu or vulkan:0 for an RX9060XT.</p>
						</div>
						<div class="field">
							<label for="unloadAfterSeconds">Unload model after</label>
							<div class="pair">
								<input id="unloadAfterSeconds" type="number" min="0" step="1" bind:value={draft.unloadAfterSeconds} />
								<span class="unit">seconds idle</span>
							</div>
							<p class="hint">0 keeps the model resident.</p>
							<button type="button" disabled={!modelLoaded || busyAction !== null} onclick={unload}>
								Unload model
							</button>
						</div>
					</section>

					<section class="group">
						<h2>Replacements</h2>
						<p class="intro">Words or phrases replaced automatically in every transcription.</p>
						<Replacements bind:replacements={draft.replacements} />
					</section>

					<div class="savebar">
						{#if saveError}
							<p class="error" role="alert">{saveError}</p>
						{/if}
						{#if saved}
							<p class="confirm" role="status">Saved.</p>
						{/if}
						<button type="submit" class="primary" disabled={saving}>
							{saving ? 'Saving…' : 'Save changes'}
						</button>
					</div>
				</form>
			{:else if appState.loadError}
				<p class="error" role="alert">Could not load settings: {appState.loadError}</p>
			{:else}
				<p class="loading">Loading…</p>
			{/if}
		</main>
	</div>
{/if}

<style>
	.browser {
		min-height: 100vh;
		display: grid;
		place-items: center;
		padding: 24px;
	}

	.card {
		max-width: 26rem;
		background: var(--surface);
		border: 1px solid var(--line);
		border-radius: 8px;
		padding: 20px 22px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.card h1 {
		font-size: 17px;
	}

	.shell {
		display: grid;
		grid-template-columns: 232px 1fr;
		min-height: 100vh;
	}

	.sidebar {
		background: var(--surface-soft);
		border-right: 1px solid var(--line);
		padding: 16px 14px;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.brand {
		font-size: 16px;
		font-weight: 700;
	}

	.status {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: 12px;
		font-weight: 600;
		width: fit-content;
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--ink-faint);
	}

	.chip[data-phase='recording'] .dot {
		background: var(--danger);
		animation: pulse 1.6s ease-in-out infinite;
	}

	.chip[data-phase='transcribing'] .dot,
	.chip[data-phase='inserting'] .dot {
		background: var(--accent);
	}

	.chip[data-phase='error'] .dot {
		background: var(--danger);
	}

	.model {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: 12px;
		color: var(--ink-soft);
	}

	.model::before {
		content: '';
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--ink-faint);
	}

	.model[data-loaded='true']::before {
		background: var(--ok);
	}

	.record {
		padding: 8px 12px;
		font-weight: 600;
		background: var(--accent);
		border-color: var(--accent);
		color: #fff;
	}

	.record:not(:disabled):hover {
		background: var(--accent-hover);
		border-color: var(--accent-hover);
	}

	.record[data-recording='true'] {
		background: var(--danger-soft);
		border-color: var(--danger);
		color: var(--danger);
	}

	.record[data-recording='true']:not(:disabled):hover {
		background: #55343b;
		border-color: var(--danger);
	}

	.transcript {
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--surface);
		padding: 10px;
	}

	.transcript h2 {
		font-size: 11.5px;
		font-weight: 600;
		color: var(--ink-soft);
		margin-bottom: 4px;
	}

	.transcript p {
		font-size: 12px;
		max-height: 8rem;
		overflow: auto;
		white-space: pre-wrap;
	}

	.content {
		padding: 20px 24px;
		max-width: 60rem;
	}

	.loading {
		color: var(--ink-soft);
	}

	.group {
		padding: 16px 0;
		border-bottom: 1px solid var(--line);
	}

	.group > h2 {
		font-size: 14px;
		margin-bottom: 10px;
	}

	.group:last-of-type {
		border-bottom: none;
	}

	.choice {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		font-size: 13px;
		cursor: pointer;
	}

	.choice + .choice {
		margin-top: 6px;
	}

	.choice small {
		display: block;
		color: var(--ink-soft);
		font-size: 11.5px;
	}

	.pair {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.pair input {
		max-width: 8rem;
	}

	.unit {
		color: var(--ink-soft);
		font-size: 12px;
	}

	.field button {
		width: fit-content;
		margin-top: 8px;
	}

	.savebar {
		position: sticky;
		bottom: 0;
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 12px 0;
		background: var(--bg);
	}

	.savebar .error,
	.confirm {
		font-size: 12px;
	}

	.confirm {
		color: var(--ok);
	}

	.primary {
		background: var(--accent);
		border-color: var(--accent);
		color: #fff;
		font-weight: 600;
	}

	.primary:not(:disabled):hover {
		background: var(--accent-hover);
		border-color: var(--accent-hover);
	}

	@keyframes pulse {
		50% {
			opacity: 0.35;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.dot {
			animation: none;
		}
	}

	@media (max-width: 720px) {
		.shell {
			grid-template-columns: 1fr;
		}

		.sidebar {
			border-right: none;
			border-bottom: 1px solid var(--line);
		}

		.content {
			padding: 16px;
		}
	}
</style>
