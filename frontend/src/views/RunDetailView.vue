<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterLink } from 'vue-router'
import { api, ApiError } from '@/api'
import { usePolling } from '@/composables/usePolling'
import { formatBytes, formatDate, formatDuration, shortSha } from '@/format'
import type { RunDetail } from '@/types'
import RunStatusTag from '@/components/RunStatusTag.vue'
import SizeBar from '@/components/SizeBar.vue'
import PrLink from '@/components/PrLink.vue'

const props = defineProps<{ id: number }>()

const detail = ref<RunDetail | null>(null)
const notFound = ref(false)

// Poll only while the run is still going.
const { error } = usePolling(async () => {
  if (detail.value && detail.value.run.status !== 'running') return
  try {
    detail.value = await api.run(props.id)
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) notFound.value = true
    else throw e
  }
}, 3000)

const run = computed(() => detail.value?.run ?? null)
const files = computed(() => detail.value?.files ?? [])
const saved = computed(() => (run.value ? run.value.bytes_before - run.value.bytes_after : 0))

const TRIGGER: Record<string, string> = {
  schedule: 'Scheduled check',
  manual: 'Manual check',
  publish: 'PR publish',
}

function dirOf(path: string) {
  const i = path.lastIndexOf('/')
  return i >= 0 ? path.slice(0, i + 1) : ''
}
function baseOf(path: string) {
  return path.slice(path.lastIndexOf('/') + 1)
}
</script>

<template>
  <div class="page">
    <nav class="crumbs small" aria-label="Breadcrumb">
      <RouterLink to="/activity">Activity</RouterLink>
      <span aria-hidden="true">/</span>
      <span>Run {{ id }}</span>
    </nav>

    <p v-if="notFound" class="empty panel">
      This run does not exist or was deleted with its repository. <RouterLink to="/activity">View activity</RouterLink>
    </p>
    <p v-else-if="error && !run" class="notice danger">{{ error }}</p>

    <template v-if="run">
      <header class="page-head">
        <div class="title">
          <h1>
            <RouterLink :to="`/repos/${run.repo_id}`" class="repo">{{ run.repo_full_name }}</RouterLink>
          </h1>
          <div class="tags">
            <RunStatusTag :status="run.status" />
            <span class="muted small">{{ TRIGGER[run.trigger] ?? run.trigger }}, {{ formatDate(run.started_at) }}</span>
            <PrLink :number="run.pr_number" :url="run.pr_url" />
          </div>
        </div>
      </header>

      <p v-if="run.message" class="notice" :class="{ danger: run.status === 'error' }">
        <span :class="{ mono: run.status === 'error' }">{{ run.message }}</span>
      </p>

      <section v-if="run.files_optimized" class="summary">
        <p class="saved">
          {{ formatBytes(saved) }} saved across {{ run.files_optimized }}
          {{ run.files_optimized === 1 ? 'image' : 'images' }}
        </p>
        <SizeBar :before="run.bytes_before" :after="run.bytes_after" size="lg" />
        <p class="small muted">
          {{ formatBytes(run.bytes_before) }} before, {{ formatBytes(run.bytes_after) }} after. Same pixels: every
          file was decoded and compared before it was accepted.
        </p>
      </section>

      <dl class="facts panel">
        <dt>Images scanned</dt>
        <dd>{{ run.files_scanned }}</dd>
        <dt>Default branch commit</dt>
        <dd class="mono">{{ shortSha(run.head_sha) }}</dd>
        <dt>git-opt commit</dt>
        <dd class="mono">{{ shortSha(run.commit_sha) }}</dd>
        <dt>Duration</dt>
        <dd>{{ run.status === 'running' ? 'running' : formatDuration(run.started_at, run.finished_at) }}</dd>
      </dl>

      <section v-if="files.length" class="section">
        <h2>Files</h2>
        <div class="table-wrap">
          <table class="table">
            <thead>
              <tr>
                <th>File</th>
                <th>Optimizer</th>
                <th class="num">Before</th>
                <th class="num">After</th>
                <th>Savings</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="f in files" :key="f.id">
                <td class="path" :title="f.path">
                  <span class="dir">{{ dirOf(f.path) }}</span><span class="base">{{ baseOf(f.path) }}</span>
                </td>
                <td class="muted nowrap">{{ f.optimizer }}</td>
                <td class="num nowrap">{{ formatBytes(f.bytes_before) }}</td>
                <td class="num nowrap">{{ formatBytes(f.bytes_after) }}</td>
                <td class="saving">
                  <SizeBar :before="f.bytes_before" :after="f.bytes_after" />
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </template>
  </div>
</template>

<style scoped>
.crumbs {
  display: flex;
  gap: 0.4rem;
  color: var(--muted);
  margin-bottom: -1rem;
}
.title {
  display: grid;
  gap: 0.5rem;
}
.repo {
  color: var(--ink);
  text-decoration: none;
}
.repo:hover {
  text-decoration: underline;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem 1rem;
}
.summary {
  display: grid;
  gap: 0.6rem;
  max-width: 48rem;
}
.saved {
  font-family: var(--font-display);
  font-size: var(--fs-xl);
  font-weight: 600;
}
.facts.panel {
  max-width: 34rem;
}
.path {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  max-width: 30rem;
  overflow-wrap: anywhere;
}
.dir {
  color: var(--muted);
}
.saving {
  min-width: 10rem;
  width: 28%;
}
</style>
