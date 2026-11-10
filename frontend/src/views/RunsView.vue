<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterLink, useRouter } from 'vue-router'
import { api } from '@/api'
import { usePolling } from '@/composables/usePolling'
import { useNow } from '@/composables/useNow'
import { formatBytes, formatDate, formatDuration, formatRelative } from '@/format'
import type { Run } from '@/types'
import RunStatusTag from '@/components/RunStatusTag.vue'
import SizeBar from '@/components/SizeBar.vue'
import PrLink from '@/components/PrLink.vue'

const router = useRouter()
const now = useNow()

const runs = ref<Run[]>([])
const { loading, error } = usePolling(async () => {
  runs.value = await api.runs(200)
})

const onlyChanges = ref(false)
const search = ref('')

const visible = computed(() => {
  const q = search.value.trim().toLowerCase()
  return runs.value.filter(
    (r) =>
      (!onlyChanges.value || r.files_optimized > 0 || r.status === 'error' || r.status === 'fork_deleted') &&
      (!q || (r.repo_full_name ?? '').toLowerCase().includes(q)),
  )
})

const proposed = computed(() =>
  visible.value.reduce((acc, r) => acc + (r.files_optimized ? r.bytes_before - r.bytes_after : 0), 0),
)

const TRIGGER: Record<string, string> = { schedule: 'Scheduled', manual: 'Manual', publish: 'Publish' }
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Activity</h1>
        <p class="lede">
          Every repository check that found something, failed or was triggered by hand is recorded here.
          <template v-if="proposed > 0">The runs shown proposed {{ formatBytes(proposed) }} of savings.</template>
        </p>
      </div>
    </header>

    <div class="filters">
      <label class="check">
        <input v-model="onlyChanges" type="checkbox" />
        Only runs with optimizations or errors
      </label>
      <input v-model="search" class="input search" type="search" placeholder="Filter by repository" aria-label="Filter by repository" />
    </div>

    <p v-if="error" class="notice danger">{{ error }}</p>

    <div v-if="visible.length" class="table-wrap">
      <table class="table">
        <thead>
          <tr>
            <th>When</th>
            <th>Repository</th>
            <th>Trigger</th>
            <th>Result</th>
            <th class="num">Images</th>
            <th>Savings</th>
            <th>PR</th>
            <th class="num">Duration</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="r in visible" :key="r.id" class="clickable" @click="router.push(`/activity/${r.id}`)">
            <td class="nowrap" :title="formatDate(r.started_at)">
              <RouterLink :to="`/activity/${r.id}`" @click.stop>{{ formatRelative(r.started_at, now) }}</RouterLink>
            </td>
            <td>
              <RouterLink :to="`/repos/${r.repo_id}`" class="repo" @click.stop>{{ r.repo_full_name }}</RouterLink>
            </td>
            <td class="muted">{{ TRIGGER[r.trigger] ?? r.trigger }}</td>
            <td>
              <RunStatusTag :status="r.status" />
              <div
                v-if="r.message"
                class="clamp"
                :class="r.status === 'error' ? 'error-text' : 'note-text'"
                :title="r.message"
              >
                {{ r.message }}
              </div>
            </td>
            <td class="num">{{ r.files_optimized }} / {{ r.files_scanned }}</td>
            <td class="saving">
              <SizeBar v-if="r.files_optimized" :before="r.bytes_before" :after="r.bytes_after" />
              <span v-else class="muted">—</span>
            </td>
            <td><PrLink :number="r.pr_number" :url="r.pr_url" /></td>
            <td class="num muted">{{ formatDuration(r.started_at, r.finished_at) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!loading && runs.length" class="empty panel">Nothing matches the filter.</p>
    <p v-else-if="!loading" class="empty panel">
      No runs yet. Add a <RouterLink to="/repos">repository</RouterLink> to get started.
    </p>
  </div>
</template>

<style scoped>
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  align-items: center;
  justify-content: space-between;
}
.search {
  width: 16rem;
}
.repo {
  font-weight: 500;
  text-decoration: none;
  color: var(--ink);
}
.repo:hover {
  text-decoration: underline;
}
.saving {
  min-width: 9rem;
}
.clamp {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  max-width: 22rem;
}
@media (max-width: 560px) {
  .search {
    width: 100%;
  }
}
</style>
