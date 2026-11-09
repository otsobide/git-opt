<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink, RouterView, useRoute } from 'vue-router'
import { api, adminToken, authRequired } from '@/api'
import { usePolling } from '@/composables/usePolling'
import { useStatus } from '@/composables/useStatus'
import BrandMark from '@/components/BrandMark.vue'
import LoginGate from '@/components/LoginGate.vue'
import ToastHost from '@/components/ToastHost.vue'

const { status, statusError, refreshStatus } = useStatus()
const ready = ref(false)

onMounted(async () => {
  try {
    const { required } = await api.auth()
    if (required && !adminToken.value) authRequired.value = true
  } catch {
    // The status poll will surface connection problems.
  }
  ready.value = true
})

usePolling(async () => {
  if (!authRequired.value) await refreshStatus()
}, 5000)

const nav = [
  { to: '/', label: 'Dashboard' },
  { to: '/repos', label: 'Repositories' },
  { to: '/sources', label: 'Sources' },
  { to: '/activity', label: 'Activity' },
  { to: '/settings', label: 'Settings' },
]

const route = useRoute()
function isActive(to: string) {
  return to === '/' ? route.path === '/' : route.path === to || route.path.startsWith(`${to}/`)
}

const running = computed(() => status.value?.jobs.running.length ?? 0)
const queued = computed(() => status.value?.jobs.queued.length ?? 0)
const github = computed(() => status.value?.github)
const jobsText = computed(() =>
  [running.value && `${running.value} running`, queued.value && `${queued.value} queued`].filter(Boolean).join(', '),
)
</script>

<template>
  <LoginGate v-if="ready && authRequired" @ok="refreshStatus" />
  <div v-else-if="ready" class="shell">
    <aside class="sidebar">
      <RouterLink to="/" class="home" aria-label="git-opt, go to dashboard"><BrandMark /></RouterLink>
      <nav aria-label="Sections">
        <RouterLink
          v-for="item in nav"
          :key="item.to"
          :to="item.to"
          class="nav-link"
          :class="{ active: isActive(item.to) }"
          :aria-current="isActive(item.to) ? 'page' : undefined"
        >
          {{ item.label }}
        </RouterLink>
      </nav>
      <div class="whoami small">
        <p v-if="statusError" class="error-text">Cannot reach the server</p>
        <template v-else-if="status">
          <p v-if="running" class="tag ok live">{{ jobsText }}</p>
          <p v-else-if="queued" class="tag hollow">{{ jobsText }}</p>
          <p v-else class="tag hollow">Idle</p>
          <p v-if="github?.ok && github.login" class="muted">PRs as <strong>@{{ github.login }}</strong></p>
          <p v-else class="tag warn">No GitHub access</p>
        </template>
      </div>
    </aside>
    <main class="content">
      <RouterView />
    </main>
  </div>
  <ToastHost />
</template>

<style scoped>
.shell {
  display: grid;
  grid-template-columns: var(--sidebar) minmax(0, 1fr);
  min-height: 100vh;
}
.sidebar {
  position: sticky;
  top: 0;
  height: 100vh;
  display: flex;
  flex-direction: column;
  gap: 1.75rem;
  padding: 1.25rem 0.75rem 1rem;
  border-right: 1px solid var(--line);
}
.home {
  color: var(--ink);
  text-decoration: none;
  padding: 0 0.5rem;
}
nav {
  display: grid;
  gap: 0.1rem;
}
.nav-link {
  padding: 0.4rem 0.6rem;
  border-radius: var(--radius-sm);
  color: var(--muted);
  text-decoration: none;
  font-weight: 500;
  font-size: var(--fs-sm);
}
.nav-link:hover {
  color: var(--ink);
  background: var(--surface-2);
}
.nav-link.active {
  color: var(--ink);
  background: var(--surface);
  box-shadow: inset 2px 0 0 var(--accent);
}
.whoami {
  margin-top: auto;
  display: grid;
  gap: 0.35rem;
  padding: 0 0.5rem;
}
.whoami strong {
  color: var(--ink);
  font-weight: 500;
}
.content {
  padding: 2rem clamp(1rem, 3vw, 2.5rem) 4rem;
  min-width: 0;
}

@media (max-width: 760px) {
  .shell {
    grid-template-columns: minmax(0, 1fr);
  }
  .sidebar {
    position: static;
    height: auto;
    flex-direction: row;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 1rem;
    padding: 0.75rem 1rem;
    border-right: 0;
    border-bottom: 1px solid var(--line);
  }
  nav {
    display: flex;
    gap: 0.15rem;
    overflow-x: auto;
    width: 100%;
    order: 3;
  }
  .nav-link {
    white-space: nowrap;
  }
  .nav-link.active {
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .whoami {
    margin: 0 0 0 auto;
    padding: 0;
    justify-items: end;
  }
  .content {
    padding: 1.25rem 1rem 3rem;
  }
}
</style>
