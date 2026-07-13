import { createFileRoute, Link } from '@tanstack/solid-router';
import { createSignal, For, Show } from 'solid-js';
import { api } from '../lib/api';
import { pollApp } from '../lib/pollApp';
import { STAGES, currentStageIndex } from '../lib/stages';

export const Route = createFileRoute('/apps/$id')({
	component: WorkflowRail
});

function WorkflowRail() {
	const { id } = Route.useParams()();

	// pollApp() fires an immediate fetch and re-polls every 2s so the rail
	// picks up stage transitions (sandbox -> live) driven by promote /
	// rollback / iterate without a manual refresh. Unlike Task 2's
	// (SvelteKit) equivalent, there's no SSR-vs-onMount pitfall to dodge
	// here: this is a plain client-rendered Vite SPA, so the eager fetch
	// inside pollApp() is always safe to run at component-init time.
	const app = pollApp(id);

	const [instruction, setInstruction] = createSignal('');
	const [iterating, setIterating] = createSignal(false);
	const [promoting, setPromoting] = createSignal(false);
	const [rollingBack, setRollingBack] = createSignal(false);
	const [error, setError] = createSignal<string | null>(null);

	async function onIterate(e: SubmitEvent) {
		e.preventDefault();
		if (!instruction().trim()) return;
		setError(null);
		setIterating(true);
		try {
			await api.iterate(id, instruction());
			setInstruction('');
		} catch (err) {
			setError(err instanceof Error ? err.message : String(err));
		} finally {
			setIterating(false);
		}
	}

	async function onPromote() {
		setError(null);
		setPromoting(true);
		try {
			await api.promote(id, { synthetic_demo: true });
		} catch (err) {
			setError(err instanceof Error ? err.message : String(err));
		} finally {
			setPromoting(false);
		}
	}

	async function onRollback() {
		setError(null);
		setRollingBack(true);
		try {
			await api.rollback(id);
		} catch (err) {
			setError(err instanceof Error ? err.message : String(err));
		} finally {
			setRollingBack(false);
		}
	}

	return (
		<>
			<p>
				<Link to="/">&larr; Builder</Link>
			</p>

			<Show when={app()} fallback={<p class="hint">Loading…</p>}>
				{(current) => (
					<>
						<h1>{current().name}</h1>
						<p class="hint">
							pack {current().pack} &middot; stage {current().stage} &middot; v{current().current_version}
						</p>

						<nav class="rail" aria-label="Workflow stages">
							<For each={STAGES}>
								{(stage, i) => (
									<span
										class="stage"
										classList={{
											current: i() === currentStageIndex(current()),
											done: i() < currentStageIndex(current())
										}}
									>
										{stage}
									</span>
								)}
							</For>
						</nav>

						<div class="card">
							<h2>Iterate</h2>
							<form onSubmit={onIterate}>
								<textarea
									rows="3"
									placeholder="describe the change you want"
									value={instruction()}
									onInput={(e) => setInstruction(e.currentTarget.value)}
								/>
								<button type="submit" disabled={iterating() || !instruction().trim()}>
									{iterating() ? 'Applying…' : 'Submit instruction'}
								</button>
							</form>
						</div>

						<div class="actions">
							<button onClick={onPromote} disabled={promoting() || current().stage === 'live'}>
								{promoting() ? 'Promoting…' : 'Promote'}
							</button>
							<button
								class="secondary"
								onClick={onRollback}
								disabled={rollingBack() || current().stage === 'sandbox'}
							>
								{rollingBack() ? 'Rolling back…' : 'Rollback'}
							</button>
							<Link class="link" to="/apps/$id/gate" params={{ id }}>
								Gate report
							</Link>
							<Link class="link" to="/apps/$id/audit" params={{ id }}>
								Audit trail
							</Link>
						</div>

						<Show when={error()}>
							<p class="error">{error()}</p>
						</Show>
					</>
				)}
			</Show>

			<style>{`
				.rail { display: flex; gap: var(--st-space-2); flex-wrap: wrap; margin: var(--st-space-4) 0; }
				.stage { padding: var(--st-space-1) var(--st-space-3); border-radius: var(--st-radius-control); border: 1px solid var(--st-line); background: var(--st-panel); color: var(--st-muted); font-size: 0.85rem; text-transform: capitalize; }
				.stage.done { background: var(--st-success-bg); color: var(--st-success); border-color: var(--st-success); }
				.stage.current { background: var(--st-brand); color: white; border-color: var(--st-brand); font-weight: 600; }
				form { display: flex; flex-direction: column; gap: var(--st-space-2); }
				.actions { display: flex; align-items: center; gap: var(--st-space-3); margin-top: var(--st-space-4); }
				.link { color: var(--st-brand-dark); }
			`}</style>
		</>
	);
}
