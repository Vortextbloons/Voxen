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
	<div class="microphone" aria-hidden="true">
		<span class="wave cyan short"></span>
		<span class="wave cyan tall"></span>
		<svg viewBox="0 0 80 108" fill="none">
			<defs>
				<linearGradient id="mic-fill" x2="0.6" y2="1">
					<stop stop-color="#ffffff" />
					<stop offset="1" stop-color="#b7caff" />
				</linearGradient>
			</defs>
			<rect x="23" y="4" width="34" height="64" rx="17" fill="url(#mic-fill)" />
			<path d="M10 38v15a30 30 0 0 0 60 0V38M40 83v17M29 100h22" stroke="url(#mic-fill)" stroke-width="8" stroke-linecap="round" />
		</svg>
		<span class="wave violet tall"></span>
		<span class="wave violet short"></span>
	</div>
	<div class="details">
		<div class="title">{appState.loadError ? 'Status unavailable' : phase === 'recording' ? 'Listening...' : phaseLabels[phase]}</div>
	</div>
</div>

<style>
	:global(body:has(.indicator)) {
		background: transparent;
		overflow: hidden;
	}

	.indicator {
		position: fixed;
		inset: 12px;
		display: flex;
		align-items: center;
		gap: 5px;
		padding: 5px 8px;
		border: 1px solid transparent;
		border-radius: 40px;
		background: linear-gradient(115deg, #14243e, #10152b 55%, #21163d) padding-box,
			linear-gradient(105deg, #4df4ff, #6f8fff 48%, #eb65ff) border-box;
		box-shadow: -5px 0 15px #00ceff66, 5px 0 15px #b43cff66, inset 0 0 18px #777dff18;
		color: #f6f7ff;
		pointer-events: none;
		user-select: none;
	}

	.microphone {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 2px;
		width: 40px;
		flex-shrink: 0;
	}

	svg {
		width: 20px;
		height: 28px;
		filter: drop-shadow(0 0 7px #a7bbff33);
	}

	.wave {
		width: 3px;
		border-radius: 8px;
		flex-shrink: 0;
	}

	.short { height: 8px; }
	.tall { height: 16px; }
	.cyan { background: #00cfff; box-shadow: 0 0 11px #00bfff99; }
	.violet { background: #b128ff; box-shadow: 0 0 11px #ae25ff99; }

	.details {
		align-self: stretch;
		display: flex;
		flex-direction: column;
		justify-content: center;
		gap: 2px;
		padding-left: 5px;
		border-left: 1px solid #7786c052;
		min-width: 0;
	}

	.title { font-size: 12px; font-weight: 650; line-height: 1.1; letter-spacing: -0.4px; }

	.indicator[data-phase='recording'] { animation: glow 2.4s ease-in-out infinite alternate; }
	.indicator[data-phase='recording'] .wave { animation: pulse 0.75s ease-in-out infinite alternate; }
	.indicator[data-phase='recording'] .short { animation-delay: -0.4s; }
	.indicator[data-phase='recording'] .violet { animation-delay: -0.2s; }
	.indicator[data-phase='error'] .title { color: var(--danger); }

	@keyframes glow { to { box-shadow: -3px 0 10px #00ceff33, 3px 0 10px #b43cff33, inset 0 0 12px #777dff10; } }
	@keyframes pulse { to { transform: scaleY(0.55); opacity: 0.7; } }
	@media (prefers-reduced-motion: reduce) {
		.indicator[data-phase='recording'],
		.indicator[data-phase='recording'] .wave { animation: none; }
	}
</style>

