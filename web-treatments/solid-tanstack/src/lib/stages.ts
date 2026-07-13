import type { App } from './api';

// The brief's Workflow rail asks for 8 named stages highlighted from
// `app.state`. The real API models app lifecycle with a 2-value
// `stage: "sandbox" | "live"` field, not an 8-value one (verified by
// reading src/state.rs — see docs/treatments/ui-svelte.md). This maps the
// real 2-value lifecycle onto the brief's 8 display labels: "sandbox" apps
// are still in the describe→iterate build loop (highlight "iterate"),
// "live" apps have cleared gate/deploy and are running (highlight
// "operate"). It's a display heuristic, not a field the API tracks.
// Same mapping as Tasks 2 (Svelte) and 3 (Nuxt) so Task 5's comparison of
// the Workflow rail is apples-to-apples across all three treatments.
export const STAGES = [
	'describe',
	'generate',
	'preview',
	'iterate',
	'gate',
	'deploy',
	'operate',
	'audit'
] as const;

export function currentStageIndex(app: App): number {
	return app.stage === 'live' ? 6 : 3;
}
