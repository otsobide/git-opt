# Architecture

git-opt is one Rust binary (tokio + axum + sqlx/SQLite) that also serves the built Vue UI. This page describes how the code is organized; [how-it-works.md](how-it-works.md) describes the behavior.

## Repository layout

```
.
├── backend/                 Rust crate `git-opt` (the only workspace member)
│   ├── migrations/          SQLite schema (sqlx migrations, embedded in the binary)
│   └── src/
│       ├── main.rs          startup and HTTP server
│       ├── app.rs           shared state (`App`)
│       ├── config.rs        environment variables
│       ├── settings.rs      runtime settings (stored in the DB)
│       ├── models.rs        rows and API payloads
│       ├── db.rs            every SQL query
│       ├── api.rs           `/api/*` handlers and admin-token middleware
│       ├── jobs.rs          in-memory job queue and live progress
│       ├── scheduler.rs     ticker + dispatcher
│       ├── pipeline.rs      per-repository work
│       ├── git.rs           system `git` wrapper
│       ├── github.rs        GitHub REST client and token resolution
│       ├── report.rs        commit message and PR body
│       └── optimizers/      `Optimizer` trait, registry, implementations, verification
├── frontend/                Vue 3 + Vite + TypeScript UI (built into frontend/dist)
├── scripts/                 e2e test, fake GitHub API, screenshot generator
├── contrib/                 systemd user unit
├── docs/                    this documentation
├── Dockerfile               single image: UI build → Rust build → Debian runtime
└── docker-compose.yml
```

## Runtime components

```
main.rs
 ├── Config::from_env()
 ├── Db::open()                → runs migrations
 ├── GitHub::new()             → HTTP client, token cache
 ├── optimizers::Registry::new → probes external tools once
 ├── App { config, db, github, optimizers, jobs }   (Arc, shared by everything)
 ├── scheduler::spawn(app)
 │     ├── ticker      every 15 s: due repos → Job::Check, due sources → Job::SyncSource
 │     └── dispatcher  pops jobs while running < max_concurrent_jobs, one tokio task each
 └── axum::serve(api::router(app) + ServeDir(static UI, SPA fallback))
```

### `jobs.rs`: the queue

A `Mutex`-protected structure with:

- `queue: VecDeque<Job>`: pending work. Scheduled checks and source syncs go to the back; manual checks and publishes go to the front.
- `queued` and `running`: sets of `JobKey` (`Repo(id)` or `Source(id)`). A key can be queued once and running once; `next()` skips jobs whose key is already running, so two jobs never touch the same clone concurrently. Re-requesting a queued repository manually upgrades the queued job (for example to a forced check) instead of adding another.
- `progress: HashMap<repo_id, RunningJob>`: the current phase string shown in the UI.
- a `tokio::sync::Notify` that wakes the dispatcher when something is queued or finishes.

Each job runs inside its own spawned task, so a panic is caught and the key is always released.

### `pipeline.rs`: the work

| Function | Job |
|---|---|
| `add_repo` | Resolve `owner/name`, decide the mode from permissions, insert. |
| `sync_source` | List a user/org/"me" and insert new repositories. |
| `check` → `run_check` | The full check: metadata, PR state, fetch, candidates, optimize, commit, publish. Records the run and schedules the next check. |
| `publish` → `run_publish` | Push a commit awaiting approval and open the PR. |
| `resolve_identity` | Commit author: settings → git config → GitHub account. |
| `next_check_at` | `now + interval + random(0..=jitter)`. |

CPU-heavy work (oxipng, image decoding for verification) runs in `spawn_blocking`; external tools run as async child processes with timeouts.

### `git.rs`

Every `git` invocation goes through `Git::exec`, which sets `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C`, a timeout (10 min local, 30 min network), `kill_on_drop`, and, when a token is available, the authentication header through `GIT_CONFIG_COUNT/KEY/VALUE`. Errors include git's stderr.

### `github.rs`

A small typed client over `reqwest` (rustls): `user`, `repo`, `list_repos` (follows `Link` pagination), `fork` (waits until ready), `pull`, `find_open_pull`, `create_pull`, `update_pull`. Errors carry GitHub's message plus a hint for 401/403/404. The token and the `/user` response are cached for 5 minutes; saving a token in the UI invalidates both.

### `optimizers/`

```rust
#[async_trait]
pub trait Optimizer: Send + Sync {
    fn id(&self) -> &'static str;              // "oxipng"
    fn label(&self) -> &'static str;           // "PNG (oxipng)"
    fn category(&self) -> &'static str;        // "image"
    fn extensions(&self) -> &'static [&'static str];
    fn enabled_by_default(&self) -> bool { true }
    fn profile(&self, settings: &Settings) -> String;   // cache key part
    async fn probe(&self) -> Result<String, String>;    // available here?
    async fn optimize(&self, input: Arc<Vec<u8>>, settings: &Settings) -> Result<Vec<u8>>;
    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()>;
}
```

`optimizers::process` is the only entry point used by the pipeline: optimize → size thresholds → verify → `Outcome::{Optimized, Optimal, Failed}`. `verify.rs` holds the pixel comparison (`image` crate) for still images and GIF animations. External tools are driven through temp files by `external.rs`.

## Data model

SQLite in WAL mode with foreign keys on. All queries live in `db.rs`.

```
sources 1───* repos 1───* runs 1───* run_files
                   │
                   └───* repo_ignored_blobs

settings            (key → JSON value; "settings" and "github_token")
blob_cache          (blob_sha, profile) → result      global, content addressed
```

| Table | Purpose |
|---|---|
| `settings` | `settings` holds the JSON-serialized `Settings`; `github_token` holds the token saved from the UI. |
| `sources` | Users/orgs/"me" to import from, with their filters and last sync result. |
| `repos` | The watch list: identity, mode, fork, per-repo settings, scheduling (`last_seen_sha`, `next_check_at`), last result, pending commit, latest PR and merged savings. |
| `runs` | One row per meaningful check or publish. |
| `run_files` | Per-file results of a run, including the **original blob id** (used to reject a PR's files when it is closed). |
| `blob_cache` | What an optimizer profile did with a blob: `optimal`, `failed` (with the reason) or `optimized` (with the output blob id, reused while a PR is open). Shared by all repositories. |
| `repo_ignored_blobs` | Blobs rejected by maintainers of a repository. |

Repository status (`repos.status`): `pending` (never checked) → `ok` | `awaiting_approval` | `error`. Live states (`queued`, `running`) are not stored; the API adds them from the job queue.

## Frontend

```
frontend/src/
├── main.ts, App.vue        app shell, navigation, theme, login gate
├── router.ts               routes (history mode; the backend falls back to index.html)
├── api.ts                  typed fetch client (adds the admin token, maps errors)
├── types.ts                API types, mirror of backend/src/models.rs
├── format.ts               bytes, percentages, dates, relative times
├── composables/            polling, status, toasts, async actions, clock
├── components/             tags, size bars, menus, PR links, confirm buttons, …
├── views/                  Dashboard, Repositories, Repository, Sources, Activity, Run, Settings
└── styles/                 design tokens (light/dark), base styles, tags
```

Views poll the API every few seconds while visible, so running jobs and their phases update live without websockets.

## Docker image

Three stages:

1. `node:24-slim` builds the UI (`npm ci && npm run build`).
2. `rust:1.98-slim-trixie` builds the release binary (with cargo registry/target cache mounts).
3. `debian:trixie-slim` runtime with `git`, `libjpeg-turbo-progs` (jpegtran), `gifsicle`, `ca-certificates` and `tini`; runs as the unprivileged user `gitopt` (uid 1000) with data in `/data`.
