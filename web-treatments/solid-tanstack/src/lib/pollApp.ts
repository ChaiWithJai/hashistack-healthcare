import { createSignal, onCleanup } from 'solid-js';
import { api, type App } from './api';

// Polls getApp() on an interval so the Workflow rail picks up stage
// transitions (sandbox -> live) driven by server-side actions (promote,
// rollback, iterate) without a manual refresh. Mirrors Task 2's
// (SvelteKit) appStore.ts and Task 3's (Nuxt) useStages.ts polling
// primitive, expressed as Solid's fine-grained createSignal instead of a
// Svelte store or Vue ref.
export function pollApp(id: string, intervalMs = 2000) {
	const [app, setApp] = createSignal<App | null>(null);

	async function tick() {
		setApp(await api.getApp(id));
	}

	tick();
	const timer = setInterval(tick, intervalMs);
	onCleanup(() => clearInterval(timer));

	return app;
}
