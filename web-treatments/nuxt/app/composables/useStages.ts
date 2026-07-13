import type { App } from './useApi'

// The brief's Workflow rail asks for 8 named stages highlighted from
// `app.state`. The real API models app lifecycle with a 2-value
// `stage: "sandbox" | "live"` field, not an 8-value one (confirmed against
// src/state.rs by Task 2's SvelteKit treatment — see
// docs/treatments/ui-svelte.md). This maps the real 2-value lifecycle onto
// the brief's 8 display labels the same way Task 2's src/lib/stages.ts
// does, so the two treatments are apples-to-apples for Task 5: "sandbox"
// apps are still in the describe->iterate build loop (highlight
// "iterate", index 3), "live" apps have cleared gate/deploy and are
// running (highlight "operate", index 6). It's a display heuristic, not a
// field the API tracks.
export const STAGES = [
  'describe',
  'generate',
  'preview',
  'iterate',
  'gate',
  'deploy',
  'operate',
  'audit'
] as const

export function currentStageIndex(app: App): number {
  return app.stage === 'live' ? 6 : 3
}
