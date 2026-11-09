<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'

// Compact "more actions" menu. Positioned as `fixed` from the trigger so
// scrolling table containers cannot clip it. Closes on outside click,
// Escape, scroll or resize.
defineProps<{ label?: string }>()

const open = ref(false)
const root = ref<HTMLElement | null>(null)
const trigger = ref<HTMLButtonElement | null>(null)
const menu = ref<HTMLElement | null>(null)
const style = ref<Record<string, string>>({})

const MARGIN = 8

async function place() {
  const t = trigger.value?.getBoundingClientRect()
  if (!t) return
  style.value = { top: `${t.bottom + 4}px`, left: `${t.left}px`, visibility: 'hidden' }
  await nextTick()
  const m = menu.value?.getBoundingClientRect()
  if (!m) return
  // Prefer right-aligning to the trigger; keep the menu inside the viewport.
  let left = t.right - m.width
  left = Math.min(Math.max(MARGIN, left), window.innerWidth - m.width - MARGIN)
  let top = t.bottom + 4
  if (top + m.height > window.innerHeight - MARGIN) top = Math.max(MARGIN, t.top - m.height - 4)
  style.value = { top: `${top}px`, left: `${left}px` }
}

function close() {
  open.value = false
}
function onDocClick(e: MouseEvent) {
  if (root.value && !root.value.contains(e.target as Node)) close()
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') {
    close()
    trigger.value?.focus()
  }
}

function listen(on: boolean) {
  const fn = on ? document.addEventListener.bind(document) : document.removeEventListener.bind(document)
  fn('click', onDocClick)
  fn('keydown', onKey)
  const w = on ? window.addEventListener.bind(window) : window.removeEventListener.bind(window)
  w('scroll', close, true)
  w('resize', close)
}

watch(open, (v) => {
  listen(v)
  if (v) void place()
})

onBeforeUnmount(() => listen(false))
</script>

<template>
  <div ref="root" class="rowmenu" @click.stop>
    <button
      ref="trigger"
      type="button"
      class="btn btn-quiet btn-sm"
      aria-haspopup="menu"
      :aria-expanded="open"
      :aria-label="label || 'More actions'"
      @click="open = !open"
    >
      ⋯
    </button>
    <div v-if="open" ref="menu" class="menu" role="menu" :style="style" @click="close">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.rowmenu {
  display: inline-block;
}
.menu {
  position: fixed;
  z-index: 40;
  min-width: 12rem;
  max-width: calc(100vw - 16px);
  display: grid;
  padding: 0.3rem;
  background: var(--surface);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-md);
  box-shadow: 0 10px 30px rgb(0 0 0 / 0.14);
  text-align: left;
  white-space: normal;
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
.menu :deep(button:hover:not(:disabled)),
.menu :deep(a:hover) {
  background: var(--surface-2);
}
.menu :deep(button:disabled) {
  color: var(--faint);
  cursor: not-allowed;
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
