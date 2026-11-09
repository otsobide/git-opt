<script setup lang="ts">
import { computed } from 'vue'
import type { PrState } from '@/types'

const props = defineProps<{
  number: number | null
  url: string | null
  state?: PrState | null
}>()

const view = computed(() => {
  switch (props.state) {
    case 'merged':
      return { cls: 'merged', text: 'merged' }
    case 'closed':
      return { cls: 'danger', text: 'closed' }
    case 'open':
      return { cls: 'ok', text: 'open' }
    default:
      return null
  }
})
</script>

<template>
  <span v-if="number" class="pr">
    <a v-if="url" :href="url" target="_blank" rel="noopener">#{{ number }}</a>
    <span v-else>#{{ number }}</span>
    <span v-if="view" class="tag" :class="view.cls">{{ view.text }}</span>
  </span>
  <span v-else class="muted">—</span>
</template>

<style scoped>
.pr {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  white-space: nowrap;
}
.pr a {
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
</style>
