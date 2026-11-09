// Mirrors the JSON returned by the Rust backend (see backend/src/models.rs).

export type RepoMode = 'direct' | 'fork'
export type RepoStatus = 'pending' | 'ok' | 'awaiting_approval' | 'error'
export type PrState = 'open' | 'merged' | 'closed'

export interface Repo {
  id: number
  owner: string
  name: string
  source_id: number | null
  mode: RepoMode
  clone_url: string
  default_branch: string
  fork_full_name: string | null
  fork_clone_url: string | null
  enabled: boolean
  auto_pr: boolean
  poll_interval_minutes: number | null
  ignore_globs: string
  status: RepoStatus
  last_error: string | null
  last_seen_sha: string | null
  last_checked_at: string | null
  next_check_at: string | null
  pending_commit: string | null
  pr_number: number | null
  pr_url: string | null
  pr_state: PrState | null
  bytes_saved: number
  created_at: string
}

export interface RepoView extends Repo {
  running: boolean
  queued: boolean
  phase: string | null
}

export interface RepoPatch {
  enabled?: boolean
  auto_pr?: boolean
  mode?: RepoMode
  poll_interval_minutes?: number | null
  ignore_globs?: string
}

export type SourceKind = 'user' | 'org' | 'me'

export interface Source {
  id: number
  kind: SourceKind
  name: string
  include_forks: boolean
  include_archived: boolean
  auto_sync: boolean
  repo_count: number
  last_synced_at: string | null
  last_error: string | null
  created_at: string
}

export interface NewSource {
  kind: SourceKind
  name: string
  include_forks: boolean
  include_archived: boolean
  auto_sync: boolean
}

export type RunTrigger = 'schedule' | 'manual' | 'publish'
export type RunStatus =
  | 'running'
  | 'no_changes'
  | 'up_to_date'
  | 'awaiting_approval'
  | 'pr_opened'
  | 'pr_updated'
  | 'error'

export interface Run {
  id: number
  repo_id: number
  repo_full_name: string | null
  trigger: RunTrigger
  status: RunStatus
  started_at: string
  finished_at: string | null
  head_sha: string | null
  commit_sha: string | null
  files_scanned: number
  files_optimized: number
  bytes_before: number
  bytes_after: number
  pr_number: number | null
  pr_url: string | null
  message: string | null
}

export interface RunFile {
  id: number
  run_id: number
  path: string
  optimizer: string
  original_blob: string
  bytes_before: number
  bytes_after: number
}

export interface RunDetail {
  run: Run
  files: RunFile[]
}

export interface Settings {
  poll_interval_minutes: number
  poll_jitter_minutes: number
  max_concurrent_jobs: number
  source_sync_interval_minutes: number
  min_savings_percent: number
  min_savings_bytes: number
  max_file_size_mb: number
  png_level: number
  strip_metadata: boolean
  optimizers: Record<string, boolean>
  ignore_globs: string
  branch_name: string
  commit_title: string
  commit_author_name: string
  commit_author_email: string
  sign_commits: boolean
}

export interface SettingsResponse {
  settings: Settings
  github_token_set: boolean
}

export interface OptimizerStatus {
  id: string
  label: string
  category: string
  extensions: string[]
  enabled: boolean
  available: boolean
  detail: string
}

export interface Totals {
  repos: number
  repos_enabled: number
  prs_open: number
  awaiting_approval: number
  errors: number
  bytes_saved: number
  prs_sent: number
}

export interface Identity {
  name: string
  email: string
}

export interface RunningJob {
  repo_id: number
  full_name: string
  phase: string | null
  started_at: string
}

export interface Status {
  version: string
  github: {
    ok: boolean
    login?: string | null
    name?: string | null
    token_source?: 'settings' | 'env' | 'gh' | null
    error?: string | null
  }
  git: {
    version?: string | null
    identity?: Identity | null
    identity_source?: 'settings' | 'git' | 'github' | null
    error?: string | null
  }
  optimizers: OptimizerStatus[]
  jobs: {
    running: RunningJob[]
    queued: number[]
  }
  totals: Totals
}
