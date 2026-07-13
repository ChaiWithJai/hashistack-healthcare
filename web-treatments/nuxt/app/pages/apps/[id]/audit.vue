<script setup lang="ts">
import type { AuditEntry } from '~/composables/useApi'

const route = useRoute()
const id = route.params.id as string

const api = useApi()
const entries = ref<AuditEntry[]>([])

onMounted(async () => {
  const events = await api.audit(id)
  entries.value = [...events].sort((a, b) => b.seq - a.seq)
})

function formatAt(at: number): string {
  return new Date(at * 1000).toISOString()
}
</script>

<template>
  <p><NuxtLink :to="`/apps/${id}`">&larr; App</NuxtLink></p>

  <h1>Audit trail</h1>

  <p v-if="entries.length === 0" class="hint">Loading…</p>
  <ul v-else class="entries">
    <li v-for="entry in entries" :key="entry.seq" class="card">
      <div class="row">
        <strong>{{ entry.action }}</strong>
        <span class="at">{{ formatAt(entry.at) }}</span>
      </div>
      <p class="actor">by {{ entry.actor }}</p>
      <p class="detail">{{ entry.detail }}</p>
    </li>
  </ul>
</template>

<style scoped>
.entries {
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
  align-items: baseline;
}

.at {
  font-size: 0.8rem;
  color: var(--st-muted);
}

.actor {
  font-size: 0.85rem;
  color: var(--st-muted);
  margin: var(--st-space-1) 0;
}

.detail {
  margin: 0;
}
</style>
