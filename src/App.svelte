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
	import { currentVersion, findUpdate, installUpdate, type Update } from './lib/updater';
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
	let appVersion = $state('');
	let update = $state<Update | null>(null);
	let updateStatus = $state<'idle' | 'checking' | 'current' | 'available' | 'installing' | 'error'>('idle');
	let updateProgress = $state(0);
	let updateError = $state<string | null>(null);

	const tabs = [
		{ id: 'microphone', label: 'Microphone' },
		{ id: 'activation', label: 'Activation' },
		{ id: 'model', label: 'Model' },
		{ id: 'cleanup', label: 'Cleanup' },
		{ id: 'replacements', label: 'Replacements' },
		{ id: 'updates', label: 'Updates' }
	] as const;
	type SettingsTab = (typeof tabs)[number]['id'];
	let activeTab = $state<SettingsTab>('microphone');

	function handleTabKeydown(event: KeyboardEvent, index: number) {
		const last = tabs.length - 1;
		let next: number;
		if (event.key === 'ArrowRight') next = index === last ? 0 : index + 1;
		else if (event.key === 'ArrowLeft') next = index === 0 ? last : index - 1;
		else if (event.key === 'Home') next = 0;
		else if (event.key === 'End') next = last;
		else return;
		event.preventDefault();
		const id = tabs[next].id;
		activeTab = id;
		document.getElementById(`tab-${id}`)?.focus();
	}

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
			if (disposed) return;
			try {
				appVersion = await currentVersion();
			} catch {
				appVersion = '';
			}
			if (!disposed) void checkUpdates();
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
		!pathsReady || phase === 'starting' || phase === 'transcribing' || phase === 'inserting' || busyAction !== null
	);

	const recordLabel = $derived(
		phase === 'recording'
			? 'Stop test'
			: phase === 'starting'
				? 'Starting…'
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

	async function checkUpdates() {
		if (updateStatus === 'checking' || updateStatus === 'installing') return;
		updateStatus = 'checking';
		updateError = null;
		try {
			const found = await findUpdate();
			await update?.close();
			update = found;
			updateStatus = found ? 'available' : 'current';
		} catch (error) {
			update = null;
			updateStatus = 'error';
			updateError = errorMessage(error);
		}
	}

	async function install() {
		if (!update) return;
		updateStatus = 'installing';
		updateProgress = 0;
		updateError = null;
		try {
			await installUpdate(update, (percent) => (updateProgress = percent));
		} catch (error) {
			updateStatus = 'error';
			updateError = errorMessage(error);
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
				{#if updateStatus === 'available' && update}
					<button type="button" class="update" onclick={() => (activeTab = 'updates')}>
						Update {update.version} available
					</button>
				{/if}
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
			<header class="page-header">
				<h1>Settings</h1>
				<p>Configure dictation, recognition, and text cleanup.</p>
			</header>
			{#if draft}
				<form onsubmit={save} oninput={() => (saved = false)}>
					<div class="tabs" role="tablist" aria-label="Settings sections">
						{#each tabs as tab, index (tab.id)}
							<button
								type="button"
								class="tab"
								role="tab"
								id={`tab-${tab.id}`}
								aria-controls={`panel-${tab.id}`}
								aria-selected={activeTab === tab.id}
								tabindex={activeTab === tab.id ? 0 : -1}
								onclick={() => (activeTab = tab.id)}
								onkeydown={(event) => handleTabKeydown(event, index)}
							>
								{tab.label}
							</button>
						{/each}
					</div>

					{#if activeTab === 'microphone'}
						<div class="group" role="tabpanel" id="panel-microphone" aria-labelledby="tab-microphone">
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
						</div>
					{:else if activeTab === 'activation'}
						<div class="group" role="tabpanel" id="panel-activation" aria-labelledby="tab-activation">
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
						</div>
					{:else if activeTab === 'model'}
						<div class="group" role="tabpanel" id="panel-model" aria-labelledby="tab-model">
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
						</div>
					{:else if activeTab === 'cleanup'}
						<div class="group" role="tabpanel" id="panel-cleanup" aria-labelledby="tab-cleanup">
							<p class="intro">Optional second pass over every transcription with a small local model. If cleanup fails or rewrites too much, the original transcript is kept and a warning appears in the sidebar.</p>
							<fieldset class="field">
								<legend>Mode</legend>
								<label class="choice">
									<input type="radio" name="cleanupMode" value="raw" bind:group={draft.cleanupMode} />
									<span>Raw<small>Keep the transcript unchanged.</small></span>
								</label>
								<label class="choice">
									<input type="radio" name="cleanupMode" value="light" bind:group={draft.cleanupMode} />
									<span>Light<small>Remove fillers and resolve self-corrections, keep every other word.</small></span>
								</label>
								<label class="choice">
									<input type="radio" name="cleanupMode" value="polished" bind:group={draft.cleanupMode} />
									<span>Polished<small>Light cleanup plus punctuation, capitalization, and lists.</small></span>
								</label>
							</fieldset>
							<div class="field">
								<label for="cleanupRuntimePath">Cleanup runtime path</label>
								<input id="cleanupRuntimePath" type="text" bind:value={draft.cleanupRuntimePath} />
								<p class="hint">Path to llama-server.exe or the folder containing it.</p>
							</div>
							<div class="field">
								<label for="cleanupModelPath">Cleanup model path</label>
								<input id="cleanupModelPath" type="text" bind:value={draft.cleanupModelPath} />
								<p class="hint">Path to a small local instruction model file, for example Qwen3.5-2B-Q4_K_S.gguf.</p>
							</div>
						</div>
					{:else if activeTab === 'replacements'}
						<div class="group" role="tabpanel" id="panel-replacements" aria-labelledby="tab-replacements">
							<p class="intro">Words or phrases replaced automatically in every transcription.</p>
							<Replacements bind:replacements={draft.replacements} />
						</div>
					{:else}
						<div class="group" role="tabpanel" id="panel-updates" aria-labelledby="tab-updates">
							<p class="intro">Voxen updates itself from GitHub releases. Downloading and installing restarts the app.</p>
							<div class="field">
								<span class="label">Installed version</span>
								<p class="hint">{appVersion || 'Unknown'}</p>
							</div>
							{#if updateStatus === 'available' && update}
								<div class="field">
									<span class="label">Version {update.version} available</span>
									{#if update.body}
										<p class="hint">{update.body}</p>
									{/if}
									<button type="button" class="primary" onclick={install}>Download and install</button>
								</div>
							{:else if updateStatus === 'installing'}
								<div class="field">
									<span class="label">Installing update{updateProgress > 0 ? ` — ${updateProgress}%` : '…'}</span>
									<progress max="100" value={updateProgress}></progress>
								</div>
							{:else if updateStatus === 'checking'}
								<p class="hint">Checking for updates…</p>
							{:else if updateStatus === 'current'}
								<p class="confirm">Voxen is up to date.</p>
							{:else if updateStatus === 'error'}
								<p class="error" role="alert">Update check failed: {updateError}</p>
							{/if}
							<div class="field">
								<button
									type="button"
									disabled={updateStatus === 'checking' || updateStatus === 'installing'}
									onclick={checkUpdates}
								>
									Check for updates
								</button>
							</div>
						</div>
					{/if}

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
		grid-template-columns: 244px 1fr;
		min-height: 100vh;
	}

	.sidebar {
		background: var(--surface-soft);
		border-right: 1px solid var(--line);
		padding: 24px 18px;
		display: flex;
		flex-direction: column;
		gap: 16px;
	}

	.brand {
		font-size: 15px;
		font-weight: 650;
		letter-spacing: 0.01em;
	}

	.status {
		display: flex;
		flex-direction: column;
		gap: 7px;
		padding-bottom: 16px;
		border-bottom: 1px solid var(--line);
	}

	.chip {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		font-size: 12px;
		font-weight: 500;
		color: var(--ink-soft);
		width: fit-content;
	}

	.dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--ink-faint);
	}

	.chip[data-phase='recording'] .dot {
		background: linear-gradient(135deg, var(--accent-cyan), var(--accent-violet));
		box-shadow: 0 0 6px rgba(94, 146, 255, 0.75);
		animation: pulse 1.6s ease-in-out infinite;
	}

	.chip[data-phase='transcribing'] .dot {
		background: var(--accent-blue);
		box-shadow: 0 0 6px rgba(59, 130, 246, 0.65);
	}

	.chip[data-phase='inserting'] .dot {
		background: var(--accent-violet);
		box-shadow: 0 0 6px rgba(155, 92, 255, 0.65);
	}

	.chip[data-phase='error'] .dot {
		background: var(--danger);
	}

	.model {
		display: inline-flex;
		align-items: center;
		gap: 7px;
		font-size: 11.5px;
		color: var(--ink-faint);
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

	.update {
		width: fit-content;
		padding: 4px 9px;
		font-size: 11.5px;
		color: var(--accent-cyan);
		border-color: rgba(47, 180, 255, 0.35);
		background: rgba(47, 180, 255, 0.08);
	}

	.record {
		width: 100%;
		padding: 9px 12px;
		font-size: 12.5px;
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
		border-color: rgba(248, 113, 113, 0.35);
		color: var(--danger);
	}

	.record[data-recording='true']:not(:disabled):hover {
		background: #351d22;
		border-color: rgba(248, 113, 113, 0.5);
	}

	.transcript {
		border: 1px solid var(--line);
		border-radius: var(--radius);
		background: var(--surface);
		padding: 10px 12px;
	}

	.transcript h2 {
		font-size: 10.5px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.07em;
		color: var(--ink-faint);
		margin-bottom: 6px;
	}

	.transcript p {
		font-size: 12px;
		color: var(--ink-soft);
		max-height: 8rem;
		overflow: auto;
		white-space: pre-wrap;
	}

	.content {
		padding: 32px 36px 24px;
		max-width: 60rem;
		display: flex;
		flex-direction: column;
	}

	.content > form {
		display: flex;
		flex-direction: column;
		flex: 1;
	}

	.page-header {
		margin-bottom: 4px;
	}

	.page-header h1 {
		font-size: 21px;
		font-weight: 600;
		letter-spacing: -0.02em;
	}

	.page-header p {
		margin-top: 3px;
		font-size: 12.5px;
		color: var(--ink-faint);
	}

	.tabs {
		display: flex;
		gap: 22px;
		margin-top: 18px;
		border-bottom: 1px solid var(--line);
	}

	.tab {
		padding: 7px 0 10px;
		border: none;
		border-bottom: 2px solid transparent;
		border-radius: 0;
		background: transparent;
		color: var(--ink-faint);
		font-size: 12.5px;
		font-weight: 500;
	}

	.tab:not(:disabled):hover {
		border-color: transparent;
		border-bottom-color: var(--line-strong);
		background: transparent;
		color: var(--ink);
	}

	.tab[aria-selected='true'] {
		color: var(--ink);
		border-bottom-color: var(--accent);
	}

	.tab[aria-selected='true']:not(:disabled):hover {
		border-bottom-color: var(--accent);
	}

	.loading {
		color: var(--ink-faint);
		font-size: 12px;
	}

	.group {
		padding: 26px 0;
	}

	.label {
		font-weight: 500;
		font-size: 12.5px;
	}

	.choice {
		display: flex;
		align-items: flex-start;
		gap: 9px;
		font-size: 12.5px;
		cursor: pointer;
	}

	.choice + .choice {
		margin-top: 10px;
	}

	.choice small {
		display: block;
		margin-top: 1px;
		color: var(--ink-faint);
		font-size: 11.5px;
		line-height: 1.5;
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
		color: var(--ink-faint);
		font-size: 12px;
	}

	.field button {
		width: fit-content;
		margin-top: 10px;
	}

	.savebar {
		position: sticky;
		bottom: 0;
		z-index: 1;
		display: flex;
		justify-content: flex-end;
		align-items: center;
		gap: 10px;
		margin-top: auto;
		padding: 14px 0;
		background: rgba(15, 20, 28, 0.92);
		backdrop-filter: blur(10px);
		border-top: 1px solid var(--line);
	}

	.savebar .error,
	.confirm {
		font-size: 12px;
		margin-right: auto;
	}

	.confirm {
		color: var(--ok);
	}

	.primary {
		background: var(--accent);
		border-color: var(--accent);
		color: #fff;
		font-weight: 600;
		padding: 7px 16px;
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
			padding: 24px 20px;
		}
	}
</style>
