<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { RouterLink } from 'vue-router'
import { api } from '@/api'
import { usePolling } from '@/composables/usePolling'
import { useAction } from '@/composables/useAction'
import { useNow } from '@/composables/useNow'
import { useStatus } from '@/composables/useStatus'
import { useToast } from '@/composables/useToast'
import { formatDate, formatRelative, minutes } from '@/format'
import type { Source, SourceKind } from '@/types'
import RowMenu from '@/components/RowMenu.vue'

const now = useNow()
const toast = useToast()
const { status } = useStatus()

const sources = ref<Source[]>([])
const syncEvery = ref<number | null>(null)

const { loading, error, refresh } = usePolling(async () => {
  sources.value = await api.sources()
})

api
  .settings()
  .then((s) => (syncEvery.value = s.settings.source_sync_interval_minutes))
  .catch(() => {})

const { busy, run } = useAction(refresh)

// ---------------------------------------------------------------- add form

const form = reactive({
  kind: 'user' as SourceKind,
  name: '',
  include_forks: false,
  include_archived: false,
  auto_sync: true,
})
const adding = ref(false)
const hasMe = computed(() => sources.value.some((s) => s.kind === 'me'))
const canAdd = computed(() => (form.kind === 'me' ? !hasMe.value : form.name.trim().length > 0))

async function add() {
  if (!canAdd.value) return
  adding.value = true
  try {
    const name = form.kind === 'me' ? '' : form.name.trim().replace(/^@/, '')
    await api.addSource({ ...form, name })
    toast.ok(`Importing ${describe({ kind: form.kind, name })}. Repositories will appear in the list shortly.`)
    form.name = ''
    await refresh()
  } catch (e) {
    toast.error(e)
  } finally {
    adding.value = false
  }
}

// ------------------------------------------------------------------- list

const KIND: Record<SourceKind, string> = { user: 'User', org: 'Organization', me: 'My repositories' }

function describe(s: Pick<Source, 'kind' | 'name'>) {
  if (s.kind === 'me') return 'every repository you can access'
  return s.kind === 'org' ? `the ${s.name} organization` : `the repositories of ${s.name}`
}

const toggle = (s: Source, key: 'include_forks' | 'include_archived' | 'auto_sync') =>
  run(`${key}-${s.id}`, () => api.patchSource(s.id, { [key]: !s[key] }))

const sync = (s: Source) => run(`sync-${s.id}`, () => api.syncSource(s.id), 'Sync queued')

const armed = ref<number | null>(null)
const remove = (s: Source, deleteRepos: boolean) => {
  armed.value = null
  return run(
    `delete-${s.id}`,
    () => api.deleteSource(s.id, deleteRepos),
    deleteRepos ? 'Source and its repositories removed' : 'Source removed; its repositories stay on the list',
  )
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Sources</h1>
        <p class="lede">
          A source is a GitHub account whose repositories are all watched. git-opt lists it again
          <template v-if="syncEvery">every {{ minutes(syncEvery) }}</template>
          <template v-else>periodically</template>
          to pick up new repositories.
        </p>
      </div>
    </header>

    <form class="panel add" @submit.prevent="add">
      <fieldset class="field">
        <legend class="field-label">What to import</legend>
        <div class="segmented">
          <label><input v-model="form.kind" type="radio" value="user" /> User</label>
          <label><input v-model="form.kind" type="radio" value="org" /> Organization</label>
          <label><input v-model="form.kind" type="radio" value="me" /> All my repositories</label>
        </div>
      </fieldset>

      <div v-if="form.kind !== 'me'" class="field">
        <label for="source-name">GitHub {{ form.kind === 'org' ? 'organization' : 'user' }}</label>
        <input
          id="source-name"
          v-model="form.name"
          class="input"
          :placeholder="form.kind === 'org' ? 'my-company' : 'octocat'"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
      <p v-else class="small muted me-hint">
        <template v-if="hasMe">Already added.</template>
        <template v-else>
          Every repository
          <strong v-if="status?.github.login">@{{ status.github.login }}</strong>
          <template v-else>the configured token</template>
          can access: owned, collaborations and your organizations.
        </template>
      </p>

      <div class="options">
        <label class="check"><input v-model="form.include_forks" type="checkbox" /> Include forks</label>
        <label class="check">
          <input v-model="form.include_archived" type="checkbox" /> Include archived
        </label>
        <label class="check"><input v-model="form.auto_sync" type="checkbox" /> Add new repositories automatically</label>
      </div>
      <p class="hint">
        Repositories you cannot push to are treated as contributions: git-opt forks them and opens the PR from the
        fork. Archived repositories do not accept PRs.
      </p>

      <div>
        <button class="btn btn-primary" type="submit" :disabled="adding || !canAdd">
          {{ adding ? 'Importing…' : 'Import repositories' }}
        </button>
      </div>
    </form>

    <p v-if="error" class="notice danger">{{ error }}</p>

    <div v-if="sources.length" class="table-wrap">
      <table class="table">
        <thead>
          <tr>
            <th>Source</th>
            <th class="num">Repositories</th>
            <th>Last sync</th>
            <th>Options</th>
            <th><span class="sr">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in sources" :key="s.id">
            <td>
              <strong>{{ s.kind === 'me' ? 'My repositories' : s.name }}</strong>
              <div class="hint">{{ s.kind === 'me' ? 'Everything the token can access' : KIND[s.kind] }}</div>
            </td>
            <td class="num">
              <RouterLink to="/repos">{{ s.repo_count }}</RouterLink>
            </td>
            <td>
              <span :title="formatDate(s.last_synced_at)">
                {{ s.last_synced_at ? formatRelative(s.last_synced_at, now) : 'pending' }}
              </span>
              <div v-if="s.last_error" class="error-text">{{ s.last_error }}</div>
            </td>
            <td>
              <div class="toggles">
                <label class="check small">
                  <input type="checkbox" :checked="s.include_forks" :disabled="busy !== null" @change="toggle(s, 'include_forks')" />
                  Forks
                </label>
                <label class="check small">
                  <input type="checkbox" :checked="s.include_archived" :disabled="busy !== null" @change="toggle(s, 'include_archived')" />
                  Archived
                </label>
                <label class="check small">
                  <input type="checkbox" :checked="s.auto_sync" :disabled="busy !== null" @change="toggle(s, 'auto_sync')" />
                  Auto sync
                </label>
              </div>
            </td>
            <td class="actions">
              <button type="button" class="btn btn-sm" :disabled="busy !== null" @click="sync(s)">Sync now</button>
              <RowMenu :label="`More actions for ${s.name || 'my repositories'}`">
                <button type="button" @click="remove(s, false)">Remove source, keep repositories</button>
                <button v-if="armed === s.id" type="button" class="danger" @click="remove(s, true)">
                  Sure? Remove {{ s.repo_count }} repositories
                </button>
                <button v-else type="button" class="danger" @click.stop="armed = s.id">
                  Remove source and its repositories
                </button>
              </RowMenu>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!loading" class="empty panel">
      No sources. Import a user, an organization or all your repositories at once.
    </p>
  </div>
</template>

<style scoped>
.add {
  display: grid;
  gap: 1rem;
  max-width: 44rem;
}
fieldset {
  border: 0;
  margin: 0;
  padding: 0;
  min-width: 0;
}
legend {
  padding: 0;
  margin-bottom: 0.35rem;
}
.options {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem 1.25rem;
}
.me-hint strong {
  color: var(--ink);
}
.toggles {
  display: flex;
  flex-wrap: wrap;
  gap: 0.25rem 1rem;
}
.sr {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip-path: inset(50%);
}
</style>
