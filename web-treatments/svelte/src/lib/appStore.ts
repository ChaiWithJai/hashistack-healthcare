import { writable } from 'svelte/store';
import { api, type App } from './api';

export function pollApp(id: string, intervalMs = 2000) {
	const store = writable<App | null>(null);
	let timer: ReturnType<typeof setInterval>;
	async function tick() {
		store.set(await api.getApp(id));
	}
	tick();
	timer = setInterval(tick, intervalMs);
	return { subscribe: store.subscribe, stop: () => clearInterval(timer) };
}
