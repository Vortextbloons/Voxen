<script lang="ts">
	import { onMount } from 'svelte';
	import { isNative, getSnapshot, onDictationState } from './tauri';
	import { appState } from './state.svelte';
	import { errorMessage, phaseLabels } from './types';

	onMount(() => {
		if (!isNative) return;
		let unlisten: (() => void) | undefined;
		let disposed = false;
		(async () => {
			try {
				const fn = await onDictationState((snapshot) => appState.apply(snapshot));
				if (disposed) { fn(); return; }
				unlisten = fn;
				const snapshot = await getSnapshot();
				if (!disposed) appState.apply(snapshot);
			} catch (error) {
				if (!disposed) appState.loadError = errorMessage(error);
			}
		})();
		return () => {
			disposed = true;
			unlisten?.();
		};
	});

	const phase = $derived(appState.snapshot?.phase ?? 'idle');
</script>

<div class="indicator" data-phase={phase} role="status" aria-live="polite">
	<span class="dot" aria-hidden="true"></span>
	{appState.loadError ? 'Status unavailable' : phaseLabels[phase]}
</div>

<style>
	:global(body:has(.indicator)) {
		background: transparent;
	}

	.indicator {
		position: fixed;
		inset: 0;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 6px;
		background: var(--surface);
		border: 1px solid var(--line);
		border-radius: 8px;
		font-size: 12px;
		font-weight: 600;
		color: var(--ink);
		pointer-events: none;
		user-select: none;
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--ink-faint);
	}

	.indicator[data-phase='recording'] .dot {
		background: var(--danger);
		animation: pulse 1.6s ease-in-out infinite;
	}

	.indicator[data-phase='transcribing'] .dot,
	.indicator[data-phase='inserting'] .dot {
		background: var(--accent);
	}

	.indicator[data-phase='error'] .dot {
		background: var(--danger);
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
</style>
