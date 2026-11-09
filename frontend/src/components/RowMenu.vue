<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'

// Compact "more actions" menu. Closes on outside click or Escape.
defineProps<{ label?: string }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)

function onDocClick(e: MouseEvent) {
  if (root.value && !root.value.contains(e.target as Node)) open.value = false
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') open.value = false
}

watch(open, (v) => {
  if (v) {
    document.addEventListener('click', onDocClick)
    document.addEventListener('keydown', onKey)
  } else {
    document.removeEventListener('click', onDocClick)
    document.removeEventListener('keydown', onKey)
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('click', onDocClick)
  document.removeEventListener('keydown', onKey)
})
</script>

<template>
  <div ref="root" class="rowmenu" @click.stop>
    <button
      type="button"
      class="btn btn-quiet btn-sm"
      :aria-expanded="open"
      :aria-label="label || 'Más acciones'"
      @click="open = !open"
    >
      ⋯
    </button>
    <div v-if="open" class="menu" role="menu" @click="open = false">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.rowmenu {
  position: relative;
  display: inline-block;
}
.menu {
  position: absolute;
  right: 0;
  top: calc(100% + 0.25rem);
  z-index: 20;
  min-width: 12rem;
  display: grid;
  padding: 0.3rem;
  background: var(--surface);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-md);
  box-shadow: 0 10px 30px rgb(0 0 0 / 0.14);
  text-align: left;
}
.menu :deep(button),
.menu :deep(a) {
  display: block;
  width: 100%;
  padding: 0.4rem 0.6rem;
  border: 0;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--ink);
  font: inherit;
  font-size: var(--fs-sm);
  text-align: left;
  text-decoration: none;
  cursor: pointer;
}
.menu :deep(button:hover),
.menu :deep(a:hover) {
  background: var(--surface-2);
}
.menu :deep(.danger) {
  color: var(--danger);
}
.menu :deep(hr) {
  border: 0;
  border-top: 1px solid var(--line);
  margin: 0.25rem 0;
}
</style>
