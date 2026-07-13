import { createFileRoute, Link } from '@tanstack/solid-router';
import { createResource, createSignal, For, Show } from 'solid-js';
import { api } from '../lib/api';

export const Route = createFileRoute('/')({
	component: Builder
});

function Builder() {
	// createResource is Solid's data-fetching primitive: fine-grained,
	// no dependency array, `refetch()` re-runs the fetcher and updates
	// only the DOM nodes that read the resource's value.
	const [packs] = createResource(() => api.listPacks());
	const [apps, { refetch: refetchApps }] = createResource(() => api.listApps());

	const [name, setName] = createSignal('');
	const [packId, setPackId] = createSignal('');
	const [description, setDescription] = createSignal('');
	const [submitting, setSubmitting] = createSignal(false);
	const [error, setError] = createSignal<string | null>(null);

	// Default the pack <select> to the first loaded pack, once.
	const effectivePackId = () => packId() || packs()?.[0]?.id || '';

	async function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		setError(null);
		setSubmitting(true);
		try {
			await api.createApp({
				name: name() || undefined,
				pack: effectivePackId(),
				prompt: description()
			});
			setName('');
			setDescription('');
			await refetchApps();
		} catch (err) {
			setError(err instanceof Error ? err.message : String(err));
		} finally {
			setSubmitting(false);
		}
	}

	return (
		<>
			<h1>Treatment Builder</h1>
			<p class="hint">Solid + TanStack Router treatment — Pareto screens against the live control-plane API.</p>

			<form class="card" onSubmit={onSubmit}>
				<h2>New app</h2>

				<label>
					Name
					<input
						type="text"
						value={name()}
						onInput={(e) => setName(e.currentTarget.value)}
						placeholder="optional — defaults to pack name"
					/>
				</label>

				<label>
					Pack
					<select
						value={effectivePackId()}
						onChange={(e) => setPackId(e.currentTarget.value)}
						required
					>
						<For each={packs()}>{(pack) => <option value={pack.id}>{pack.name}</option>}</For>
					</select>
				</label>

				<label>
					Description
					<textarea
						rows="3"
						required
						placeholder="what should this app do?"
						value={description()}
						onInput={(e) => setDescription(e.currentTarget.value)}
					/>
				</label>

				<Show when={error()}>
					<p class="error">{error()}</p>
				</Show>

				<button type="submit" disabled={submitting() || !effectivePackId()}>
					{submitting() ? 'Creating…' : 'Create app'}
				</button>
			</form>

			<h2>Apps</h2>
			<Show when={(apps() ?? []).length > 0} fallback={<p class="hint">No apps yet — create one above.</p>}>
				<ul class="apps">
					<For each={apps()}>
						{(app) => (
							<li class="card">
								<Link to="/apps/$id" params={{ id: app.id }}>
									{app.name}
								</Link>
								<span class="stage">{app.stage}</span>
							</li>
						)}
					</For>
				</ul>
			</Show>

			<style>{`
				form { display: flex; flex-direction: column; gap: var(--st-space-3); margin-bottom: var(--st-space-4); }
				label { display: flex; flex-direction: column; gap: var(--st-space-1); font-size: 0.9rem; color: var(--st-muted); }
				.apps { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--st-space-2); }
				.apps li { display: flex; justify-content: space-between; align-items: center; }
				.stage { font-size: 0.8rem; color: var(--st-muted); text-transform: uppercase; letter-spacing: 0.04em; }
			`}</style>
		</>
	);
}
