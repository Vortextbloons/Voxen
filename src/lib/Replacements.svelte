<script lang="ts">
	import type { Replacement } from './types';

	let { replacements = $bindable() }: { replacements: Replacement[] } = $props();

	function add() {
		replacements.push({ from: '', to: '' });
	}

	function remove(row: Replacement) {
		replacements.splice(replacements.indexOf(row), 1);
	}
</script>

<div class="rows">
	{#each replacements as row (row)}
		<div class="row">
			<label class="cell">
				<span>Replace</span>
				<input type="text" bind:value={row.from} />
			</label>
			<label class="cell">
				<span>With</span>
				<input type="text" bind:value={row.to} />
			</label>
			<button type="button" onclick={() => remove(row)}>Remove</button>
		</div>
	{:else}
		<p class="none">No replacements.</p>
	{/each}
</div>
<button type="button" onclick={add}>Add replacement</button>

<style>
	.rows {
		display: flex;
		flex-direction: column;
		gap: 8px;
		margin-bottom: 10px;
	}

	.row {
		display: grid;
		grid-template-columns: 1fr 1fr auto;
		gap: 8px;
		align-items: end;
	}

	.cell {
		display: flex;
		flex-direction: column;
		gap: 3px;
	}

	.cell > span {
		font-size: 11.5px;
		font-weight: 600;
		color: var(--ink-soft);
	}

	.none {
		color: var(--ink-soft);
		font-size: 12px;
	}

	@media (max-width: 480px) {
		.row {
			grid-template-columns: 1fr;
			align-items: stretch;
		}
	}
</style>