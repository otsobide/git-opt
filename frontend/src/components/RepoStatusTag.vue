<script setup lang="ts">
import { computed } from 'vue'
import type { RepoView } from '@/types'

const props = defineProps<{ repo: RepoView }>()

const view = computed(() => {
  const r = props.repo
  if (r.running) return { cls: 'ok live', text: 'Processing' }
  if (r.queued) return { cls: 'hollow', text: 'Queued' }
  if (!r.enabled) return { cls: 'hollow', text: 'Paused' }
  switch (r.status) {
    case 'ok':
      return { cls: 'ok', text: 'Up to date' }
    case 'awaiting_approval':
      return { cls: 'warn', text: 'Awaiting approval' }
    case 'error':
      return { cls: 'danger', text: 'Error' }
    default:
      return { cls: 'hollow', text: 'Pending' }
  }
})
</script>

<template>
  <span class="tag" :class="view.cls">{{ view.text }}</span>
</template>
