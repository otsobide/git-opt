# HTTP API

The web UI is a client of this JSON API; you can script against it too. Base path: `/api`.

- Requests and responses are JSON. Timestamps are RFC 3339 strings in UTC.
- Errors use the HTTP status and a body `{ "error": "message" }`.
- If `GIT_OPT_ADMIN_TOKEN` is set, every endpoint except `GET /api/health` and `GET /api/auth` needs `Authorization: Bearer <token>`; otherwise `401`.
- Actions that start background work answer `202 Accepted` with no body; follow progress with `GET /api/repos/{id}` (`running`, `queued`, `phase`) or `GET /api/status`.

```bash
curl -s -H "Authorization: Bearer $GIT_OPT_ADMIN_TOKEN" localhost:8484/api/repos | jq
```

## Service

### `GET /api/health`
`200 ok` (plain text). No auth.

### `GET /api/auth`
`{ "required": true }` when an admin token is configured. No auth.

### `GET /api/status`
Everything the Dashboard needs:

```json
{
  "version": "0.1.0",
  "github": { "ok": true, "login": "octocat", "name": "The Octocat", "token_source": "gh" },
  "git": {
    "version": "git version 2.47.3",
    "identity": { "name": "The Octocat", "email": "octocat@example.com" },
    "identity_source": "git"
  },
  "optimizers": [
    { "id": "oxipng", "label": "PNG and APNG (oxipng)", "category": "image", "extensions": ["png"],
      "enabled": true, "available": true, "detail": "built-in oxipng 10" }
  ],
  "jobs": {
    "running": [{ "repo_id": 3, "full_name": "octo/site", "phase": "Optimizing 4/12: img/hero.png",
                  "started_at": "2026-11-09T10:00:00Z" }],
    "queued": [5, 8]
  },
  "totals": { "repos": 12, "repos_enabled": 11, "prs_open": 2, "awaiting_approval": 1,
              "errors": 0, "bytes_saved": 1843221, "prs_sent": 9 }
}
```

- `github.token_source`: `settings` | `env` | `gh`. When there is no usable token, `ok` is `false` and `error` explains why.
- `git.identity_source`: `settings` | `git` | `github` (see [author resolution](how-it-works.md#35-commit)).

## Settings

### `GET /api/settings`
`{ "settings": Settings, "github_token_set": bool }`. `github_token_set` only refers to the token saved through the API, not to env/`gh`.

### `PUT /api/settings`
Body: a full `Settings` object (missing fields take their defaults). Values are clamped to their valid ranges and the stored result is returned. See [configuration.md](configuration.md#global-settings) for every field.

```json
{
  "poll_interval_minutes": 20, "poll_jitter_minutes": 20, "max_concurrent_jobs": 2,
  "source_sync_interval_minutes": 360, "min_savings_percent": 1.0, "min_savings_bytes": 256,
  "max_file_size_mb": 30, "png_level": 3, "png_zopfli": false, "strip_metadata": false,
  "optimizers": { "gifsicle": false }, "ignore_globs": "docs/\n*.min.png",
  "branch_name": "git-opt/optimize-images", "commit_title": "[git-opt] Optimize images",
  "commit_author_name": "", "commit_author_email": "", "sign_commits": true
}
```

### `PUT /api/settings/github-token`
Body `{ "token": "ghp_…" }` saves a token after checking it against GitHub (`400` if GitHub rejects it, and the previous token is kept). `{ "token": null }` removes it. `204` on success. The token can never be read back.

## Repositories

A **Repo** as returned by the API:

| Field | Type | Notes |
|---|---|---|
| `id`, `owner`, `name` | | |
| `source_id` | int \| null | the source that imported it |
| `mode` | `direct` \| `fork` | |
| `clone_url`, `default_branch` | | refreshed from GitHub on every check |
| `fork_full_name`, `fork_clone_url` | string \| null | your fork, once created |
| `enabled`, `auto_pr` | bool | |
| `poll_interval_minutes` | int \| null | null = global |
| `ignore_globs` | string | newline separated |
| `status` | `pending` \| `ok` \| `awaiting_approval` \| `error` | result of the last check |
| `last_error` | string \| null | |
| `last_seen_sha`, `last_checked_at`, `next_check_at` | | |
| `pending_commit` | string \| null | commit waiting for approval |
| `pr_number`, `pr_url`, `pr_state` | | latest bot PR; `pr_state` is `open` \| `merged` \| `closed` |
| `bytes_saved` | int | bytes saved by merged bot PRs |
| `created_at` | | |
| `running`, `queued` | bool | live job state |
| `phase` | string \| null | what the running job is doing |

### `GET /api/repos`
All repositories, sorted by owner and name.

### `POST /api/repos`
```json
{ "repo": "octo/site", "mode": "auto", "auto_pr": true }
```
`repo` accepts `owner/name`, `https://github.com/owner/name(.git)` or `git@github.com:owner/name.git`. `mode`: `auto` (default), `direct` or `fork`. Returns `201` with the repo and queues a first check. `400` if GitHub does not know the repository, it is archived, or the mode is impossible; `409` if it is already watched.

### `GET /api/repos/{id}`
One repository.

### `PATCH /api/repos/{id}`
Any subset of:
```json
{ "enabled": true, "auto_pr": false, "mode": "fork", "poll_interval_minutes": 60, "ignore_globs": "docs/" }
```
`"poll_interval_minutes": null` goes back to the global interval.

### `DELETE /api/repos/{id}`
Removes the repository, its runs and its local clone (`204`). Nothing is changed on GitHub. `409` while a job is running for it.

### `POST /api/repos/{id}/check`
Optional body `{ "force": true }` to re-scan even if the default branch did not move. `202`.

### `POST /api/repos/check-all`
Queues a check of every enabled repository. `202`.

### `POST /api/repos/{id}/publish`
Pushes the commit that is awaiting approval and opens/updates the PR. `202`; `409` if nothing is awaiting approval.

### `POST /api/repos/{id}/reset-ignored`
Forgets files rejected in closed PRs. `204`.

### `GET /api/repos/{id}/runs?limit=50`
Runs of one repository, newest first.

## Runs

A **Run**: `id`, `repo_id`, `repo_full_name`, `trigger` (`schedule` | `manual` | `publish`), `status` (`no_changes` | `up_to_date` | `awaiting_approval` | `pr_opened` | `pr_updated` | `error`), `started_at`, `finished_at`, `head_sha`, `commit_sha`, `files_scanned`, `files_optimized`, `bytes_before`, `bytes_after`, `pr_number`, `pr_url`, `message`.

### `GET /api/runs?limit=100&repo_id=3`
Newest first; `limit` is capped at 500.

### `GET /api/runs/{id}`
```json
{ "run": Run, "files": [
  { "id": 1, "run_id": 7, "path": "img/hero.png", "optimizer": "oxipng",
    "original_blob": "5f1c…", "bytes_before": 360669, "bytes_after": 1335 } ] }
```

## Sources

A **Source**: `id`, `kind` (`user` | `org` | `me`), `name` (empty for `me`), `include_forks`, `include_archived`, `auto_sync`, `repo_count`, `last_synced_at`, `last_error`, `created_at`.

### `GET /api/sources`

### `POST /api/sources`
```json
{ "kind": "org", "name": "my-org", "include_forks": false, "include_archived": false, "auto_sync": true }
```
Returns `201` and starts a sync. `409` if it already exists.

### `PATCH /api/sources/{id}`
Any of `include_forks`, `include_archived`, `auto_sync`.

### `DELETE /api/sources/{id}?delete_repos=false`
Removes the source. With `delete_repos=true` its repositories (and clones) are removed too; otherwise they stay in the watch list.

### `POST /api/sources/{id}/sync`
Re-lists the source now. `202`.
