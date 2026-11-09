<script setup lang="ts">
import { computed } from 'vue'
import type { RunStatus } from '@/types'

const props = defineProps<{ status: RunStatus }>()

const MAP: Record<RunStatus, { cls: string; text: string }> = {
  running: { cls: 'ok live', text: 'Running' },
  no_changes: { cls: 'hollow', text: 'No changes' },
  up_to_date: { cls: '', text: 'Nothing to optimize' },
  awaiting_approval: { cls: 'warn', text: 'Awaiting approval' },
  pr_opened: { cls: 'ok', text: 'PR opened' },
  pr_updated: { cls: 'ok', text: 'PR updated' },
  error: { cls: 'danger', text: 'Error' },
}

const view = computed(() => MAP[props.status] ?? { cls: '', text: props.status })
</script>

<template>
  <span class="tag" :class="view.cls">{{ view.text }}</span>
</template>
