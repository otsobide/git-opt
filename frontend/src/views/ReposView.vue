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
    toast.ok(`${repo.owner}/${repo.name} añadido. La primera comprobación ya está en cola.`)
    newRepo.value = ''
    await refresh()
  } catch (e) {
    toast.error(e)
  } finally {
    adding.value = false
  }
}

// ----------------------------------------------------------------- filters

type Filter = 'todos' | 'pr' | 'aprobacion' | 'errores' | 'pausa'
const FILTERS: { id: Filter; label: string; test: (r: RepoView) => boolean }[] = [
  { id: 'todos', label: 'Todos', test: () => true },
  { id: 'pr', label: 'Con PR abierta', test: (r) => r.pr_state === 'open' },
  { id: 'aprobacion', label: 'Esperando aprobación', test: (r) => r.status === 'awaiting_approval' },
  { id: 'errores', label: 'Con errores', test: (r) => r.status === 'error' },
  { id: 'pausa', label: 'En pausa', test: (r) => !r.enabled },
]

const filter = computed<Filter>({
  get: () => {
    const q = route.query.filtro
    return FILTERS.some((f) => f.id === q) ? (q as Filter) : 'todos'
  },
  set: (v) => router.replace({ query: { ...route.query, filtro: v === 'todos' ? undefined : v } }),
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
  if (r.running) return 'ahora'
  if (r.queued) return 'en cola'
  if (!r.enabled) return 'en pausa'
  return formatRelative(r.next_check_at, now.value)
}

// ----------------------------------------------------------------- actions

const check = (r: RepoView, force = false) =>
  run(
    `check-${r.id}`,
    () => api.checkRepo(r.id, force),
    force ? `Revisión completa de ${r.name} en cola` : `Comprobación de ${r.name} en cola`,
  )
const toggle = (r: RepoView) =>
  run(
    `toggle-${r.id}`,
    () => api.patchRepo(r.id, { enabled: !r.enabled }),
    r.enabled ? `${r.name} en pausa` : `${r.name} reanudado`,
  )
const publish = (r: RepoView) => run(`publish-${r.id}`, () => api.publishRepo(r.id), `Abriendo la PR de ${r.name}`)
// Deleting from the row menu takes two clicks: the first arms the item.
const armedDelete = ref<number | null>(null)
const remove = (r: RepoView) => {
  armedDelete.value = null
  return run(`delete-${r.id}`, () => api.deleteRepo(r.id), `${r.owner}/${r.name} eliminado`)
}
const checkAll = () => run('all', api.checkAll, 'Comprobación de todos los repositorios en cola')
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Repositorios</h1>
        <p class="lede">
          Cada repositorio se descarga y se revisa periódicamente. Si cambia su rama principal, git-opt optimiza las
          imágenes nuevas y propone los cambios en una PR.
        </p>
      </div>
      <div class="toolbar">
        <button type="button" class="btn" :disabled="busy !== null || !repos.length" @click="checkAll">
          Comprobar todos
        </button>
      </div>
    </header>

    <form class="add panel" @submit.prevent="add">
      <div class="field grow">
        <label for="new-repo">Añadir repositorio</label>
        <input
          id="new-repo"
          v-model="newRepo"
          class="input"
          placeholder="propietario/nombre o https://github.com/propietario/nombre"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
      <div class="field">
        <label for="new-mode">Cómo enviar los cambios</label>
        <select id="new-mode" v-model="newMode" class="select">
          <option value="auto">Automático según permisos</option>
          <option value="direct">Rama en el propio repositorio</option>
          <option value="fork">Contribución desde un fork</option>
        </select>
      </div>
      <label class="check">
        <input v-model="newAutoPr" type="checkbox" />
        Abrir la PR sin pedir aprobación
      </label>
      <button class="btn btn-primary" type="submit" :disabled="adding || !newRepo.trim()">
        {{ adding ? 'Añadiendo…' : 'Añadir' }}
      </button>
      <p class="hint full">
        Para repositorios de terceros, el modo automático crea un fork en tu cuenta y abre la PR como contribución.
        ¿Quieres todos los repositorios de una cuenta? Usa <RouterLink to="/fuentes">Fuentes</RouterLink>.
      </p>
    </form>

    <div class="filters">
      <div class="segmented" role="radiogroup" aria-label="Filtrar repositorios">
        <label v-for="f in FILTERS" :key="f.id">
          <input v-model="filter" type="radio" name="filter" :value="f.id" />
          {{ f.label }} <span class="count">{{ counts[f.id] }}</span>
        </label>
      </div>
      <input v-model="search" class="input search" type="search" placeholder="Buscar" aria-label="Buscar repositorio" />
    </div>

    <p v-if="error" class="notice danger">{{ error }}</p>

    <div v-if="visible.length" class="table-wrap">
      <table class="table repos">
        <thead>
          <tr>
            <th>Repositorio</th>
            <th>Estado</th>
            <th>PR</th>
            <th>Próxima comprobación</th>
            <th><span class="visually-hidden">Acciones</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="r in visible" :key="r.id" :class="{ paused: !r.enabled }">
            <td data-label="Repositorio">
              <RouterLink :to="`/repos/${r.id}`" class="name">
                <span class="owner">{{ r.owner }}/</span>{{ r.name }}
              </RouterLink>
              <div class="meta">
                <span class="chip">{{ r.default_branch }}</span>
                <span v-if="r.mode === 'fork'" class="chip fork">contribución</span>
                <span v-if="!r.auto_pr" class="chip">aprobación manual</span>
              </div>
            </td>
            <td data-label="Estado">
              <RepoStatusTag :repo="r" />
              <div v-if="r.running && r.phase" class="hint">{{ r.phase }}</div>
              <div v-else-if="r.status === 'error' && r.last_error" class="error-text clamp" :title="r.last_error">
                {{ r.last_error }}
              </div>
            </td>
            <td data-label="PR">
              <PrLink :number="r.pr_number" :url="r.pr_url" :state="r.pr_state" />
            </td>
            <td data-label="Próxima" class="nowrap">
              {{ nextCheck(r) }}
              <div v-if="r.last_checked_at" class="hint">última {{ formatRelative(r.last_checked_at, now) }}</div>
            </td>
            <td class="actions">
              <button
                v-if="r.status === 'awaiting_approval' && !r.running"
                type="button"
                class="btn btn-primary btn-sm"
                :disabled="busy !== null"
                @click="publish(r)"
              >
                Abrir PR
              </button>
              <button
                type="button"
                class="btn btn-sm"
                :disabled="busy !== null || r.running || r.queued"
                @click="check(r)"
              >
                Comprobar
              </button>
              <RowMenu :label="`Más acciones para ${r.name}`">
                <button type="button" :disabled="r.running || r.queued" @click="check(r, true)">
                  Revisar todo aunque no haya cambios
                </button>
                <button type="button" @click="toggle(r)">{{ r.enabled ? 'Pausar' : 'Reanudar' }}</button>
                <RouterLink :to="`/repos/${r.id}`">Configurar</RouterLink>
                <hr />
                <button v-if="armedDelete === r.id" type="button" class="danger" @click="remove(r)">
                  ¿Seguro? Eliminar {{ r.name }}
                </button>
                <button v-else type="button" class="danger" @click.stop="armedDelete = r.id">
                  Eliminar de la lista
                </button>
              </RowMenu>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!loading && repos.length" class="empty panel">Ningún repositorio coincide con el filtro.</p>
    <p v-else-if="!loading" class="empty panel">
      Aún no hay repositorios. Escribe <code>propietario/nombre</code> arriba para empezar.
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
  .repos td[data-label='Estado'],
  .repos td[data-label='PR'],
  .repos td[data-label='Próxima'] {
    display: inline-block;
    margin-right: 1rem;
  }
  .repos .actions {
    text-align: left;
    padding-top: 0.5rem;
  }
}
</style>
