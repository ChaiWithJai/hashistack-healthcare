import type { App } from './useApi'

// Polls GET /apps/:id every `intervalMs` and exposes the latest value as a
// ref. The fetch is kicked off from onMounted (not eagerly at setup-script
// top level) so it only ever runs client-side after hydration — onMounted
// never fires during Nuxt's server render, so this composable needs no
// extra `import.meta.client` / <ClientOnly> guard the way Task 2's
// SvelteKit treatment needed a `browser` check (that bug came from an
// *eager* top-level fetch, not from onMount). See docs/treatments/ui-nuxt.md
// for the full comparison.
export function usePollApp(id: string, intervalMs = 2000) {
  const api = useApi()
  const app = ref<App | null>(null)
  let timer: ReturnType<typeof setInterval> | undefined

  async function tick() {
    app.value = await api.getApp(id)
  }

  onMounted(() => {
    tick()
    timer = setInterval(tick, intervalMs)
  })

  onUnmounted(() => {
    if (timer) clearInterval(timer)
  })

  return app
}
