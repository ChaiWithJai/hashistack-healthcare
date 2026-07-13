<script lang="ts">
	import { onMount } from 'svelte';
	import { api, type App, type Pack } from '$lib/api';

	let packs = $state<Pack[]>([]);
	let apps = $state<App[]>([]);
	let name = $state('');
	let packId = $state('');
	let description = $state('');
	let submitting = $state(false);
	let error = $state<string | null>(null);

	onMount(async () => {
		await load();
	});

	async function load() {
		[packs, apps] = await Promise.all([api.listPacks(), api.listApps()]);
		if (!packId && packs.length) packId = packs[0].id;
	}

	async function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		error = null;
		submitting = true;
		try {
			await api.createApp({ name: name || undefined, pack: packId, prompt: description });
			name = '';
			description = '';
			await load();
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			submitting = false;
		}
	}
</script>

<h1>Treatment Builder</h1>
<p class="hint">SvelteKit treatment — Pareto screens against the live control-plane API.</p>

<form class="card" onsubmit={onSubmit}>
	<h2>New app</h2>

	<label>
		Name
		<input type="text" bind:value={name} placeholder="optional — defaults to pack name" />
	</label>

	<label>
		Pack
		<select bind:value={packId} required>
			{#each packs as pack (pack.id)}
				<option value={pack.id}>{pack.name}</option>
			{/each}
		</select>
	</label>

	<label>
		Description
		<textarea bind:value={description} rows="3" required placeholder="what should this app do?"
		></textarea>
	</label>

	{#if error}
		<p class="error">{error}</p>
	{/if}

	<button type="submit" disabled={submitting || !packId}>
		{submitting ? 'Creating…' : 'Create app'}
	</button>
</form>

<h2>Apps</h2>
{#if apps.length === 0}
	<p class="hint">No apps yet — create one above.</p>
{:else}
	<ul class="apps">
		{#each apps as app (app.id)}
			<li class="card">
				<a href={`/apps/${app.id}`}>{app.name}</a>
				<span class="stage">{app.stage}</span>
			</li>
		{/each}
	</ul>
{/if}

<style>
	form {
		display: flex;
		flex-direction: column;
		gap: var(--st-space-3);
		margin-bottom: var(--st-space-4);
	}

	label {
		display: flex;
		flex-direction: column;
		gap: var(--st-space-1);
		font-size: 0.9rem;
		color: var(--st-muted);
	}

	.hint {
		color: var(--st-muted);
	}

	.error {
		color: var(--st-danger);
	}

	.apps {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: var(--st-space-2);
	}

	.apps li {
		display: flex;
		justify-content: space-between;
		align-items: center;
	}

	.stage {
		font-size: 0.8rem;
		color: var(--st-muted);
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}
</style>
