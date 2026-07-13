// Client for the Rust control-plane API (src/api.rs in the main repo).
//
// NOTE on divergence from the task brief's illustrative snippet: the brief's
// inline api.ts sketched an idealized shape (App.pack_id, App.state as one
// of 8 stage names, GateReport.checks[].passed, AuditEntry.id/kind). The
// real, running API is different — verified by reading src/api.rs,
// src/state.rs, src/gates.rs, src/audit.rs, src/packs.rs directly:
//   - AppRecord has `pack` (not pack_id) and `stage: "sandbox" | "live"`
//     (not an 8-value `state`) — see docs/treatments/ui-svelte.md for the
//     full reasoning and how the Workflow rail maps the 2-value lifecycle
//     onto the brief's 8 display stages.
//   - GateReport.results[] carries a flattened `status: "pass" | "stubbed" |
//     "fail"` (plus `reason`/`fixable` on non-pass), not `checks[].passed`.
//   - Audit events are `{ seq, at, actor, action, detail, app_id, sensitive }`,
//     not `{ id, kind, at, detail }`.
// Function names below match the brief exactly (listPacks, listApps,
// createApp, getApp, iterate, gateReport, fixGate, promote, rollback,
// audit) since Tasks 3-4 (Nuxt, Solid) implement the identical contract
// independently and Task 5 compares all three treatments on it.

// Dev-only bearer token. Never hardcode this outside a gitignored .env —
// copy .env.example to .env to set VITE_DEV_TOKEN locally.
const TOKEN = import.meta.env.VITE_DEV_TOKEN;
if (!TOKEN) {
	throw new Error('VITE_DEV_TOKEN is not set. Copy .env.example to .env and set it.');
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
	const res = await fetch(`/api${path}`, {
		...init,
		headers: {
			Authorization: `Bearer ${TOKEN}`,
			'Content-Type': 'application/json',
			...(init?.headers ?? {})
		}
	});
	if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
	return res.status === 204 ? (undefined as T) : res.json();
}

export interface Pack {
	id: string;
	name: string;
	description: string;
	profile: string;
	tier: number;
	wave: number;
}

export type Stage = 'sandbox' | 'live';

export interface App {
	id: string;
	name: string;
	prompt: string;
	pack: string;
	stage: Stage;
	data_source: string;
	controls: string[];
	external_calls: string[];
	features: string[];
	routes: number;
	current_version: number;
	reviewer_note: string | null;
	tenant: string;
}

export type GateStatus = 'pass' | 'stubbed' | 'fail';

export interface GateResult {
	id: string;
	title: string;
	basis: 'control' | 'evidence';
	citation?: string | null;
	status: GateStatus;
	reason?: string;
	fixable?: boolean;
}

export interface GateReport {
	app_id: string;
	app_version: number;
	results: GateResult[];
	passed: number;
	stubbed: number;
	total: number;
	green: boolean;
}

export interface AuditEntry {
	seq: number;
	at: number;
	actor: string;
	action: string;
	detail: string;
	app_id: string | null;
	sensitive?: Record<string, string>;
}

export const api = {
	listPacks: () => request<{ packs: Pack[] }>('/packs').then((r) => r.packs),

	listApps: () => request<{ apps: App[] }>('/apps').then((r) => r.apps),

	createApp: (body: { name?: string; pack: string; prompt: string }) =>
		request<{ app: App; scaffold: unknown[] }>('/apps', {
			method: 'POST',
			body: JSON.stringify(body)
		}).then((r) => r.app),

	getApp: (id: string) => request<App>(`/apps/${id}`),

	iterate: (id: string, instruction: string) =>
		request<{ reply: string; app: App }>(`/apps/${id}/iterate`, {
			method: 'POST',
			body: JSON.stringify({ instruction })
		}).then((r) => r.app),

	gateReport: (id: string) =>
		request<{ report: GateReport; meter: Record<string, boolean>; reviewer_note: string | null }>(
			`/apps/${id}/gate`
		).then((r) => r.report),

	fixGate: (id: string, gateId: string) =>
		request<{ wired: string; already_wired: boolean; app: App }>(
			`/apps/${id}/gate/${gateId}/fix`,
			{ method: 'POST' }
		).then(() => api.gateReport(id)),

	promote: (id: string, opts: { synthetic_demo: boolean } = { synthetic_demo: true }) =>
		request<{ app: App; report: GateReport }>(`/apps/${id}/promote`, {
			method: 'POST',
			body: JSON.stringify(opts)
		}).then((r) => r.app),

	rollback: (id: string) => request<App>(`/apps/${id}/rollback`, { method: 'POST' }),

	audit: (id: string) => request<{ events: AuditEntry[] }>(`/apps/${id}/audit`).then((r) => r.events)
};
