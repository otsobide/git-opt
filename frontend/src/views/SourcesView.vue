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
    toast.ok(`Importando ${describe({ kind: form.kind, name })}. Los repositorios irán apareciendo en la lista.`)
    form.name = ''
    await refresh()
  } catch (e) {
    toast.error(e)
  } finally {
    adding.value = false
  }
}

// ------------------------------------------------------------------- list

const KIND: Record<SourceKind, string> = { user: 'Usuario', org: 'Organización', me: 'Mis repositorios' }

function describe(s: Pick<Source, 'kind' | 'name'>) {
  if (s.kind === 'me') return 'todos los repositorios a los que tienes acceso'
  return s.kind === 'org' ? `la organización ${s.name}` : `los repositorios de ${s.name}`
}

const toggle = (s: Source, key: 'include_forks' | 'include_archived' | 'auto_sync') =>
  run(`${key}-${s.id}`, () => api.patchSource(s.id, { [key]: !s[key] }))

const sync = (s: Source) => run(`sync-${s.id}`, () => api.syncSource(s.id), 'Sincronización en cola')

const armed = ref<number | null>(null)
const remove = (s: Source, deleteRepos: boolean) => {
  armed.value = null
  return run(
    `delete-${s.id}`,
    () => api.deleteSource(s.id, deleteRepos),
    deleteRepos ? 'Fuente y repositorios eliminados' : 'Fuente eliminada; sus repositorios siguen en la lista',
  )
}
</script>

<template>
  <div class="page">
    <header class="page-head">
      <div>
        <h1>Fuentes</h1>
        <p class="lede">
          Una fuente es una cuenta de GitHub cuyos repositorios se vigilan todos. git-opt la vuelve a listar
          <template v-if="syncEvery">cada {{ minutes(syncEvery) }}</template>
          <template v-else>periódicamente</template>
          para añadir los repositorios nuevos.
        </p>
      </div>
    </header>

    <form class="panel add" @submit.prevent="add">
      <fieldset class="field">
        <legend class="field-label">Qué importar</legend>
        <div class="segmented">
          <label><input v-model="form.kind" type="radio" value="user" /> Usuario</label>
          <label><input v-model="form.kind" type="radio" value="org" /> Organización</label>
          <label><input v-model="form.kind" type="radio" value="me" /> Todos mis repositorios</label>
        </div>
      </fieldset>

      <div v-if="form.kind !== 'me'" class="field">
        <label for="source-name">{{ form.kind === 'org' ? 'Organización' : 'Usuario' }} de GitHub</label>
        <input
          id="source-name"
          v-model="form.name"
          class="input"
          :placeholder="form.kind === 'org' ? 'mi-empresa' : 'octocat'"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
      <p v-else class="small muted me-hint">
        <template v-if="hasMe">Ya está añadida.</template>
        <template v-else>
          Todos los repositorios a los que tiene acceso
          <strong v-if="status?.github.login">@{{ status.github.login }}</strong>
          <template v-else>el token configurado</template>: propios, de colaboración y de tus organizaciones.
        </template>
      </p>

      <div class="options">
        <label class="check"><input v-model="form.include_forks" type="checkbox" /> Incluir forks</label>
        <label class="check">
          <input v-model="form.include_archived" type="checkbox" /> Incluir archivados
        </label>
        <label class="check"><input v-model="form.auto_sync" type="checkbox" /> Añadir repositorios nuevos automáticamente</label>
      </div>
      <p class="hint">
        Los repositorios donde no puedes escribir se tratarán como contribuciones: git-opt hará un fork y abrirá la PR
        desde él. Los archivados no aceptan PRs.
      </p>

      <div>
        <button class="btn btn-primary" type="submit" :disabled="adding || !canAdd">
          {{ adding ? 'Importando…' : 'Importar repositorios' }}
        </button>
      </div>
    </form>

    <p v-if="error" class="notice danger">{{ error }}</p>

    <div v-if="sources.length" class="table-wrap">
      <table class="table">
        <thead>
          <tr>
            <th>Fuente</th>
            <th class="num">Repositorios</th>
            <th>Última sincronización</th>
            <th>Opciones</th>
            <th><span class="sr">Acciones</span></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in sources" :key="s.id">
            <td>
              <strong>{{ s.kind === 'me' ? 'Mis repositorios' : s.name }}</strong>
              <div class="hint">{{ s.kind === 'me' ? 'Todo lo accesible con el token' : KIND[s.kind] }}</div>
            </td>
            <td class="num">
              <RouterLink to="/repos">{{ s.repo_count }}</RouterLink>
            </td>
            <td>
              <span :title="formatDate(s.last_synced_at)">
                {{ s.last_synced_at ? formatRelative(s.last_synced_at, now) : 'pendiente' }}
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
                  Archivados
                </label>
                <label class="check small">
                  <input type="checkbox" :checked="s.auto_sync" :disabled="busy !== null" @change="toggle(s, 'auto_sync')" />
                  Automática
                </label>
              </div>
            </td>
            <td class="actions">
              <button type="button" class="btn btn-sm" :disabled="busy !== null" @click="sync(s)">Sincronizar</button>
              <RowMenu :label="`Más acciones para ${s.name || 'mis repositorios'}`">
                <button type="button" @click="remove(s, false)">Eliminar fuente y conservar repositorios</button>
                <button v-if="armed === s.id" type="button" class="danger" @click="remove(s, true)">
                  ¿Seguro? Eliminar {{ s.repo_count }} repositorios
                </button>
                <button v-else type="button" class="danger" @click.stop="armed = s.id">
                  Eliminar fuente y sus repositorios
                </button>
              </RowMenu>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!loading" class="empty panel">
      Sin fuentes. Importa un usuario, una organización o todos tus repositorios de una vez.
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
