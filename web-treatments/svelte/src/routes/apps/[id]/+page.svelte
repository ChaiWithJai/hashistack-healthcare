<script lang="ts">
	import { onDestroy } from 'svelte';
	import { browser } from '$app/environment';
	import { page } from '$app/state';
	import { api } from '$lib/api';
	import { pollApp } from '$lib/appStore';
	import { STAGES, currentStageIndex } from '$lib/stages';

	const id = page.params.id as string;
	// pollApp() fires an immediate fetch — only safe in the browser. During
	// SSR (and the server-rendered pass of hydration) fall back to an inert
	// store so the eager `getApp` call never runs off-browser; see
	// docs/treatments/ui-svelte.md for the SSR-vs-onMount pitfall this
	// dodges (SvelteKit throws if you `fetch` a relative URL outside
	// `onMount`/`load`).
	const poll = browser
		? pollApp(id)
		: { subscribe: (fn: (v: null) => void) => { fn(null); return () => {}; }, stop: () => {} };
	const app = poll;

	let instruction = $state('');
	let iterating = $state(false);
	let promoting = $state(false);
	let rollingBack = $state(false);
	let error = $state<string | null>(null);

	onDestroy(() => poll.stop());

	async function onIterate(e: SubmitEvent) {
		e.preventDefault();
		if (!instruction.trim()) return;
		error = null;
		iterating = true;
		try {
			await api.iterate(id, instruction);
			instruction = '';
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			iterating = false;
		}
	}

	async function onPromote() {
		error = null;
		promoting = true;
		try {
			await api.promote(id, { synthetic_demo: true });
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			promoting = false;
		}
	}

	async function onRollback() {
		error = null;
		rollingBack = true;
		try {
			await api.rollback(id);
		} catch (err) {
			error = err instanceof Error ? err.message : String(err);
		} finally {
			rollingBack = false;
		}
	}
</script>

<p><a href="/">&larr; Builder</a></p>

{#if $app}
	<h1>{$app.name}</h1>
	<p class="hint">
		pack {$app.pack} &middot; stage {$app.stage} &middot; v{$app.current_version}
	</p>

	<nav class="rail" aria-label="Workflow stages">
		{#each STAGES as stage, i (stage)}
			<span class="stage" class:current={i === currentStageIndex($app)} class:done={i < currentStageIndex($app)}>
				{stage}
			</span>
		{/each}
	</nav>

	<div class="card">
		<h2>Iterate</h2>
		<form onsubmit={onIterate}>
			<textarea
				bind:value={instruction}
				rows="3"
				placeholder="describe the change you want"
			></textarea>
			<button type="submit" disabled={iterating || !instruction.trim()}>
				{iterating ? 'Applying…' : 'Submit instruction'}
			</button>
		</form>
	</div>

	<div class="actions">
		<button onclick={onPromote} disabled={promoting || $app.stage === 'live'}>
			{promoting ? 'Promoting…' : 'Promote'}
		</button>
		<button class="secondary" onclick={onRollback} disabled={rollingBack || $app.stage === 'sandbox'}>
			{rollingBack ? 'Rolling back…' : 'Rollback'}
		</button>
		<a class="link" href={`/apps/${id}/gate`}>Gate report</a>
		<a class="link" href={`/apps/${id}/audit`}>Audit trail</a>
	</div>

	{#if error}
		<p class="error">{error}</p>
	{/if}
{:else}
	<p class="hint">Loading…</p>
{/if}

<style>
	.hint {
		color: var(--st-muted);
	}

	.rail {
		display: flex;
		gap: var(--st-space-2);
		flex-wrap: wrap;
		margin: var(--st-space-4) 0;
	}

	.stage {
		padding: var(--st-space-1) var(--st-space-3);
		border-radius: var(--st-radius-control);
		border: 1px solid var(--st-line);
		background: var(--st-panel);
		color: var(--st-muted);
		font-size: 0.85rem;
		text-transform: capitalize;
	}

	.stage.done {
		background: var(--st-success-bg);
		color: var(--st-success);
		border-color: var(--st-success);
	}

	.stage.current {
		background: var(--st-brand);
		color: white;
		border-color: var(--st-brand);
		font-weight: 600;
	}

	form {
		display: flex;
		flex-direction: column;
		gap: var(--st-space-2);
	}

	.actions {
		display: flex;
		align-items: center;
		gap: var(--st-space-3);
		margin-top: var(--st-space-4);
	}

	.link {
		color: var(--st-brand-dark);
	}

	.error {
		color: var(--st-danger);
	}
</style>
