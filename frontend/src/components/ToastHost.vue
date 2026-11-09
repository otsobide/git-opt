<script setup lang="ts">
import { useToast } from '@/composables/useToast'

const { toasts, dismiss } = useToast()
</script>

<template>
  <div class="toasts" aria-live="polite">
    <div v-for="t in toasts" :key="t.id" class="toast" :class="t.kind" role="status">
      <span>{{ t.text }}</span>
      <button type="button" class="close" aria-label="Dismiss" @click="dismiss(t.id)">×</button>
    </div>
  </div>
</template>

<style scoped>
.toasts {
  position: fixed;
  right: 1rem;
  bottom: 1rem;
  z-index: 50;
  display: grid;
  gap: 0.5rem;
  width: min(24rem, calc(100vw - 2rem));
}
.toast {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.65rem 0.8rem 0.65rem 1rem;
  background: var(--ink);
  color: var(--surface);
  border-radius: var(--radius-sm);
  border-left: 4px solid var(--accent);
  font-size: var(--fs-sm);
  box-shadow: 0 6px 24px rgb(0 0 0 / 0.18);
}
.toast.error {
  border-left-color: var(--danger);
}
.close {
  border: 0;
  background: none;
  color: inherit;
  opacity: 0.7;
  font-size: 1.1rem;
  line-height: 1;
  cursor: pointer;
}
.close:hover {
  opacity: 1;
}
</style>
