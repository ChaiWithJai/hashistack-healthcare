<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { api, type AuditEntry } from '$lib/api';

	const id = page.params.id as string;
	let entries = $state<AuditEntry[]>([]);

	onMount(async () => {
		const events = await api.audit(id);
		entries = [...events].sort((a, b) => b.seq - a.seq);
	});

	function formatAt(at: number): string {
		return new Date(at * 1000).toISOString();
	}
</script>

<p><a href={`/apps/${id}`}>&larr; App</a></p>

<h1>Audit trail</h1>

{#if entries.length === 0}
	<p class="hint">Loading…</p>
{:else}
	<ul class="entries">
		{#each entries as entry (entry.seq)}
			<li class="card">
				<div class="row">
					<strong>{entry.action}</strong>
					<span class="at">{formatAt(entry.at)}</span>
				</div>
				<p class="actor">by {entry.actor}</p>
				<p class="detail">{entry.detail}</p>
			</li>
		{/each}
	</ul>
{/if}

<style>
	.hint {
		color: var(--st-muted);
	}

	.entries {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: var(--st-space-2);
	}

	.row {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
	}

	.at {
		font-size: 0.8rem;
		color: var(--st-muted);
	}

	.actor {
		font-size: 0.85rem;
		color: var(--st-muted);
		margin: var(--st-space-1) 0;
	}

	.detail {
		margin: 0;
	}
</style>
