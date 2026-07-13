import { createFileRoute, Link } from '@tanstack/solid-router';
import { createResource, For, Show } from 'solid-js';
import { api } from '../lib/api';

export const Route = createFileRoute('/apps/$id_/audit')({
	component: AuditTrail
});

function formatAt(at: number): string {
	return new Date(at * 1000).toISOString();
}

function AuditTrail() {
	const { id } = Route.useParams()();

	const [entries] = createResource(() => api.audit(id), {
		initialValue: []
	});
	const sorted = () => [...(entries() ?? [])].sort((a, b) => b.seq - a.seq);

	return (
		<>
			<p>
				<Link to="/apps/$id" params={{ id }}>
					&larr; App
				</Link>
			</p>

			<h1>Audit trail</h1>

			<Show when={sorted().length > 0} fallback={<p class="hint">Loading…</p>}>
				<ul class="entries">
					<For each={sorted()}>
						{(entry) => (
							<li class="card">
								<div class="row">
									<strong>{entry.action}</strong>
									<span class="at">{formatAt(entry.at)}</span>
								</div>
								<p class="actor">by {entry.actor}</p>
								<p class="detail">{entry.detail}</p>
							</li>
						)}
					</For>
				</ul>
			</Show>

			<style>{`
				.entries { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--st-space-2); }
				.row { display: flex; justify-content: space-between; align-items: baseline; }
				.at { font-size: 0.8rem; color: var(--st-muted); }
				.actor { font-size: 0.85rem; color: var(--st-muted); margin: var(--st-space-1) 0; }
				.detail { margin: 0; }
			`}</style>
		</>
	);
}
