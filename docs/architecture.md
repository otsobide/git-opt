# Architecture

git-opt follows a **hexagonal architecture** (ports and adapters). The core (what a repository, a run or an optimization is, and what the bot does with them) knows nothing about SQLite, git, GitHub, HTTP or image libraries. Those live in adapters that implement the core's **ports** (Rust traits), and a small composition root plugs them together. Replacing a dependency means writing another adapter; nothing in the core changes.

[how-it-works.md](how-it-works.md) describes the behavior; this page describes how the code is organized.

## Layers and crates

```
                         ┌──────────────────────────── driving side ───────────────────────────┐
                         │  adapters/http  (axum JSON API + static UI, DTOs)                    │
                         └───────────────────────────────┬─────────────────────────────────────┘
                                                         │ calls use cases
┌────────────────────────────────────────────────────────▼────────────────────────────────────────────────────────┐
│ application   use cases (check, publish, add/remove repos, sources, settings, status), job queue, scheduler,    │
│               strategies: delivery (direct / via fork), author chain                                            │
│ ┌─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐ │
│ │ domain      value objects · entities · policies (attempt, report, poll, ignore, forks) · PORTS (traits)     │ │
│ └─────────────────────────────────────────────────────────────────────────────────────────────────────────────┘ │
└───────┬───────────────────┬────────────────────┬──────────────────────┬─────────────────────────────────────────┘
        │ RepositoryStore…  │ VersionControl     │ CodeForge            │ FormatStrategy
┌───────▼────────┐ ┌────────▼────────┐ ┌─────────▼─────────┐ ┌──────────▼─────────────────────────────────────────────┐
│adapters/sqlite │ │adapters/git-cli │ │adapters/github    │ │codecs: oxipng · mozjpeg · gifsicle · libwebp · ICO ·    │
│ sqlx, SQLite   │ │ system git      │ │ REST + token chain│ │        ICNS · SVG (resvg) · SVGZ   (one feature each)   │
└────────────────┘ └─────────────────┘ └───────────────────┘ └────────────────────────────────────────────────────────┘
                                         driven side
server: composition root (reads env config, builds every adapter, injects them into the application, starts everything)
```

| Crate | Package | Depends on | Role |
|---|---|---|---|
| `crates/domain` | `git-opt-domain` | nothing but small pure libraries (`chrono`, `globset`, `thiserror`, `async-trait`) | value objects, entities, policies, **ports** |
| `crates/application` | `git-opt-application` | domain only | use cases, job queue, scheduler, application strategies |
| `crates/codecs` | `git-opt-codecs` | domain + image libraries | one `FormatStrategy` per format, each behind a Cargo feature |
| `crates/adapters/sqlite` | `git-opt-sqlite` | domain + sqlx | every persistence port; owns the migrations |
| `crates/adapters/git-cli` | `git-opt-git-cli` | domain | `VersionControl` with the system `git` |
| `crates/adapters/github` | `git-opt-github` | domain + reqwest | `CodeForge` and the token source chain |
| `crates/adapters/http` | `git-opt-http` | application + axum | JSON API and static UI (driving adapter) |
| `crates/server` | `git-opt` (binary `git-opt`) | everything | composition root |

**The dependency rule:** arrows only point inwards. The domain depends on no other crate of the project; the application depends only on the domain (its tests use real adapters as dev-dependencies); adapters depend on the domain (and the HTTP adapter on the application); only `server` sees everything. Cargo enforces this: a use case cannot call `sqlx` or `git` because the application crate does not depend on them.

## Domain (`crates/domain`)

### Value objects (`values/`)

Small immutable types that are **valid by construction**: if you hold one, it is valid. They replace strings and integers everywhere in the core.

| Value object | Guarantees |
|---|---|
| `RepoSlug` (`OwnerName` + `RepoName`) | parses `owner/name`, HTTPS and SSH URLs; only valid name characters |
| `CommitSha`, `BlobId` | 40 (SHA-1) or 64 (SHA-256) lowercase hex characters |
| `BranchName` | `git check-ref-format` rules |
| `RepoPath` | relative, no `..`/empty segments, not inside `.git`; knows its extension |
| `CloneUrl` | known scheme, no whitespace; knows its HTTPS credential scope |
| `ByteSize`, `Savings`, `SavingsThreshold` | sizes with binary-unit display; percent saved; "is this gain worth a change?" |
| `FileFormat` | the formats git-opt handles, from extension or media type |
| `PollPolicy` | interval (1 min–7 days) + jitter (0–24 h); computes the next check |
| `IgnoreRules`, `IgnoreMatcher` | compiled `.gitignore`-like patterns |
| `Identity` | a commit author (name + email) |
| `AccessToken` | a secret whose `Debug`/`Display` print `***` |
| `OptimizerId`, `Profile` | strategy slug; cache key of a strategy's options |
| `RepoId`, `SourceId`, `RunId`, `PrNumber` | typed ids |

### Entities and enums (`model/`)

`Repository`, `Source`, `Run` (+ `RunOutcome`, `FileChange`), `Settings` (+ `CodecOptions`, the subset strategies see), `BlobVerdict`. States are enums with stable names (`RepoStatus`, `RunStatus`, `PrState`, `RunTrigger`, `RepoMode`, `SourceKind`), never free text.

### Policies

- `ports::attempt`: the only way a file is optimized. It is a template method around a strategy: optimize → keep only if the gain passes the `SavingsThreshold` → require the strategy's proof of equivalence → `Outcome::{Optimized, Optimal, Failed}`.
- `report`: the commit message and pull request body, and `BOT_MARKER`, the trailer that lets the bot recognize its own work in any history.
- `fork_cleanup`: when the developer's fork may be deleted: only their own fork of that repository, with no open pull request and no branch holding unmerged commits.
- `PollPolicy::next_check`, `IgnoreMatcher`, `Settings::sanitized`, `BlobVerdict::skips_file`.

### Ports (`ports/`)

| Port | Implemented by | Purpose |
|---|---|---|
| `FormatStrategy` | `codecs` | optimize one family of formats and prove equivalence |
| `VersionControl` + `Workspace` | `adapters/git-cli` | the bot's working copies: clone, fetch, tree, marked blobs, commit, push |
| `CodeForge` | `adapters/github` | account, repositories, forks (create, inspect branches and compare, delete), pull requests, credentials |
| `RepositoryStore`, `SourceStore`, `RunStore`, `SettingsStore`, `SecretStore`, `VerdictCache`, `RejectionStore` | `adapters/sqlite` | persistence, shaped by the use cases (targeted updates, so a background check never overwrites what the user changed meanwhile) |
| `Clock`, `Random` | `server` (`system.rs`) | time and jitter, so scheduling is testable |

## Application (`crates/application`)

`Services` holds the injected `Ports` and the `StrategyCatalog`; each use case is a method on it, one module per area:

| Module | Use cases |
|---|---|
| `use_cases/check.rs` | `check_repository`: refresh PR state, fetch, find candidates, optimize, commit, deliver |
| `use_cases/forks.rs` | `tidy_fork`: after a check, delete the fork once every pull request from it is merged |
| `use_cases/publish.rs` | `publish_pending`: deliver a commit that was waiting for approval |
| `use_cases/repositories.rs` | add (with mode selection), update, remove, request check/publish, forget rejections |
| `use_cases/sources.rs` | add/update/remove sources, `sync_source` |
| `use_cases/settings.rs` | update settings (rescheduling repositories), save/validate the token |
| `use_cases/runs.rs`, `status.rs` | queries for the activity log and the dashboard |

Supporting pieces:

- `catalog.rs`: the injected format strategies with their availability (probed at start and after settings change); picks the active ones and provides each strategy's explanation for the PR body.
- `jobs.rs`: the in-memory queue. A repository or source is queued at most once and never processed by two jobs at once; manual requests jump the queue; it also holds each running job's phase for the UI.
- `scheduler.rs`: a 15-second ticker that queues due checks and source syncs, and a dispatcher that runs up to `max_concurrent_jobs` jobs (re-read on every loop) in their own tasks so a panic cannot leave a repository marked busy.
- `error.rs`: `AppError::{NotFound, Conflict, Invalid, Internal}`, which the HTTP adapter maps to 404/409/400/500.

### Strategies

| Strategy | Where | Chosen by |
|---|---|---|
| `FormatStrategy` (oxipng, mozjpeg, gifsicle, libwebp, ICO, ICNS, SVG, SVGZ) | `codecs`, through the catalog | the file's `FileFormat` |
| `DeliveryStrategy` (`Direct`, `ViaFork`) | `application/delivery.rs` | the repository's `RepoMode` |
| Author sources (settings → VCS config → forge account) | `application/authorship.rs` | first that answers |
| Token sources (saved → environment → `gh` CLI) | `adapters/github/token.rs` | first that answers |

Composite strategies receive their collaborators by injection: the SVG strategy gets the raster strategies (to optimize embedded images), SVGZ gets the SVG strategy. Nothing in the codecs crate names a concrete library from another module.

## Adapters

### `codecs`

Each strategy is a module behind a Cargo feature (`png`, `jpeg`, `gif`, `webp`, `ico`, `icns`, `svg`, `svgz`; all on by default). `default_strategies()` wires the compiled-in ones. Building without a feature removes that strategy and its library:

```bash
cargo build -p git-opt-codecs --no-default-features --features png,svg
```

FFI strategies (mozjpeg, libwebp) own every C object through guards, keep libjpeg structures boxed, and turn C errors (and libjpeg warnings) into `Err`. `verify.rs` holds the shared pixel comparisons.

### `adapters/sqlite`

One `SqliteStore` implements every persistence port over one pool (WAL, foreign keys). All SQL of the project lives here, next to `migrations/`. Rows are converted into domain entities at the boundary; settings are stored as one JSON document whose shape is stable across versions. `SqliteStore::in_memory()` is used by tests.

### `adapters/git-cli`

Runs the system `git` with prompts disabled, a C locale and timeouts. The token is sent as an `http.<host>.extraHeader` through `GIT_CONFIG_*` variables, scoped to the host of the URL each command uses. Working copies live in `$GIT_OPT_DATA_DIR/repos/<owner>/<name>`.

### `adapters/github`

A typed client over reqwest (rustls), with `Link` pagination, fork readiness polling, error messages with hints, and a 5-minute cache for the token and the account. The token comes from a chain of `TokenSource`s.

### `adapters/http`

Axum routes that translate HTTP into use cases. DTOs in `dto.rs` define the JSON contract (mirrored by `frontend/src/types.ts`), so domain types can evolve without breaking the UI. It also serves the built UI with an SPA fallback and enforces the optional admin token.

### `server`

Reads the environment (`config.rs`), builds `SqliteStore`, `GitHub`, `GitCli`, the codec catalog, `SystemClock` and `FastRandom`, injects them into `Services`, starts the scheduler and serves the HTTP router with graceful shutdown on SIGTERM.

## Swapping a dependency

| To replace… | Write… | Then… |
|---|---|---|
| an image library (e.g. another PNG optimizer) | a `FormatStrategy` for the same `FileFormat` (in `codecs` behind a new feature, or in your own crate) | register it in `default_strategies()` or in `server` |
| SQLite (e.g. PostgreSQL) | a crate implementing the seven storage ports | build it in `server/main.rs` instead of `SqliteStore` |
| GitHub (e.g. GitLab, Gitea) | a crate implementing `CodeForge` | build it in `server` |
| the system git (e.g. gitoxide) | a crate implementing `VersionControl`/`Workspace` | build it in `server` |
| the HTTP framework | another driving adapter calling `Services` | serve it from `server` |

Use cases, domain rules and the other adapters stay untouched.

## Testing by layer

| Layer | How |
|---|---|
| domain | pure unit tests of every value object and policy (no I/O) |
| codecs | each strategy shrinks a sample, its result passes its proof, its verifier rejects a one-pixel/one-byte change, and damaged/lossy/unusual inputs are refused; feature subsets must compile |
| adapters | SQLite against an in-memory database; git against local repositories; GitHub payload mapping and the token chain |
| application | use cases end to end with real SQLite and git, an in-memory forge, a fixed clock and a fake format strategy: PR opened as the developer, quiet polls, merges credited, fork + approval, rejected PRs, merged forks deleted only when safe |
| whole system | `scripts/e2e.sh`: the real binary against a fake GitHub API, ten formats, pixels compared with ImageMagick/librsvg |

## Data model

SQLite schema (`crates/adapters/sqlite/migrations`):

```
sources 1───* repos 1───* runs 1───* run_files
                   │
                   └───* repo_ignored_blobs

settings            (key → JSON: "settings" and "github_token")
blob_cache          (blob_sha, profile) → verdict      global, content addressed
```

| Table | Purpose |
|---|---|
| `settings` | the JSON-serialized settings; the token saved from the UI |
| `sources` | users/orgs/"me" to import from, with filters and last sync result |
| `repos` | the watch list: identity, mode, fork, per-repo settings, scheduling, last result, pending commit, latest PR, merged savings |
| `runs` | one row per meaningful check or publish |
| `run_files` | per-file results, including the original blob id (used to reject a PR's files when it is closed) |
| `blob_cache` | `BlobVerdict` per blob and strategy profile, shared by all repositories |
| `repo_ignored_blobs` | blobs rejected by a repository's maintainers |

## Frontend

```
frontend/src/
├── main.ts, App.vue        app shell, navigation, theme switcher, login gate
├── router.ts               routes (history mode; the HTTP adapter falls back to index.html)
├── api.ts                  typed fetch client (adds the admin token, maps errors)
├── types.ts                API types, mirror of crates/adapters/http/src/dto.rs
├── format.ts               bytes, percentages, dates, relative times
├── composables/            polling, status, toasts, async actions, clock, theme
├── components/             tags, size bars, menus, PR links, confirm buttons, …
├── views/                  Dashboard, Repositories, Repository, Sources, Activity, Run, Settings
└── styles/                 design tokens (light-dark), base styles, tags
```

Views poll the API every few seconds while visible, so running jobs and their phases update live without websockets.

## Docker image

Three stages: the UI is built with `node:24-slim`; the `git-opt` binary with `rust:1.98-slim-trixie` (cache mounts; mozjpeg and libwebp compiled from source by their `-sys` crates); the runtime is `debian:trixie-slim` with `git`, `gifsicle`, `ca-certificates` and `tini`, running as the unprivileged user `gitopt` (uid 1000) with data in `/data`.
