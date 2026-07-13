import { createFileRoute, Link } from '@tanstack/solid-router';
import { createResource, createSignal, For, Show } from 'solid-js';
import { api } from '../lib/api';

export const Route = createFileRoute('/apps/$id_/gate')({
	component: GateReportScreen
});

function GateReportScreen() {
	const { id } = Route.useParams()();

	const [report, { refetch }] = createResource(() => api.gateReport(id));
	const [fixing, setFixing] = createSignal<string | null>(null);
	const [error, setError] = createSignal<string | null>(null);

	async function onFix(gateId: string) {
		setError(null);
		setFixing(gateId);
		try {
			await api.fixGate(id, gateId);
			await refetch();
		} catch (err) {
			setError(err instanceof Error ? err.message : String(err));
		} finally {
			setFixing(null);
		}
	}

	return (
		<>
			<p>
				<Link to="/apps/$id" params={{ id }}>
					&larr; App
				</Link>
			</p>

			<h1>Gate report</h1>

			<Show when={report()} fallback={<p class="hint">Loading…</p>}>
				{(current) => (
					<>
						<p class="hint">
							{current().passed}/{current().total} passed &middot; {current().stubbed} stubbed &middot;
							{current().green ? ' green' : ' not green'}
						</p>

						<Show when={error()}>
							<p class="error">{error()}</p>
						</Show>

						<ul class="checks">
							<For each={current().results}>
								{(check) => (
									<li class="card" classList={{ pass: check.status === 'pass', fail: check.status === 'fail' }}>
										<div class="row">
											<strong>{check.title}</strong>
											<span class="status">{check.status}</span>
										</div>
										<Show when={check.reason}>
											<p class="reason">{check.reason}</p>
										</Show>
										<Show when={check.status === 'fail' && check.fixable}>
											<button onClick={() => onFix(check.id)} disabled={fixing() === check.id}>
												{fixing() === check.id ? 'Fixing…' : 'Fix'}
											</button>
										</Show>
									</li>
								)}
							</For>
						</ul>
					</>
				)}
			</Show>

			<style>{`
				.checks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--st-space-2); }
				.row { display: flex; justify-content: space-between; align-items: center; }
				.status { text-transform: uppercase; font-size: 0.8rem; letter-spacing: 0.04em; }
				.reason { color: var(--st-muted); font-size: 0.9rem; }
				.card.pass .status { color: var(--st-success); }
				.card.fail .status { color: var(--st-danger); }
				.card.pass { border-color: var(--st-success); }
				.card.fail { border-color: var(--st-danger); }
			`}</style>
		</>
	);
}
