<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'

// Two-step destructive button: the first click arms it, the second confirms.
// Avoids browser confirm() dialogs.
withDefaults(
  defineProps<{
    label: string
    confirmLabel?: string
    disabled?: boolean
    small?: boolean
  }>(),
  { confirmLabel: 'Sure? Confirm', disabled: false, small: false },
)
const emit = defineEmits<{ confirm: [] }>()

const armed = ref(false)
let timer: ReturnType<typeof setTimeout> | null = null

function click() {
  if (armed.value) {
    armed.value = false
    if (timer) clearTimeout(timer)
    emit('confirm')
    return
  }
  armed.value = true
  timer = setTimeout(() => (armed.value = false), 4000)
}

onBeforeUnmount(() => {
  if (timer) clearTimeout(timer)
})
</script>

<template>
  <button
    type="button"
    class="btn btn-danger"
    :class="{ 'btn-sm': small, armed }"
    :disabled="disabled"
    @click="click"
    @blur="armed = false"
  >
    {{ armed ? confirmLabel : label }}
  </button>
</template>

<style scoped>
.armed {
  background: var(--danger);
  border-color: var(--danger);
  color: var(--surface);
}
.armed:hover:not(:disabled) {
  color: var(--surface);
}
</style>
