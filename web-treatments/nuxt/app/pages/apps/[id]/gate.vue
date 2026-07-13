<script setup lang="ts">
import type { GateReport } from '~/composables/useApi'

const route = useRoute()
const id = route.params.id as string

const api = useApi()
const report = ref<GateReport | null>(null)
const fixing = ref<string | null>(null)
const error = ref<string | null>(null)

async function load() {
  report.value = await api.gateReport(id)
}

onMounted(load)

async function onFix(gateId: string) {
  error.value = null
  fixing.value = gateId
  try {
    report.value = await api.fixGate(id, gateId)
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err)
  } finally {
    fixing.value = null
  }
}
</script>

<template>
  <p><NuxtLink :to="`/apps/${id}`">&larr; App</NuxtLink></p>

  <h1>Gate report</h1>

  <template v-if="report">
    <p class="hint">
      {{ report.passed }}/{{ report.total }} passed &middot; {{ report.stubbed }} stubbed &middot;
      {{ report.green ? 'green' : 'not green' }}
    </p>

    <p v-if="error" class="error">{{ error }}</p>

    <ul class="checks">
      <li
        v-for="check in report.results"
        :key="check.id"
        class="card"
        :class="{ pass: check.status === 'pass', fail: check.status === 'fail' }"
      >
        <div class="row">
          <strong>{{ check.title }}</strong>
          <span class="status">{{ check.status }}</span>
        </div>
        <p v-if="check.reason" class="reason">{{ check.reason }}</p>
        <button
          v-if="check.status === 'fail' && check.fixable"
          :disabled="fixing === check.id"
          @click="onFix(check.id)"
        >
          {{ fixing === check.id ? 'Fixing…' : 'Fix' }}
        </button>
      </li>
    </ul>
  </template>
  <p v-else class="hint">Loading…</p>
</template>

<style scoped>
.checks {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--st-space-2);
}

.row {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.status {
  text-transform: uppercase;
  font-size: 0.8rem;
  letter-spacing: 0.04em;
}

.reason {
  color: var(--st-muted);
  font-size: 0.9rem;
}

.card.pass .status {
  color: var(--st-success);
}

.card.fail .status {
  color: var(--st-danger);
}

.card.pass {
  border-color: var(--st-success);
}

.card.fail {
  border-color: var(--st-danger);
}
</style>
