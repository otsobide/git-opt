import { createRouter, createWebHistory } from 'vue-router'
import DashboardView from './views/DashboardView.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', name: 'dashboard', component: DashboardView, meta: { title: 'Panel' } },
    {
      path: '/repos',
      name: 'repos',
      component: () => import('./views/ReposView.vue'),
      meta: { title: 'Repositorios' },
    },
    {
      path: '/repos/:id(\\d+)',
      name: 'repo',
      component: () => import('./views/RepoDetailView.vue'),
      props: (route) => ({ id: Number(route.params.id) }),
      meta: { title: 'Repositorio' },
    },
    {
      path: '/sources',
      name: 'sources',
      component: () => import('./views/SourcesView.vue'),
      meta: { title: 'Fuentes' },
    },
    {
      path: '/activity',
      name: 'runs',
      component: () => import('./views/RunsView.vue'),
      meta: { title: 'Actividad' },
    },
    {
      path: '/activity/:id(\\d+)',
      name: 'run',
      component: () => import('./views/RunDetailView.vue'),
      props: (route) => ({ id: Number(route.params.id) }),
      meta: { title: 'Ejecución' },
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('./views/SettingsView.vue'),
      meta: { title: 'Ajustes' },
    },
    { path: '/:pathMatch(.*)*', redirect: '/' },
  ],
  scrollBehavior: () => ({ top: 0 }),
})

router.afterEach((to) => {
  const title = to.meta.title as string | undefined
  document.title = title ? `${title} · git-opt` : 'git-opt'
})
