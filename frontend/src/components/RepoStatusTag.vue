<script setup lang="ts">
import { computed } from 'vue'
import type { RepoView } from '@/types'

const props = defineProps<{ repo: RepoView }>()

const view = computed(() => {
  const r = props.repo
  if (r.running) return { cls: 'ok live', text: 'Procesando' }
  if (r.queued) return { cls: 'hollow', text: 'En cola' }
  if (!r.enabled) return { cls: 'hollow', text: 'En pausa' }
  switch (r.status) {
    case 'ok':
      return { cls: 'ok', text: 'Al día' }
    case 'awaiting_approval':
      return { cls: 'warn', text: 'Esperando aprobación' }
    case 'error':
      return { cls: 'danger', text: 'Error' }
    default:
      return { cls: 'hollow', text: 'Pendiente' }
  }
})
</script>

<template>
  <span class="tag" :class="view.cls">{{ view.text }}</span>
</template>
