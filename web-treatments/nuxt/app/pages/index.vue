<script setup lang="ts">
import type { App, Pack } from '~/composables/useApi'

const api = useApi()

const packs = ref<Pack[]>([])
const apps = ref<App[]>([])
const name = ref('')
const packId = ref('')
const description = ref('')
const submitting = ref(false)
const error = ref<string | null>(null)

async function load() {
  ;[packs.value, apps.value] = await Promise.all([api.listPacks(), api.listApps()])
  if (!packId.value && packs.value.length) packId.value = packs.value[0]?.id ?? ''
}

onMounted(load)

async function onSubmit() {
  error.value = null
  submitting.value = true
  try {
    await api.createApp({ name: name.value || undefined, pack: packId.value, prompt: description.value })
    name.value = ''
    description.value = ''
    await load()
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <h1>Treatment Builder</h1>
  <p class="hint">Nuxt treatment — Pareto screens against the live control-plane API.</p>

  <form class="card" @submit.prevent="onSubmit">
    <h2>New app</h2>

    <label>
      Name
      <input v-model="name" type="text" placeholder="optional — defaults to pack name" />
    </label>

    <label>
      Pack
      <select v-model="packId" required>
        <option v-for="pack in packs" :key="pack.id" :value="pack.id">{{ pack.name }}</option>
      </select>
    </label>

    <label>
      Description
      <textarea v-model="description" rows="3" required placeholder="what should this app do?"></textarea>
    </label>

    <p v-if="error" class="error">{{ error }}</p>

    <button type="submit" :disabled="submitting || !packId">
      {{ submitting ? 'Creating…' : 'Create app' }}
    </button>
  </form>

  <h2>Apps</h2>
  <p v-if="apps.length === 0" class="hint">No apps yet — create one above.</p>
  <ul v-else class="apps">
    <li v-for="app in apps" :key="app.id" class="card">
      <NuxtLink :to="`/apps/${app.id}`">{{ app.name }}</NuxtLink>
      <span class="stage">{{ app.stage }}</span>
    </li>
  </ul>
</template>

<style scoped>
form {
  display: flex;
  flex-direction: column;
  gap: var(--st-space-3);
  margin-bottom: var(--st-space-4);
}

label {
  display: flex;
  flex-direction: column;
  gap: var(--st-space-1);
  font-size: 0.9rem;
  color: var(--st-muted);
}

.apps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--st-space-2);
}

.apps li {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.stage {
  font-size: 0.8rem;
  color: var(--st-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
</style>
