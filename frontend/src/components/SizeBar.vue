<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes, formatPercent, savedPercent } from '@/format'

// Before/after bar: the solid part is what remains, the checkerboard is what
// the optimizer removed without changing a single pixel.
const props = withDefaults(
  defineProps<{
    before: number
    after: number
    size?: 'sm' | 'lg'
    showLabel?: boolean
  }>(),
  { size: 'sm', showLabel: true },
)

const pct = computed(() => savedPercent(props.before, props.after))
const keptWidth = computed(() => `${Math.max(0, Math.min(100, 100 - pct.value))}%`)
const label = computed(
  () =>
    `${formatBytes(props.before)} → ${formatBytes(props.after)}, ${formatPercent(pct.value)} menos`,
)
</script>

<template>
  <div class="sizebar" :class="size" role="img" :aria-label="label" :title="label">
    <div class="track checker">
      <div class="kept" :style="{ width: keptWidth }"></div>
    </div>
    <span v-if="showLabel" class="pct">−{{ formatPercent(pct) }}</span>
  </div>
</template>

<style scoped>
.sizebar {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  min-width: 7rem;
}
.track {
  position: relative;
  flex: 1;
  height: 0.6rem;
  border-radius: 2px;
  overflow: hidden;
  background-size: 6px 6px;
  box-shadow: inset 0 0 0 1px var(--line);
}
.kept {
  height: 100%;
  background: var(--bar-fill);
  border-right: 2px solid var(--accent);
}
.pct {
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  color: var(--accent);
  font-weight: 600;
  min-width: 4.2em;
  text-align: right;
}
.lg .track {
  height: 1.6rem;
  border-radius: var(--radius-sm);
  background-size: 10px 10px;
}
.lg .kept {
  border-right-width: 3px;
}
.lg .pct {
  font-family: var(--font-display);
  font-size: var(--fs-xl);
  min-width: 5em;
}
</style>
