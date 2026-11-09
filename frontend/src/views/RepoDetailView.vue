<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { RouterLink, useRouter } from 'vue-router'
import { api, ApiError } from '@/api'
import { usePolling } from '@/composables/usePolling'
import { useAction } from '@/composables/useAction'
import { useNow } from '@/composables/useNow'
import { useStatus } from '@/composables/useStatus'
import { useToast } from '@/composables/useToast'
import { formatBytes, formatDate, formatDuration, formatRelative, minutes, shortSha } from '@/format'
import type { RepoMode, RepoView, Run, Settings } from '@/types'
import RepoStatusTag from '@/components/RepoStatusTag.vue'
import RunStatusTag from '@/components/RunStatusTag.vue'
import PrLink from '@/components/PrLink.vue'
import SizeBar from '@/components/SizeBar.vue'
import ConfirmButton from '@/components/ConfirmButton.vue'

const props = defineProps<{ id: number }>()

const router = useRouter()
const now = useNow()
const toast = useToast()
const { refreshStatus } = useStatus()

const repo = ref<RepoView | null>(null)
const runs = ref<Run[]>([])
const globals = ref<Settings | null>(null)
const notFound = ref(false)

const { loading, error, refresh } = usePolling(async () => {
  try {
    const [r, rs] = await Promise.all([api.repo(props.id), api.repoRuns(props.id, 50)])
    repo.value = r
    runs.value = rs
    notFound.value = false
  } catch (e) {
    if (e instanceof ApiError && e.status === 404) notFound.value = true
    else throw e
  }
})

api
  .settings()
  .then((s) => (globals.value = s.settings))
  .catch(() => {})

const { busy, run } = useAction(async () => {
  await Promise.all([refresh(), refreshStatus()])
})

const webUrl = computed(() => repo.value?.clone_url.replace(/\.git$/, '') ?? null)
const forkUrl = computed(() => repo.value?.fork_clone_url?.replace(/\.git$/, '') ?? null)

// ------------------------------------------------------------ settings form

const form = reactive({
  enabled: true,
  auto_pr: true,
  mode: 'direct' as RepoMode,
  useGlobalInterval: true,
  interval: 20,
  ignore_globs: '',
})
const formReady = ref(false)

function loadForm(r: RepoView) {
  form.enabled = r.enabled
  form.auto_pr = r.auto_pr
  form.mode = r.mode
  form.useGlobalInterval = r.poll_interval_minutes == null
  form.interval = r.poll_interval_minutes ?? globals.value?.poll_interval_minutes ?? 20
  form.ignore_globs = r.ignore_globs
  formReady.value = true
}

// Fill the form once; later polls must not overwrite what the user is typing.
watch(repo, (r) => {
  if (r && !formReady.value) loadForm(r)
})

const dirty = computed(() => {
  const r = repo.value
  if (!r || !formReady.value) return false
  const interval = form.useGlobalInterval ? null : form.interval
  return (
    form.enabled !== r.enabled ||
    form.auto_pr !== r.auto_pr ||
    form.mode !== r.mode ||
    interval !== r.poll_interval_minutes ||
    form.ignore_globs !== r.ignore_globs
  )
})

const saving = ref(false)
async function save() {
  saving.value = true
  try {
    const updated = await api.patchRepo(props.id, {
      enabled: form.enabled,
      auto_pr: form.auto_pr,
      mode: form.mode,
      poll_interval_minutes: form.useGlobalInterval ? null : Math.max(1, Math.round(form.interval)),
      ignore_globs: form.ignore_globs,
    })
    repo.value = updated
    loadForm(updated)
    toast.ok('Changes saved')
  } catch (e) {
    toast.error(e)
  } finally {
    saving.value = false
  }
}

// ----------------------------------------------------------------- actions

const name = computed(() => repo.value?.name ?? '')
const check = (force = false) =>
  run(
    force ? 'force' : 'check',
    () => api.checkRepo(props.id, force),
    force ? 'Forced check queued' : 'Check queued',
  )
const publish = () => run('publish', () => api.publishRepo(props.id), 'Opening the PR')
const toggle = () =>
  run('toggle', async () => {
    const updated = await api.patchRepo(props.id, { enabled: !repo.value!.enabled })
    form.enabled = updated.enabled
  })
const resetIgnored = () =>
  run('reset', () => api.resetIgnored(props.id), 'Rejected files will be proposed again')
async function remove() {
  try {
    await api.deleteRepo(props.id)
    toast.ok(`${repo.value?.owner}/${name.value} removed`)
    router.push('/repos')
  } catch (e) {
    toast.error(e)
  }
}

const TRIGGER: Record<string, string> = { schedule: 'Scheduled', manual: 'Manual', publish: 'Publish' }
</script>

<template>
  <div class="page">
    <nav class="crumbs small" aria-label="Breadcrumb">
      <RouterLink to="/repos">Repositories</RouterLink>
      <span aria-hidden="true">/</span>
      <span>{{ repo ? repo.owner : '…' }}</span>
    </nav>

    <p v-if="notFound" class="empty panel">
      This repository is no longer on the list. <RouterLink to="/repos">Back to repositories</RouterLink>
    </p>
    <p v-else-if="error && !repo" class="notice danger">{{ error }}</p>

    <template v-if="repo">
      <header class="page-head">
        <div class="title">
          <h1>{{ repo.name }}</h1>
          <div class="tags">
            <RepoStatusTag :repo="repo" />
            <span v-if="repo.running && repo.phase" class="muted small">{{ repo.phase }}</span>
            <span v-if="repo.mode === 'fork'" class="chip fork">fork contribution</span>
            <span v-if="repo.pr_number" class="small">
              PR <PrLink :number="repo.pr_number" :url="repo.pr_url" :state="repo.pr_state" />
            </span>
          </div>
        </div>
        <div class="toolbar">
          <button
            v-if="repo.status === 'awaiting_approval' && !repo.running"
            type="button"
            class="btn btn-primary"
            :disabled="busy !== null"
            @click="publish"
          >
            Open PR
          </button>
          <button type="button" class="btn" :disabled="busy !== null || repo.running || repo.queued" @click="check()">
            Check now
          </button>
          <button
            type="button"
            class="btn"
            :disabled="busy !== null || repo.running || repo.queued"
            title="Review every image even if the default branch has not changed"
            @click="check(true)"
          >
            Force check
          </button>
          <button type="button" class="btn btn-quiet" :disabled="busy !== null" @click="toggle">
            {{ repo.enabled ? 'Pause' : 'Resume' }}
          </button>
        </div>
      </header>

      <div v-if="repo.status === 'awaiting_approval'" class="notice">
        <strong>An optimization is ready.</strong>
        <span>
          Commit <code>{{ shortSha(repo.pending_commit) }}</code> is ready locally. Review the latest run and
          press “Open PR” to publish it.
        </span>
      </div>
      <div v-if="repo.status === 'error' && repo.last_error" class="notice danger">
        <strong>The last check failed.</strong>
        <span class="mono small">{{ repo.last_error }}</span>
      </div>

      <div class="columns">
        <section class="section">
          <h2>Summary</h2>
          <dl class="facts panel">
            <dt>On GitHub</dt>
            <dd>
              <a v-if="webUrl" :href="webUrl" target="_blank" rel="noopener">{{ repo.owner }}/{{ repo.name }}</a>
            </dd>
            <dt>Delivery</dt>
            <dd>
              <template v-if="repo.mode === 'fork'">
                Contribution from
                <a v-if="forkUrl" :href="forkUrl" target="_blank" rel="noopener">{{ repo.fork_full_name }}</a>
                <span v-else class="muted">a fork (created with the first PR)</span>
              </template>
              <template v-else>Direct (branch in the repository)</template>
            </dd>
            <dt>Default branch</dt>
            <dd><span class="chip">{{ repo.default_branch }}</span></dd>
            <dt>Last commit checked</dt>
            <dd class="mono">{{ shortSha(repo.last_seen_sha) }}</dd>
            <dt>Last check</dt>
            <dd :title="formatDate(repo.last_checked_at)">{{ formatRelative(repo.last_checked_at, now) }}</dd>
            <dt>Next check</dt>
            <dd :title="formatDate(repo.next_check_at)">
              {{ repo.enabled ? formatRelative(repo.next_check_at, now) : 'paused' }}
            </dd>
            <dt>Saved in merged PRs</dt>
            <dd>{{ formatBytes(repo.bytes_saved) }}</dd>
            <dt>Added</dt>
            <dd>{{ formatDate(repo.created_at) }}</dd>
          </dl>
        </section>

        <section class="section">
          <h2>Configuration</h2>
          <form class="panel settings" @submit.prevent="save">
            <label class="check">
              <input v-model="form.enabled" type="checkbox" />
              Watch this repository
            </label>
            <label class="check">
              <input v-model="form.auto_pr" type="checkbox" />
              Open PRs automatically
            </label>

            <fieldset class="field">
              <legend class="field-label">How to send changes</legend>
              <div class="segmented">
                <label><input v-model="form.mode" type="radio" value="direct" /> Direct</label>
                <label><input v-model="form.mode" type="radio" value="fork" /> Fork (contribution)</label>
              </div>
            </fieldset>

            <fieldset class="field">
              <legend class="field-label">Check frequency</legend>
              <label class="check">
                <input v-model="form.useGlobalInterval" type="checkbox" />
                Use the global interval
                <span v-if="globals" class="muted">
                  ({{ minutes(globals.poll_interval_minutes) }} + up to {{ minutes(globals.poll_jitter_minutes) }} random)
                </span>
              </label>
              <div v-if="!form.useGlobalInterval" class="inline">
                <input
                  id="interval"
                  v-model.number="form.interval"
                  class="input narrow"
                  type="number"
                  min="1"
                  step="1"
                  aria-label="Minutes between checks"
                />
                <span class="small muted">minutes, plus the global random delay</span>
              </div>
            </fieldset>

            <div class="field">
              <label for="ignore">Ignored files</label>
              <textarea
                id="ignore"
                v-model="form.ignore_globs"
                class="textarea"
                spellcheck="false"
                placeholder="docs/screenshots/**&#10;**/*.min.png"
              ></textarea>
              <span class="hint">One pattern per line. Added to the global patterns in Settings.</span>
            </div>

            <div class="form-actions">
              <button class="btn btn-primary" type="submit" :disabled="!dirty || saving">
                {{ saving ? 'Saving…' : 'Save changes' }}
              </button>
              <button v-if="dirty" class="btn btn-quiet" type="button" @click="repo && loadForm(repo)">
                Discard
              </button>
            </div>
          </form>
        </section>
      </div>

      <section class="section">
        <div class="section-head">
          <h2>History</h2>
          <span v-if="runs.length > 1" class="muted small">last {{ runs.length }} runs</span>
        </div>
        <div v-if="runs.length" class="table-wrap">
          <table class="table">
            <thead>
              <tr>
                <th>When</th>
                <th>Trigger</th>
                <th>Result</th>
                <th class="num">Images</th>
                <th>Savings</th>
                <th>PR</th>
                <th class="num">Duration</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="r in runs" :key="r.id" class="clickable" @click="router.push(`/activity/${r.id}`)">
                <td class="nowrap" :title="formatDate(r.started_at)">
                  <RouterLink :to="`/activity/${r.id}`" @click.stop>{{ formatRelative(r.started_at, now) }}</RouterLink>
                </td>
                <td class="muted">{{ TRIGGER[r.trigger] ?? r.trigger }}</td>
                <td>
                  <RunStatusTag :status="r.status" />
                  <div v-if="r.status === 'error' && r.message" class="error-text clamp" :title="r.message">
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
        <p v-else-if="!loading" class="empty panel">This repository has not been checked yet.</p>
      </section>

      <section class="section danger-zone">
        <h2>Maintenance</h2>
        <div class="panel maint">
          <div>
            <p><strong>Forget rejected files</strong></p>
            <p class="small muted">
              When you close a PR without merging, git-opt stops touching those images. This makes it forget them.
            </p>
          </div>
          <button type="button" class="btn" :disabled="busy !== null" @click="resetIgnored">Forget rejected files</button>
        </div>
        <div class="panel maint">
          <div>
            <p><strong>Remove from list</strong></p>
            <p class="small muted">
              Deletes the local clone and the run history. Leaves the GitHub repository and any open PRs untouched.
            </p>
          </div>
          <ConfirmButton label="Remove repository" confirm-label="Sure? Remove" @confirm="remove" />
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
.tags {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem 1rem;
}
.columns {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 1.5rem;
  align-items: start;
}
.settings {
  display: grid;
  gap: 1rem;
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
.inline {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}
.narrow {
  width: 6rem;
}
.form-actions {
  display: flex;
  gap: 0.5rem;
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
  max-width: 24rem;
}
.maint {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem 1.5rem;
}
.maint p + p {
  margin-top: 0.15rem;
}
.maint strong {
  font-weight: 600;
}
@media (max-width: 900px) {
  .columns {
    grid-template-columns: 1fr;
  }
}
</style>
