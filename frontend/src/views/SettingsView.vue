<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { api } from '@/api'
import { useStatus } from '@/composables/useStatus'
import { useToast } from '@/composables/useToast'
import { minutes } from '@/format'
import type { Settings } from '@/types'

const toast = useToast()
const { status, refreshStatus } = useStatus()

const form = ref<Settings | null>(null)
const original = ref('')
const tokenSet = ref(false)
const loadError = ref<string | null>(null)

async function load() {
  try {
    const res = await api.settings()
    form.value = structuredClone(res.settings)
    original.value = JSON.stringify(res.settings)
    tokenSet.value = res.github_token_set
    loadError.value = null
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : String(e)
  }
}
onMounted(load)

const dirty = computed(() => form.value !== null && JSON.stringify(form.value) !== original.value)

const saving = ref(false)
async function save() {
  if (!form.value) return
  saving.value = true
  try {
    const saved = await api.saveSettings(form.value)
    form.value = structuredClone(saved)
    original.value = JSON.stringify(saved)
    toast.ok('Settings saved')
    await refreshStatus()
  } catch (e) {
    toast.error(e)
  } finally {
    saving.value = false
  }
}

function discard() {
  form.value = JSON.parse(original.value)
}

// ------------------------------------------------------------- optimizers

const optimizers = computed(() => status.value?.optimizers ?? [])

function optimizerOn(id: string, fallback: boolean) {
  return form.value?.optimizers[id] ?? fallback
}
function setOptimizer(id: string, on: boolean) {
  if (form.value) form.value.optimizers = { ...form.value.optimizers, [id]: on }
}

const levelText = computed(() => {
  const l = form.value?.png_level ?? 3
  if (l <= 1) return 'fast, smaller savings'
  if (l <= 3) return 'balanced'
  if (l <= 5) return 'more savings, slower'
  return 'maximum savings, very slow'
})

// ------------------------------------------------------------ github token

const newToken = ref('')
const savingToken = ref(false)

async function saveToken(token: string | null) {
  savingToken.value = true
  try {
    await api.setGithubToken(token)
    tokenSet.value = token !== null
    newToken.value = ''
    toast.ok(token ? 'Token saved' : 'Token removed')
    await refreshStatus()
  } catch (e) {
    toast.error(e)
  } finally {
    savingToken.value = false
  }
}

const TOKEN_SOURCE: Record<string, string> = {
  settings: 'the token saved here',
  env: 'the server environment variable',
  gh: 'the server gh CLI session',
}

const detected = computed(() => {
  const g = status.value?.git
  return g?.identity_source && g.identity_source !== 'settings' ? g.identity : null
})
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Settings</h1>
        <p class="lede">Global values. Each repository can override its interval and ignored files.</p>
      </div>
    </header>

    <p v-if="loadError" class="notice danger">{{ loadError }}</p>

    <form v-if="form" class="settings" @submit.prevent="save">
      <section class="group">
        <div class="group-head">
          <h2>Checks</h2>
          <p class="hint">
            Each repository runs <code>git fetch</code> every {{ minutes(form.poll_interval_minutes) }} plus a random delay
            of up to {{ minutes(form.poll_jitter_minutes) }}, so they don't all run at once.
          </p>
        </div>
        <div class="fields two">
          <div class="field">
            <label for="interval">Interval (minutes)</label>
            <input id="interval" v-model.number="form.poll_interval_minutes" class="input" type="number" min="1" />
          </div>
          <div class="field">
            <label for="jitter">Max random delay (minutes)</label>
            <input id="jitter" v-model.number="form.poll_jitter_minutes" class="input" type="number" min="0" />
          </div>
          <div class="field">
            <label for="jobs">Repositories at once</label>
            <input id="jobs" v-model.number="form.max_concurrent_jobs" class="input" type="number" min="1" max="32" />
          </div>
          <div class="field">
            <label for="sync">Re-list sources every (minutes)</label>
            <input
              id="sync"
              v-model.number="form.source_sync_interval_minutes"
              class="input"
              type="number"
              min="5"
            />
            <span class="hint">To discover new repositories of users and organizations.</span>
          </div>
        </div>
      </section>

      <section class="group">
        <div class="group-head">
          <h2>Optimization</h2>
          <p class="hint">
            Only lossless changes are accepted: every optimized image is decoded and compared pixel by pixel with the
            original before it is kept.
          </p>
        </div>
        <div class="fields">
          <fieldset class="field">
            <legend class="field-label">Formats</legend>
            <ul class="optimizers">
              <li v-for="o in optimizers" :key="o.id">
                <label class="check">
                  <input
                    type="checkbox"
                    :checked="optimizerOn(o.id, o.enabled)"
                    @change="setOptimizer(o.id, ($event.target as HTMLInputElement).checked)"
                  />
                  {{ o.label }}
                </label>
                <span class="hint">
                  {{ o.extensions.map((e) => '.' + e).join(' ') }}
                  <span v-if="o.available"> ({{ o.detail }})</span>
                  <span v-else class="error-text"> Unavailable: {{ o.detail }}</span>
                </span>
              </li>
              <li v-if="!optimizers.length" class="hint">Loading optimizers…</li>
            </ul>
          </fieldset>

          <div class="field">
            <label for="level">PNG compression effort: {{ form.png_level }}, {{ levelText }}</label>
            <input id="level" v-model.number="form.png_level" class="range" type="range" min="0" max="6" step="1" />
          </div>

          <label class="check">
            <input v-model="form.png_zopfli" type="checkbox" />
            Use Zopfli for PNG (a few % smaller, much slower)
          </label>

          <label class="check">
            <input v-model="form.strip_metadata" type="checkbox" />
            Strip PNG metadata that doesn't affect rendering (text, timestamps)
          </label>

          <div class="fields three">
            <div class="field">
              <label for="min-pct">Min savings (%)</label>
              <input id="min-pct" v-model.number="form.min_savings_percent" class="input" type="number" min="0" step="0.1" />
            </div>
            <div class="field">
              <label for="min-bytes">Min savings (bytes)</label>
              <input id="min-bytes" v-model.number="form.min_savings_bytes" class="input" type="number" min="0" />
            </div>
            <div class="field">
              <label for="max-size">Max file size (MB)</label>
              <input id="max-size" v-model.number="form.max_file_size_mb" class="input" type="number" min="1" />
            </div>
          </div>
          <p class="hint">An image only goes into the PR if it meets both minimums.</p>
        </div>
      </section>

      <section class="group">
        <div class="group-head">
          <h2>Ignored files</h2>
          <p class="hint">Glob patterns, one per line, applied to every repository.</p>
        </div>
        <div class="fields">
          <textarea
            v-model="form.ignore_globs"
            class="textarea"
            spellcheck="false"
            aria-label="Patterns ignored in every repository"
            placeholder="**/node_modules/**&#10;vendor/**&#10;**/*.9.png"
          ></textarea>
        </div>
      </section>

      <section class="group">
        <div class="group-head">
          <h2>Commits and PRs</h2>
          <p class="hint">
            git-opt marks its commits so it can recognize them later and never optimize the same file twice.
          </p>
        </div>
        <div class="fields">
          <div class="fields two">
            <div class="field">
              <label for="branch">Working branch</label>
              <input id="branch" v-model="form.branch_name" class="input mono" spellcheck="false" />
            </div>
            <div class="field">
              <label for="title">Commit and PR title</label>
              <input id="title" v-model="form.commit_title" class="input" />
            </div>
            <div class="field">
              <label for="author-name">Commit author</label>
              <input
                id="author-name"
                v-model="form.commit_author_name"
                class="input"
                :placeholder="detected?.name || 'Name'"
              />
            </div>
            <div class="field">
              <label for="author-email">Author email</label>
              <input
                id="author-email"
                v-model="form.commit_author_email"
                class="input"
                type="email"
                :placeholder="detected?.email || 'email@example.com'"
              />
            </div>
          </div>
          <p class="hint">
            Leave empty to use your <code>git config</code> or, failing that, your GitHub account.
            <template v-if="detected">Currently: {{ detected.name }} &lt;{{ detected.email }}&gt;.</template>
          </p>
          <label class="check">
            <input v-model="form.sign_commits" type="checkbox" />
            Sign commits (when your git config asks for it)
          </label>
          <p class="hint">Turn it off if the server cannot reach your GPG or SSH key (in Docker, for example).</p>
        </div>
      </section>

      <div v-if="dirty" class="savebar">
        <span>You have unsaved changes.</span>
        <div class="toolbar">
          <button type="button" class="btn btn-quiet" @click="discard">Discard</button>
          <button type="submit" class="btn btn-primary" :disabled="saving">
            {{ saving ? 'Saving…' : 'Save settings' }}
          </button>
        </div>
      </div>
    </form>

    <section class="group">
      <div class="group-head">
        <h2>GitHub</h2>
        <p class="hint">
          PRs are opened with this account. Lookup order: the token saved here, <code>GIT_OPT_GITHUB_TOKEN</code> or
          <code>GITHUB_TOKEN</code>, then the <code>gh</code> session.
        </p>
      </div>
      <div class="fields">
        <p v-if="status?.github.ok">
          <span class="tag ok">Connected as @{{ status.github.login }}</span>
          <span v-if="status.github.token_source" class="hint">
            using {{ TOKEN_SOURCE[status.github.token_source] }}</span>
        </p>
        <p v-else-if="status" class="error-text">{{ status.github.error || 'No GitHub token.' }}</p>

        <form class="token" @submit.prevent="saveToken(newToken.trim())">
          <div class="field grow">
            <label for="token">{{ tokenSet ? 'Replace the saved token' : 'Save a token' }}</label>
            <input
              id="token"
              v-model="newToken"
              class="input mono"
              type="password"
              autocomplete="off"
              placeholder="github_pat_…"
            />
          </div>
          <button class="btn btn-primary" type="submit" :disabled="savingToken || !newToken.trim()">Save token</button>
          <button
            v-if="tokenSet"
            class="btn btn-danger"
            type="button"
            :disabled="savingToken"
            @click="saveToken(null)"
          >
            Remove saved token
          </button>
        </form>
        <p class="hint">
          The token is never shown again. It needs write access to repository contents and pull requests, and
          permission to create forks for contributions.
        </p>
      </div>
    </section>
  </div>
</template>

<style scoped>
.settings {
  display: grid;
  gap: 0;
}
.group {
  display: grid;
  grid-template-columns: minmax(12rem, 17rem) minmax(0, 40rem);
  gap: 1rem 2.5rem;
  padding: 1.5rem 0;
  border-top: 1px solid var(--line);
}
.group-head {
  display: grid;
  gap: 0.4rem;
  align-content: start;
}
.group-head h2 {
  font-size: var(--fs-lg);
}
.fields {
  display: grid;
  gap: 1rem;
  align-content: start;
}
.fields.two {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}
.fields.three {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}
fieldset {
  border: 0;
  margin: 0;
  padding: 0;
  min-width: 0;
}
legend {
  padding: 0;
  margin-bottom: 0.4rem;
}
.optimizers {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 0.6rem;
}
.optimizers li {
  display: grid;
  gap: 0.1rem;
}
.optimizers .hint {
  padding-left: 1.5rem;
}
.range {
  width: 100%;
  accent-color: var(--accent);
}
.token {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 0.5rem;
}
.token .grow {
  flex: 1 1 16rem;
}
.savebar {
  position: sticky;
  bottom: 1rem;
  z-index: 10;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  margin-top: 0.5rem;
  padding: 0.65rem 0.75rem 0.65rem 1rem;
  background: var(--ink);
  color: var(--surface);
  border-radius: var(--radius-md);
  box-shadow: 0 8px 28px rgb(0 0 0 / 0.2);
}
.savebar .btn-quiet {
  color: var(--surface);
  opacity: 0.8;
}
.savebar .btn-quiet:hover:not(:disabled) {
  color: var(--surface);
  opacity: 1;
  border-color: transparent;
}

@media (max-width: 820px) {
  .group {
    grid-template-columns: 1fr;
  }
}
@media (max-width: 560px) {
  .fields.two,
  .fields.three {
    grid-template-columns: 1fr;
  }
}
</style>
