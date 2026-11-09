// Typed client for the git-opt backend. Every call goes through `request`,
// which adds the admin token (when configured) and normalizes errors.

import { ref } from 'vue'
import type {
  NewSource,
  RepoPatch,
  RepoView,
  Run,
  RunDetail,
  Settings,
  SettingsResponse,
  Source,
  Status,
} from './types'

const TOKEN_KEY = 'git-opt.admin-token'

function readToken(): string | null {
  try {
    return localStorage.getItem(TOKEN_KEY)
  } catch {
    return null
  }
}

/** Admin token in memory, mirrored to localStorage when available. */
export const adminToken = ref<string | null>(readToken())

/** Becomes true when the backend answers 401: the app shows the login screen. */
export const authRequired = ref(false)

export function setAdminToken(token: string | null) {
  adminToken.value = token
  try {
    if (token) localStorage.setItem(TOKEN_KEY, token)
    else localStorage.removeItem(TOKEN_KEY)
  } catch {
    // Storage blocked (private mode…): the token lives only in memory.
  }
}

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' }
  if (body !== undefined) headers['Content-Type'] = 'application/json'
  if (adminToken.value) headers.Authorization = `Bearer ${adminToken.value}`

  let res: Response
  try {
    res = await fetch(`/api${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch {
    throw new ApiError(0, 'Cannot connect to the git-opt server.')
  }

  if (res.status === 401) {
    authRequired.value = true
    throw new ApiError(401, 'The admin token is required.')
  }
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`
    try {
      const data = await res.json()
      if (data && typeof data.error === 'string') message = data.error
    } catch {
      // Body was not JSON; keep the HTTP status text.
    }
    throw new ApiError(res.status, message)
  }
  if (res.status === 202 || res.status === 204) return undefined as T
  const text = await res.text()
  return (text ? JSON.parse(text) : undefined) as T
}

const enc = encodeURIComponent

export const api = {
  auth: () => request<{ required: boolean }>('GET', '/auth'),
  status: () => request<Status>('GET', '/status'),

  settings: () => request<SettingsResponse>('GET', '/settings'),
  saveSettings: (s: Settings) => request<Settings>('PUT', '/settings', s),
  setGithubToken: (token: string | null) => request<void>('PUT', '/settings/github-token', { token }),

  repos: () => request<RepoView[]>('GET', '/repos'),
  repo: (id: number) => request<RepoView>('GET', `/repos/${id}`),
  addRepo: (repo: string, mode: 'auto' | 'direct' | 'fork', autoPr: boolean) =>
    request<RepoView>('POST', '/repos', { repo, mode, auto_pr: autoPr }),
  patchRepo: (id: number, patch: RepoPatch) => request<RepoView>('PATCH', `/repos/${id}`, patch),
  deleteRepo: (id: number) => request<void>('DELETE', `/repos/${id}`),
  checkRepo: (id: number, force = false) => request<void>('POST', `/repos/${id}/check`, { force }),
  checkAll: () => request<void>('POST', '/repos/check-all'),
  publishRepo: (id: number) => request<void>('POST', `/repos/${id}/publish`),
  resetIgnored: (id: number) => request<void>('POST', `/repos/${id}/reset-ignored`),
  repoRuns: (id: number, limit = 50) => request<Run[]>('GET', `/repos/${id}/runs?limit=${limit}`),

  runs: (limit = 100) => request<Run[]>('GET', `/runs?limit=${limit}`),
  run: (id: number) => request<RunDetail>('GET', `/runs/${id}`),

  sources: () => request<Source[]>('GET', '/sources'),
  addSource: (s: NewSource) => request<Source>('POST', '/sources', s),
  patchSource: (id: number, patch: Partial<Pick<Source, 'include_forks' | 'include_archived' | 'auto_sync'>>) =>
    request<Source>('PATCH', `/sources/${id}`, patch),
  deleteSource: (id: number, deleteRepos: boolean) =>
    request<void>('DELETE', `/sources/${id}?delete_repos=${enc(String(deleteRepos))}`),
  syncSource: (id: number) => request<void>('POST', `/sources/${id}/sync`),
}
