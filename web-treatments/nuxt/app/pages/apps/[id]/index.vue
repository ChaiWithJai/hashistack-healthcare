<script setup lang="ts">
import { STAGES, currentStageIndex } from '~/composables/useStages'

const route = useRoute()
const id = route.params.id as string

const api = useApi()
const app = usePollApp(id)

const instruction = ref('')
const iterating = ref(false)
const promoting = ref(false)
const rollingBack = ref(false)
const error = ref<string | null>(null)

async function onIterate() {
  if (!instruction.value.trim()) return
  error.value = null
  iterating.value = true
  try {
    await api.iterate(id, instruction.value)
    instruction.value = ''
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    iterating.value = false
  }
}

async function onPromote() {
  error.value = null
  promoting.value = true
  try {
    await api.promote(id, { synthetic_demo: true })
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    promoting.value = false
  }
}

async function onRollback() {
  error.value = null
  rollingBack.value = true
  try {
    await api.rollback(id)
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    rollingBack.value = false
  }
}
</script>

<template>
  <p><NuxtLink to="/">&larr; Builder</NuxtLink></p>

  <template v-if="app">
    <h1>{{ app.name }}</h1>
    <p class="hint">pack {{ app.pack }} &middot; stage {{ app.stage }} &middot; v{{ app.current_version }}</p>

    <nav class="rail" aria-label="Workflow stages">
      <span
        v-for="(stage, i) in STAGES"
        :key="stage"
        class="stage"
        :class="{ current: i === currentStageIndex(app), done: i < currentStageIndex(app) }"
      >
        {{ stage }}
      </span>
    </nav>

    <div class="card">
      <h2>Iterate</h2>
      <form @submit.prevent="onIterate">
        <textarea v-model="instruction" rows="3" placeholder="describe the change you want"></textarea>
        <button type="submit" :disabled="iterating || !instruction.trim()">
          {{ iterating ? 'Applying…' : 'Submit instruction' }}
        </button>
      </form>
    </div>

    <div class="actions">
      <button :disabled="promoting || app.stage === 'live'" @click="onPromote">
        {{ promoting ? 'Promoting…' : 'Promote' }}
      </button>
      <button class="secondary" :disabled="rollingBack || app.stage === 'sandbox'" @click="onRollback">
        {{ rollingBack ? 'Rolling back…' : 'Rollback' }}
      </button>
      <NuxtLink class="link" :to="`/apps/${id}/gate`">Gate report</NuxtLink>
      <NuxtLink class="link" :to="`/apps/${id}/audit`">Audit trail</NuxtLink>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
  </template>
  <p v-else class="hint">Loading…</p>
</template>

<style scoped>
.rail {
  display: flex;
  gap: var(--st-space-2);
  flex-wrap: wrap;
  margin: var(--st-space-4) 0;
}

.stage {
  padding: var(--st-space-1) var(--st-space-3);
  border-radius: var(--st-radius-control);
  border: 1px solid var(--st-line);
  background: var(--st-panel);
  color: var(--st-muted);
  font-size: 0.85rem;
  text-transform: capitalize;
}

.stage.done {
  background: var(--st-success-bg);
  color: var(--st-success);
  border-color: var(--st-success);
}

.stage.current {
  background: var(--st-brand);
  color: white;
  border-color: var(--st-brand);
  font-weight: 600;
}

form {
  display: flex;
  flex-direction: column;
  gap: var(--st-space-2);
}

.actions {
  display: flex;
  align-items: center;
  gap: var(--st-space-3);
  margin-top: var(--st-space-4);
}

.link {
  color: var(--st-brand-dark);
}
</style>
