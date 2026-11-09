<script setup lang="ts">
import { computed } from 'vue'
import type { RunStatus } from '@/types'

const props = defineProps<{ status: RunStatus }>()

const MAP: Record<RunStatus, { cls: string; text: string }> = {
  running: { cls: 'ok live', text: 'En curso' },
  no_changes: { cls: 'hollow', text: 'Sin cambios' },
  up_to_date: { cls: '', text: 'Nada que optimizar' },
  awaiting_approval: { cls: 'warn', text: 'Esperando aprobación' },
  pr_opened: { cls: 'ok', text: 'PR abierta' },
  pr_updated: { cls: 'ok', text: 'PR actualizada' },
  error: { cls: 'danger', text: 'Error' },
}

const view = computed(() => MAP[props.status] ?? { cls: '', text: props.status })
</script>

<template>
  <span class="tag" :class="view.cls">{{ view.text }}</span>
</template>
