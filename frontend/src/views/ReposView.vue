<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { api } from '@/api'
import { usePolling } from '@/composables/usePolling'
import { useAction } from '@/composables/useAction'
import { useNow } from '@/composables/useNow'
import { useStatus } from '@/composables/useStatus'
import { useToast } from '@/composables/useToast'
import { formatRelative } from '@/format'
import type { RepoView } from '@/types'
import RepoStatusTag from '@/components/RepoStatusTag.vue'
import PrLink from '@/components/PrLink.vue'
import RowMenu from '@/components/RowMenu.vue'

const route = useRoute()
const router = useRouter()
const now = useNow()
const toast = useToast()
const { refreshStatus } = useStatus()

const repos = ref<RepoView[]>([])
const { loading, error, refresh } = usePolling(async () => {
  repos.value = await api.repos()
})

const { busy, run } = useAction(async () => {
  await Promise.all([refresh(), refreshStatus()])
})

// ---------------------------------------------------------------- add form

const newRepo = ref('')
const newMode = ref<'auto' | 'direct' | 'fork'>('auto')
const newAutoPr = ref(true)
const adding = ref(false)

async function add() {
  const value = newRepo.value.trim()
  if (!value) return
  adding.value = true
  try {
    const repo = await api.addRepo(value, newMode.value, newAutoPr.value)
    toast.ok(`${repo.owner}/${repo.name} added. Its first check is queued.`)
    newRepo.value = ''
    await refresh()
  } catch (e) {
    toast.error(e)
  } finally {
    adding.value = false
  }
}

// ----------------------------------------------------------------- filters

type Filter = 'all' | 'pr' | 'awaiting' | 'errors' | 'paused'
const FILTERS: { id: Filter; label: string; test: (r: RepoView) => boolean }[] = [
  { id: 'all', label: 'All', test: () => true },
  { id: 'pr', label: 'Open PR', test: (r) => r.pr_state === 'open' },
  { id: 'awaiting', label: 'Awaiting approval', test: (r) => r.status === 'awaiting_approval' },
  { id: 'errors', label: 'Failing', test: (r) => r.status === 'error' },
  { id: 'paused', label: 'Paused', test: (r) => !r.enabled },
]

const filter = computed<Filter>({
  get: () => {
    const q = route.query.filter
    return FILTERS.some((f) => f.id === q) ? (q as Filter) : 'all'
  },
  set: (v) => router.replace({ query: { ...route.query, filter: v === 'all' ? undefined : v } }),
})
const search = ref('')

const counts = computed(() =>
  Object.fromEntries(FILTERS.map((f) => [f.id, repos.value.filter(f.test).length])),
)

const visible = computed(() => {
  const f = FILTERS.find((x) => x.id === filter.value)!
  const q = search.value.trim().toLowerCase()
  return repos.value.filter((r) => f.test(r) && (!q || `${r.owner}/${r.name}`.toLowerCase().includes(q)))
})

function nextCheck(r: RepoView) {
  if (r.running) return 'now'
  if (r.queued) return 'queued'
  if (!r.enabled) return 'paused'
  return formatRelative(r.next_check_at, now.value)
}

// ----------------------------------------------------------------- actions

const check = (r: RepoView, force = false) =>
  run(
    `check-${r.id}`,
    () => api.checkRepo(r.id, force),
    force ? `Forced check of ${r.name} queued` : `Check of ${r.name} queued`,
  )
const toggle = (r: RepoView) =>
  run(
    `toggle-${r.id}`,
    () => api.patchRepo(r.id, { enabled: !r.enabled }),
    r.enabled ? `${r.name} paused` : `${r.name} resumed`,
  )
const publish = (r: RepoView) => run(`publish-${r.id}`, () => api.publishRepo(r.id), `Opening the PR for ${r.name}`)
// Deleting from the row menu takes two clicks: the first arms the item.
const armedDelete = ref<number | null>(null)
const remove = (r: RepoView) => {
  armedDelete.value = null
  return run(`delete-${r.id}`, () => api.deleteRepo(r.id), `${r.owner}/${r.name} removed`)
}
const checkAll = () => run('all', api.checkAll, 'All repositories queued for a check')
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Repositories</h1>
        <p class="lede">
          Each repository is cloned and checked periodically. When its default branch changes, git-opt optimizes the new
          images and proposes the changes in a PR.
        </p>
      </div>
      <div class="toolbar">
        <button type="button" class="btn" :disabled="busy !== null || !repos.length" @click="checkAll">
          Check all
        </button>
      </div>
    </header>

    <form class="add panel" @submit.prevent="add">
      <div class="field grow">
        <label for="new-repo">Add repository</label>
        <input
          id="new-repo"
          v-model="newRepo"
          class="input"
          placeholder="owner/name or https://github.com/owner/name"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
      <div class="field">
        <label for="new-mode">How to send changes</label>
        <select id="new-mode" v-model="newMode" class="select">
          <option value="auto">Automatic (based on permissions)</option>
          <option value="direct">Direct</option>
          <option value="fork">Fork (contribution)</option>
        </select>
      </div>
      <label class="check">
        <input v-model="newAutoPr" type="checkbox" />
        Open PRs automatically
      </label>
      <button class="btn btn-primary" type="submit" :disabled="adding || !newRepo.trim()">
        {{ adding ? 'Adding…' : 'Add' }}
      </button>
      <p class="hint full">
        For third-party repositories, automatic mode forks into your account and opens the PR as a contribution.
        Want every repository of an account? Use <RouterLink to="/sources">Sources</RouterLink>.
      </p>
    </form>

    <div class="filters">
      <div class="segmented" role="radiogroup" aria-label="Filter repositories">
        <label v-for="f in FILTERS" :key="f.id">
          <input v-model="filter" type="radio" name="filter" :value="f.id" />
          {{ f.label }} <span class="count">{{ counts[f.id] }}</span>
        </label>
      </div>
      <input v-model="search" class="input search" type="search" placeholder="Search" aria-label="Search repositories" />
    </div>

    <p v-if="error" class="notice danger">{{ error }}</p>

    <div v-if="visible.length" class="table-wrap">
      <table class="table repos">
        <thead>
          <tr>
            <th>Repository</th>
            <th>Status</th>
            <th>PR</th>
            <th>Next check</th>
            <th><span class="visually-hidden">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="r in visible" :key="r.id" :class="{ paused: !r.enabled }">
            <td data-label="Repository">
              <RouterLink :to="`/repos/${r.id}`" class="name">
                <span class="owner">{{ r.owner }}/</span>{{ r.name }}
              </RouterLink>
              <div class="meta">
                <span class="chip">{{ r.default_branch }}</span>
                <span v-if="r.mode === 'fork'" class="chip fork">contribution</span>
                <span v-if="!r.auto_pr" class="chip">manual approval</span>
              </div>
            </td>
            <td data-label="Status">
              <RepoStatusTag :repo="r" />
              <div v-if="r.running && r.phase" class="hint">{{ r.phase }}</div>
              <div v-else-if="r.status === 'error' && r.last_error" class="error-text clamp" :title="r.last_error">
                {{ r.last_error }}
              </div>
            </td>
            <td data-label="PR">
              <PrLink :number="r.pr_number" :url="r.pr_url" :state="r.pr_state" />
            </td>
            <td data-label="Next check" class="nowrap">
              {{ nextCheck(r) }}
              <div v-if="r.last_checked_at" class="hint">checked {{ formatRelative(r.last_checked_at, now) }}</div>
            </td>
            <td class="actions">
              <button
                v-if="r.status === 'awaiting_approval' && !r.running"
                type="button"
                class="btn btn-primary btn-sm"
                :disabled="busy !== null"
                @click="publish(r)"
              >
                Open PR
              </button>
              <button
                type="button"
                class="btn btn-sm"
                :disabled="busy !== null || r.running || r.queued"
                @click="check(r)"
              >
                Check now
              </button>
              <RowMenu :label="`More actions for ${r.name}`">
                <button type="button" :disabled="r.running || r.queued" @click="check(r, true)">
                  Force check
                </button>
                <button type="button" @click="toggle(r)">{{ r.enabled ? 'Pause' : 'Resume' }}</button>
                <RouterLink :to="`/repos/${r.id}`">Configure</RouterLink>
                <hr />
                <button v-if="armedDelete === r.id" type="button" class="danger" @click="remove(r)">
                  Sure? Remove {{ r.name }}
                </button>
                <button v-else type="button" class="danger" @click.stop="armedDelete = r.id">
                  Remove from list
                </button>
              </RowMenu>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!loading && repos.length" class="empty panel">No repository matches the filter.</p>
    <p v-else-if="!loading" class="empty panel">
      No repositories yet. Type <code>owner/name</code> above to start.
    </p>
  </div>
</template>

<style scoped>
.add {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 0.75rem 1rem;
}
.add .grow {
  flex: 1 1 20rem;
}
.add .check {
  min-height: 2.1rem;
}
.add .full {
  flex-basis: 100%;
}
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  align-items: center;
  justify-content: space-between;
}
.search {
  width: 14rem;
}
.count {
  margin-left: 0.25rem;
  font-variant-numeric: tabular-nums;
  opacity: 0.7;
}
.name {
  font-weight: 600;
  text-decoration: none;
}
.name:hover {
  text-decoration: underline;
}
.owner {
  font-weight: 400;
  color: var(--muted);
}
.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0.3rem;
  margin-top: 0.25rem;
}
.paused td {
  opacity: 0.72;
}
.clamp {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  max-width: 28rem;
}
.visually-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
}

@media (max-width: 760px) {
  .search {
    width: 100%;
  }
  .filters .segmented {
    flex-wrap: nowrap;
    overflow-x: auto;
  }
  .filters .segmented label {
    white-space: nowrap;
  }
  .table-wrap {
    overflow: visible;
  }
  .repos thead {
    display: none;
  }
  .repos,
  .repos tbody,
  .repos tr,
  .repos td {
    display: block;
  }
  .repos tr {
    padding: 0.6rem 0.75rem;
    border-bottom: 1px solid var(--line);
  }
  .repos tbody tr:last-child {
    border-bottom: 0;
  }
  .repos td {
    border: 0;
    padding: 0.2rem 0;
  }
  .repos td[data-label='Status'],
  .repos td[data-label='PR'],
  .repos td[data-label='Next check'] {
    display: inline-block;
    margin-right: 1rem;
  }
  .repos .actions {
    text-align: left;
    padding-top: 0.5rem;
  }
}
</style>
