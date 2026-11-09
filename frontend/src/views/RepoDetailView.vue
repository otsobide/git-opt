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
    toast.ok('Cambios guardados')
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
    force ? 'Revisión completa en cola' : 'Comprobación en cola',
  )
const publish = () => run('publish', () => api.publishRepo(props.id), 'Abriendo la PR')
const toggle = () =>
  run('toggle', async () => {
    const updated = await api.patchRepo(props.id, { enabled: !repo.value!.enabled })
    form.enabled = updated.enabled
  })
const resetIgnored = () =>
  run('reset', () => api.resetIgnored(props.id), 'Los archivos rechazados se volverán a proponer')
async function remove() {
  try {
    await api.deleteRepo(props.id)
    toast.ok(`${repo.value?.owner}/${name.value} eliminado`)
    router.push('/repos')
  } catch (e) {
    toast.error(e)
  }
}

const TRIGGER: Record<string, string> = { schedule: 'Programada', manual: 'Manual', publish: 'Publicación' }
</script>

<template>
  <div class="page">
    <nav class="crumbs small" aria-label="Ruta">
      <RouterLink to="/repos">Repositorios</RouterLink>
      <span aria-hidden="true">/</span>
      <span>{{ repo ? repo.owner : '…' }}</span>
    </nav>

    <p v-if="notFound" class="empty panel">
      Este repositorio ya no está en la lista. <RouterLink to="/repos">Volver a repositorios</RouterLink>
    </p>
    <p v-else-if="error && !repo" class="notice danger">{{ error }}</p>

    <template v-if="repo">
      <header class="page-head">
        <div class="title">
          <h1>{{ repo.name }}</h1>
          <div class="tags">
            <RepoStatusTag :repo="repo" />
            <span v-if="repo.running && repo.phase" class="muted small">{{ repo.phase }}</span>
            <span v-if="repo.mode === 'fork'" class="chip fork">contribución desde fork</span>
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
            Abrir PR
          </button>
          <button type="button" class="btn" :disabled="busy !== null || repo.running || repo.queued" @click="check()">
            Comprobar ahora
          </button>
          <button
            type="button"
            class="btn"
            :disabled="busy !== null || repo.running || repo.queued"
            title="Revisa todas las imágenes aunque la rama principal no haya cambiado"
            @click="check(true)"
          >
            Revisar todo
          </button>
          <button type="button" class="btn btn-quiet" :disabled="busy !== null" @click="toggle">
            {{ repo.enabled ? 'Pausar' : 'Reanudar' }}
          </button>
        </div>
      </header>

      <div v-if="repo.status === 'awaiting_approval'" class="notice">
        <strong>Hay una optimización preparada.</strong>
        <span>
          El commit <code>{{ shortSha(repo.pending_commit) }}</code> está listo en local. Revisa la última ejecución y
          pulsa «Abrir PR» para publicarlo.
        </span>
      </div>
      <div v-if="repo.status === 'error' && repo.last_error" class="notice danger">
        <strong>La última comprobación falló.</strong>
        <span class="mono small">{{ repo.last_error }}</span>
      </div>

      <div class="columns">
        <section class="section">
          <h2>Resumen</h2>
          <dl class="facts panel">
            <dt>En GitHub</dt>
            <dd>
              <a v-if="webUrl" :href="webUrl" target="_blank" rel="noopener">{{ repo.owner }}/{{ repo.name }}</a>
            </dd>
            <dt>Envío</dt>
            <dd>
              <template v-if="repo.mode === 'fork'">
                Contribución desde
                <a v-if="forkUrl" :href="forkUrl" target="_blank" rel="noopener">{{ repo.fork_full_name }}</a>
                <span v-else class="muted">un fork (se crea en la primera PR)</span>
              </template>
              <template v-else>Rama en el propio repositorio</template>
            </dd>
            <dt>Rama principal</dt>
            <dd><span class="chip">{{ repo.default_branch }}</span></dd>
            <dt>Último commit revisado</dt>
            <dd class="mono">{{ shortSha(repo.last_seen_sha) }}</dd>
            <dt>Última comprobación</dt>
            <dd :title="formatDate(repo.last_checked_at)">{{ formatRelative(repo.last_checked_at, now) }}</dd>
            <dt>Próxima comprobación</dt>
            <dd :title="formatDate(repo.next_check_at)">
              {{ repo.enabled ? formatRelative(repo.next_check_at, now) : 'en pausa' }}
            </dd>
            <dt>Ahorro en PRs fusionadas</dt>
            <dd>{{ formatBytes(repo.bytes_saved) }}</dd>
            <dt>Añadido</dt>
            <dd>{{ formatDate(repo.created_at) }}</dd>
          </dl>
        </section>

        <section class="section">
          <h2>Configuración</h2>
          <form class="panel settings" @submit.prevent="save">
            <label class="check">
              <input v-model="form.enabled" type="checkbox" />
              Vigilar este repositorio
            </label>
            <label class="check">
              <input v-model="form.auto_pr" type="checkbox" />
              Abrir la PR sin pedir aprobación
            </label>

            <fieldset class="field">
              <legend class="field-label">Cómo enviar los cambios</legend>
              <div class="segmented">
                <label><input v-model="form.mode" type="radio" value="direct" /> Rama en el repositorio</label>
                <label><input v-model="form.mode" type="radio" value="fork" /> Fork y contribución</label>
              </div>
            </fieldset>

            <fieldset class="field">
              <legend class="field-label">Frecuencia de comprobación</legend>
              <label class="check">
                <input v-model="form.useGlobalInterval" type="checkbox" />
                Usar el intervalo global
                <span v-if="globals" class="muted">
                  ({{ minutes(globals.poll_interval_minutes) }} + hasta {{ minutes(globals.poll_jitter_minutes) }} al azar)
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
                  aria-label="Minutos entre comprobaciones"
                />
                <span class="small muted">minutos, más el margen aleatorio global</span>
              </div>
            </fieldset>

            <div class="field">
              <label for="ignore">Archivos ignorados</label>
              <textarea
                id="ignore"
                v-model="form.ignore_globs"
                class="textarea"
                spellcheck="false"
                placeholder="docs/screenshots/**&#10;**/*.min.png"
              ></textarea>
              <span class="hint">Un patrón por línea. Se suman a los patrones globales de Ajustes.</span>
            </div>

            <div class="form-actions">
              <button class="btn btn-primary" type="submit" :disabled="!dirty || saving">
                {{ saving ? 'Guardando…' : 'Guardar cambios' }}
              </button>
              <button v-if="dirty" class="btn btn-quiet" type="button" @click="repo && loadForm(repo)">
                Descartar
              </button>
            </div>
          </form>
        </section>
      </div>

      <section class="section">
        <div class="section-head">
          <h2>Historial</h2>
          <span v-if="runs.length > 1" class="muted small">últimas {{ runs.length }} ejecuciones</span>
        </div>
        <div v-if="runs.length" class="table-wrap">
          <table class="table">
            <thead>
              <tr>
                <th>Cuándo</th>
                <th>Origen</th>
                <th>Resultado</th>
                <th class="num">Imágenes</th>
                <th>Ahorro</th>
                <th>PR</th>
                <th class="num">Duración</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="r in runs" :key="r.id" class="clickable" @click="router.push(`/actividad/${r.id}`)">
                <td class="nowrap" :title="formatDate(r.started_at)">
                  <RouterLink :to="`/actividad/${r.id}`" @click.stop>{{ formatRelative(r.started_at, now) }}</RouterLink>
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
        <p v-else-if="!loading" class="empty panel">Este repositorio todavía no se ha comprobado.</p>
      </section>

      <section class="section danger-zone">
        <h2>Mantenimiento</h2>
        <div class="panel maint">
          <div>
            <p><strong>Volver a proponer archivos rechazados</strong></p>
            <p class="small muted">
              Cuando cierras una PR sin fusionarla, git-opt no vuelve a tocar esas imágenes. Esto lo olvida.
            </p>
          </div>
          <button type="button" class="btn" :disabled="busy !== null" @click="resetIgnored">Olvidar rechazos</button>
        </div>
        <div class="panel maint">
          <div>
            <p><strong>Eliminar de la lista</strong></p>
            <p class="small muted">
              Borra el clon local y el historial de ejecuciones. No toca el repositorio en GitHub ni las PRs abiertas.
            </p>
          </div>
          <ConfirmButton label="Eliminar repositorio" confirm-label="¿Seguro? Eliminar" @confirm="remove" />
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
