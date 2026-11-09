import { createRouter, createWebHistory } from 'vue-router'
import DashboardView from './views/DashboardView.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', name: 'dashboard', component: DashboardView, meta: { title: 'Dashboard' } },
    {
      path: '/repos',
      name: 'repos',
      component: () => import('./views/ReposView.vue'),
      meta: { title: 'Repositories' },
    },
    {
      path: '/repos/:id(\\d+)',
      name: 'repo',
      component: () => import('./views/RepoDetailView.vue'),
      props: (route) => ({ id: Number(route.params.id) }),
      meta: { title: 'Repository' },
    },
    {
      path: '/sources',
      name: 'sources',
      component: () => import('./views/SourcesView.vue'),
      meta: { title: 'Sources' },
    },
    {
      path: '/activity',
      name: 'runs',
      component: () => import('./views/RunsView.vue'),
      meta: { title: 'Activity' },
    },
    {
      path: '/activity/:id(\\d+)',
      name: 'run',
      component: () => import('./views/RunDetailView.vue'),
      props: (route) => ({ id: Number(route.params.id) }),
      meta: { title: 'Run' },
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('./views/SettingsView.vue'),
      meta: { title: 'Settings' },
    },
    { path: '/:pathMatch(.*)*', redirect: '/' },
  ],
  scrollBehavior: () => ({ top: 0 }),
})

router.afterEach((to) => {
  const title = to.meta.title as string | undefined
  document.title = title ? `${title} · git-opt` : 'git-opt'
})
