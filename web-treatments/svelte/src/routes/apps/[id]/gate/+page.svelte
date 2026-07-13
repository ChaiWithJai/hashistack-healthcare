<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { api, type GateReport } from '$lib/api';

	const id = page.params.id as string;
	let report = $state<GateReport | null>(null);
	let fixing = $state<string | null>(null);
	let error = $state<string | null>(null);

	onMount(load);

	async function load() {
		report = await api.gateReport(id);
	}

	async function onFix(gateId: string) {
		error = null;
		fixing = gateId;
		try {
			report = await api.fixGate(id, gateId);
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			fixing = null;
		}
	}
</script>

<p><a href={`/apps/${id}`}>&larr; App</a></p>

<h1>Gate report</h1>

{#if report}
	<p class="hint">
		{report.passed}/{report.total} passed &middot; {report.stubbed} stubbed &middot;
		{report.green ? 'green' : 'not green'}
	</p>

	{#if error}
		<p class="error">{error}</p>
	{/if}

	<ul class="checks">
		{#each report.results as check (check.id)}
			<li class="card" class:pass={check.status === 'pass'} class:fail={check.status === 'fail'}>
				<div class="row">
					<strong>{check.title}</strong>
					<span class="status">{check.status}</span>
				</div>
				{#if check.reason}
					<p class="reason">{check.reason}</p>
				{/if}
				{#if check.status === 'fail' && check.fixable}
					<button onclick={() => onFix(check.id)} disabled={fixing === check.id}>
						{fixing === check.id ? 'Fixing…' : 'Fix'}
					</button>
				{/if}
			</li>
		{/each}
	</ul>
{:else}
	<p class="hint">Loading…</p>
{/if}

<style>
	.hint {
		color: var(--st-muted);
	}

	.error {
		color: var(--st-danger);
	}

	.checks {
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
		align-items: center;
	}

	.status {
		text-transform: uppercase;
		font-size: 0.8rem;
		letter-spacing: 0.04em;
	}

	.reason {
		color: var(--st-muted);
		font-size: 0.9rem;
	}

	.card.pass .status {
		color: var(--st-success);
	}

	.card.fail .status {
		color: var(--st-danger);
	}

	.card.pass {
		border-color: var(--st-success);
	}

	.card.fail {
		border-color: var(--st-danger);
	}
</style>
