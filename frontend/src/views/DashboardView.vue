<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterLink, useRouter } from 'vue-router'
import { api } from '@/api'
import { usePolling } from '@/composables/usePolling'
import { useStatus } from '@/composables/useStatus'
import { useNow } from '@/composables/useNow'
import { useAction } from '@/composables/useAction'
import { formatBytes, formatNumber, formatRelative } from '@/format'
import type { RepoView, Run } from '@/types'
import SizeBar from '@/components/SizeBar.vue'
import RunStatusTag from '@/components/RunStatusTag.vue'

const router = useRouter()
const { status, refreshStatus } = useStatus()
const now = useNow()

const repos = ref<RepoView[]>([])
const runs = ref<Run[]>([])

const { loading, error } = usePolling(async () => {
  const [r, rs] = await Promise.all([api.repos(), api.runs(10)])
  repos.value = r
  runs.value = rs
})

const { busy, run } = useAction(refreshStatus)

const totals = computed(() => status.value?.totals)
const jobs = computed(() => status.value?.jobs)

const watched = computed(() => repos.value.filter((r) => r.enabled).length)

const nextUp = computed(() => {
  const due = repos.value
    .filter((r) => r.enabled && r.next_check_at && !r.running && !r.queued)
    .sort((a, b) => (a.next_check_at! < b.next_check_at! ? -1 : 1))
  return due[0] ?? null
})

const headline = computed(() => {
  if (!repos.value.length) return 'Aún no vigilas ningún repositorio'
  const n = watched.value
  return n === 1 ? 'Vigilando 1 repositorio' : `Vigilando ${formatNumber(n)} repositorios`
})

const paused = computed(() => repos.value.length - watched.value)

const missingOptimizers = computed(() => (status.value?.optimizers ?? []).filter((o) => o.enabled && !o.available))

const identitySource: Record<string, string> = {
  settings: 'definida en Ajustes',
  git: 'de git config',
  github: 'de tu cuenta de GitHub',
}
const identityHint = computed(() => {
  const g = status.value?.git
  if (!g?.identity) return ''
  const source = g.identity_source ? identitySource[g.identity_source] : null
  return source ? `${g.identity.email}, ${source}` : g.identity.email
})
const tokenSource: Record<string, string> = {
  settings: 'token guardado en Ajustes',
  env: 'token de variable de entorno',
  gh: 'sesión de gh CLI',
}
</script>

<template>
  <div class="page">
    <header class="hero">
      <h1>{{ headline }}</h1>
      <p v-if="jobs?.running.length" class="now">
        Ahora procesa
        <template v-for="(j, i) in jobs.running" :key="j.repo_id">
          <span v-if="i > 0">{{ i === jobs.running.length - 1 ? ' y ' : ', ' }}</span>
          <RouterLink :to="`/repos/${j.repo_id}`">{{ j.full_name }}</RouterLink>
          <span v-if="j.phase" class="muted"> ({{ j.phase }})</span>
        </template>.
        <span v-if="jobs.queued.length" class="muted">{{ jobs.queued.length }} más en cola.</span>
      </p>
      <p v-else-if="nextUp" class="now">
        Próxima comprobación:
        <RouterLink :to="`/repos/${nextUp.id}`">{{ nextUp.owner }}/{{ nextUp.name }}</RouterLink>,
        {{ formatRelative(nextUp.next_check_at, now) }}.
        <span v-if="paused" class="muted">{{ paused }} en pausa.</span>
      </p>
      <p v-else-if="!repos.length && !loading" class="now">
        Añade un repositorio o una cuenta entera y git-opt empezará a buscar imágenes que comprimir sin perder un solo píxel.
      </p>
      <div class="hero-actions">
        <RouterLink to="/repos" class="btn btn-primary">{{ repos.length ? 'Ver repositorios' : 'Añadir repositorio' }}</RouterLink>
        <RouterLink v-if="!repos.length" to="/sources" class="btn">Importar usuario u organización</RouterLink>
        <button
          v-else
          type="button"
          class="btn"
          :disabled="busy !== null"
          @click="run('all', api.checkAll, 'Comprobación de todos los repositorios en cola')"
        >
          Comprobar todos ahora
        </button>
      </div>
    </header>

    <p v-if="error" class="notice danger">{{ error }}</p>

    <div v-if="status" class="notices">
      <div v-if="!status.github.ok" class="notice danger">
        <strong>git-opt no puede hablar con GitHub.</strong>
        <span>{{ status.github.error || 'No hay token.' }}</span>
        <span>
          Define <code>GITHUB_TOKEN</code>, inicia sesión con <code>gh auth login</code> o guarda un token en
          <RouterLink to="/settings">Ajustes</RouterLink>.
        </span>
      </div>
      <div v-if="status.git.error" class="notice">
        <strong>Problema con git.</strong>
        <span>{{ status.git.error }}</span>
      </div>
      <div v-if="missingOptimizers.length" class="notice">
        <strong>Faltan herramientas.</strong>
        <span v-for="o in missingOptimizers" :key="o.id">
          {{ o.label }}: {{ o.detail }}. Los archivos {{ o.extensions.map((e) => '.' + e).join(', ') }} se saltan.
        </span>
      </div>
    </div>

    <dl v-if="totals" class="stats">
      <div>
        <dt>PRs abiertas</dt>
        <dd>{{ formatNumber(totals.prs_open) }}</dd>
      </div>
      <div :class="{ attention: totals.awaiting_approval > 0 }">
        <dt>Esperan tu aprobación</dt>
        <dd>
          <RouterLink v-if="totals.awaiting_approval" :to="{ path: '/repos', query: { filtro: 'aprobacion' } }">
            {{ formatNumber(totals.awaiting_approval) }}
          </RouterLink>
          <template v-else>0</template>
        </dd>
      </div>
      <div :class="{ failing: totals.errors > 0 }">
        <dt>Con errores</dt>
        <dd>
          <RouterLink v-if="totals.errors" :to="{ path: '/repos', query: { filtro: 'errores' } }">
            {{ formatNumber(totals.errors) }}
          </RouterLink>
          <template v-else>0</template>
        </dd>
      </div>
      <div>
        <dt>Ahorrado en PRs fusionadas</dt>
        <dd>{{ formatBytes(totals.bytes_saved) }}</dd>
      </div>
      <div>
        <dt>PRs enviadas</dt>
        <dd>{{ formatNumber(totals.prs_sent) }}</dd>
      </div>
    </dl>

    <div class="columns">
      <section class="section">
        <div class="section-head">
          <h2>Actividad reciente</h2>
          <RouterLink to="/activity" class="small">Ver toda la actividad</RouterLink>
        </div>
        <div v-if="runs.length" class="table-wrap">
          <table class="table">
            <thead>
              <tr>
                <th>Repositorio</th>
                <th>Resultado</th>
                <th>Ahorro</th>
                <th class="num">Cuándo</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="r in runs" :key="r.id" class="clickable" @click="router.push(`/activity/${r.id}`)">
                <td>
                  <RouterLink :to="`/activity/${r.id}`" @click.stop>{{ r.repo_full_name }}</RouterLink>
                </td>
                <td><RunStatusTag :status="r.status" /></td>
                <td class="saving">
                  <SizeBar v-if="r.files_optimized" :before="r.bytes_before" :after="r.bytes_after" />
                  <span v-else class="muted small">—</span>
                </td>
                <td class="num muted nowrap">{{ formatRelative(r.started_at, now) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p v-else-if="!loading" class="empty panel">Todavía no ha habido ninguna ejecución.</p>
      </section>

      <aside class="side">
        <section v-if="status" class="section">
          <h3>Identidad</h3>
          <dl class="facts">
            <dt>PRs</dt>
            <dd>
              <template v-if="status.github.ok">
                @{{ status.github.login }}
                <span v-if="status.github.name" class="muted">({{ status.github.name }})</span>
                <div v-if="status.github.token_source" class="hint">{{ tokenSource[status.github.token_source] }}</div>
              </template>
              <span v-else class="tag danger">sin acceso</span>
            </dd>
            <dt>Commits</dt>
            <dd>
              <template v-if="status.git.identity">
                {{ status.git.identity.name }}
                <div class="hint">{{ identityHint }}</div>
              </template>
              <span v-else class="tag warn">sin identidad</span>
            </dd>
            <dt>git</dt>
            <dd class="muted">{{ status.git.version || '—' }}</dd>
            <dt>Versión</dt>
            <dd class="muted">git-opt {{ status.version }}</dd>
          </dl>
        </section>

        <section v-if="status" class="section">
          <h3>Optimizadores</h3>
          <ul class="optimizers">
            <li v-for="o in status.optimizers" :key="o.id">
              <span
                class="tag"
                :class="!o.enabled ? 'hollow' : o.available ? 'ok' : 'danger'"
              >{{ o.label }}</span>
              <span class="hint">
                {{ !o.enabled ? 'desactivado' : o.available ? o.detail : 'no disponible' }}
              </span>
            </li>
          </ul>
          <RouterLink to="/settings" class="small">Configurar</RouterLink>
        </section>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.hero {
  display: grid;
  gap: 0.75rem;
  padding-bottom: 0.5rem;
}
.hero h1 {
  font-size: var(--fs-3xl);
  font-weight: 600;
  letter-spacing: -0.02em;
}
.now {
  font-size: var(--fs-lg);
  max-width: 62ch;
}
.hero-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-top: 0.25rem;
}
.notices {
  display: grid;
  gap: 0.5rem;
}
.notices:empty {
  display: none;
}

.stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(9.5rem, 1fr));
  margin: 0;
  border-top: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
}
.stats > div {
  padding: 0.85rem 1rem 0.9rem;
}
.stats > div + div {
  border-left: 1px solid var(--line);
}
.stats dt {
  font-size: var(--fs-xs);
  color: var(--muted);
}
.stats dd {
  margin: 0.15rem 0 0;
  font-family: var(--font-display);
  font-size: var(--fs-xl);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.stats .attention dd,
.stats .attention a {
  color: var(--warn);
}
.stats .failing dd,
.stats .failing a {
  color: var(--danger);
}

.columns {
  display: grid;
  grid-template-columns: minmax(0, 2.2fr) minmax(15rem, 1fr);
  gap: 2rem;
  align-items: start;
}
.side {
  display: grid;
  gap: 1.75rem;
}
.saving {
  width: 34%;
}
.optimizers {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 0.5rem;
}
.optimizers li {
  display: grid;
  gap: 0.1rem;
}
.optimizers .tag {
  font-size: var(--fs-sm);
  color: var(--ink);
}
.optimizers .tag.ok::before {
  background: var(--accent);
}
.optimizers .tag.danger::before {
  background: var(--danger);
}
.optimizers .tag.hollow::before {
  color: var(--faint);
}

@media (max-width: 960px) {
  .columns {
    grid-template-columns: 1fr;
  }
}
@media (max-width: 560px) {
  .hero h1 {
    font-size: var(--fs-2xl);
  }
  .stats > div + div {
    border-left: 0;
  }
  .stats {
    grid-template-columns: 1fr 1fr;
  }
}
</style>
