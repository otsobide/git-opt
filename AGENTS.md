# AGENTS.md

Guide for agents (and people) working in this repository. Overview: `README.md`. Full documentation: `docs/` (start at `docs/README.md`).

## What this is

`git-opt` is a self-hosted ImgBot-like bot: it watches GitHub repositories, optimizes images **losslessly** and opens PRs with the token and git identity of the developer who owns the machine. Rust backend (`backend/`), Vue UI (`frontend/`), a single Docker image.

## Commands

```bash
cargo build                         # from the root (workspace)
cargo test                          # backend unit tests
cargo clippy --all-targets          # lint
scripts/e2e.sh                      # full flow against a fake GitHub; needs magick and jpegtran
scripts/demo.sh                     # UI with sample repos on :18484, fake GitHub, no github.com
scripts/demo.sh --screenshots docs/screenshots   # regenerate the README/docs screenshots (dark)
cd frontend && npm ci && npm run build   # type-check (vue-tsc) + build into frontend/dist
cd frontend && npm run dev          # Vite :5173, proxies /api -> 127.0.0.1:8484
docker compose up -d --build        # single image (API + UI + git/jpegtran/gifsicle)
```

Before calling something done: `cargo test` and `cargo clippy` with no warnings; if you touched `pipeline.rs`, `git.rs`, `github.rs` or `api.rs`, also run `scripts/e2e.sh`; if you touched `frontend/`, run `npm run build`.

Keep the documentation true: behavior → `docs/how-it-works.md`, settings/env vars → `docs/configuration.md`, endpoints → `docs/api.md`, UI labels or screens → `docs/usage.md` (and regenerate screenshots), code layout → `docs/architecture.md`, tooling → `docs/development.md`.

## Backend map

| File | Responsibility |
|---|---|
| `main.rs` | startup, axum server, static UI with SPA fallback, shutdown on SIGTERM |
| `config.rs` | environment variables (`GIT_OPT_*`); nothing editable at runtime |
| `settings.rs` | settings edited from the UI (JSON in the `settings` table); `sanitized()` clamps values |
| `db.rs` + `migrations/` | **every** SQL query. Do not write SQL outside `db.rs` |
| `api.rs` | `/api/*` routes; errors as `{ "error": "..." }` |
| `jobs.rs` | in-memory queue: dedupes per repo, never runs two jobs for the same repo at once, live progress |
| `scheduler.rs` | ticker (15 s) that queues due work, and a dispatcher whose concurrency is re-read on every loop |
| `pipeline.rs` | per-repo work: fetch → candidates → optimize → commit → push → PR; adding repos and syncing sources |
| `git.rs` | the **only** place that runs `git`. Holds the `BOT_MARKER` constant |
| `github.rs` | REST client and token resolution (UI → env → `gh auth token`) |
| `optimizers/` | `Optimizer` trait, registry, oxipng/jpegtran/gifsicle, pixel verification |
| `report.rs` | commit message and PR body (both carry `BOT_MARKER`) |

## Invariants that must not break

- **Truly lossless.** Every optimizer implements `verify` and must prove equivalence (today: decode both and compare pixels/frames). If it cannot be verified, the file is not touched. Never enable lossy options (`--lossy`, `optimize_alpha`, `scale_16`, `-copy none` for JPEG…).
- **The marker** `Git-Opt: lossless-optimization` goes in every bot commit and PR. "Already optimized" detection relies on the blobs those commits introduce (`Git::blobs_from_bot_commits`); changing the text breaks recognition of existing history.
- **The developer's identity.** Commits use the system `git config` (or what is set in Settings, or the GitHub account); API calls and pushes use their token. There is no "bot" account.
- **Secrets.** The token never goes into arguments or URLs: it reaches git through `GIT_CONFIG_COUNT/KEY/VALUE` (`GitAuth`). Do not log it or return it from the API (`PUT /api/settings/github-token` is write-only).
- **The clone belongs to the bot.** It lives in `$GIT_OPT_DATA_DIR/repos/<owner>/<name>`; `checkout --force`/`clean` are fine. Commits use `--no-verify` and `core.hooksPath=/dev/null`.
- **No noise in the history**: scheduled checks that find nothing new do not create `runs` rows, and neither do repeated identical errors.

## Adding an optimizer / file type

1. New module in `backend/src/optimizers/` implementing `Optimizer` (stable `id`, `extensions`, `profile` covering everything that changes the output, `probe`, `optimize`, `verify`).
2. Register it in `Registry::new`.
3. If it wraps an external tool, use `external::run_on_bytes` and add the tool to the `Dockerfile` (runtime stage).
4. Tests in `optimizers/mod.rs`: it shrinks a file, `verify` rejects a minimal change, and an already optimal file is left alone.
5. Add its line to the "How" block in `report::pr_body` and to the README table.

The pipeline, the cache (`blob_cache`, keyed by `blob + profile`), the UI and the reports need no changes.

## Schema and migrations

SQLite through `sqlx` with `sqlx::migrate!`. To change the schema add `backend/migrations/000N_description.sql`; **never edit a migration that has been published**. Dates are `DateTime<Utc>` (RFC 3339 text, lexicographically comparable).

## Frontend

Vue 3 + Vite + TypeScript + vue-router, hand-written CSS without UI frameworks. The API contract lives in the types in `frontend/src/types.ts`; if you change a model in `backend/src/models.rs` or a response in `api.rs`, update both sides.

Colors only come from the tokens in `src/styles/tokens.css`, each written once as `light-dark(light, dark)`; the System/Light/Dark switcher (`useTheme`) sets `data-theme` on `<html>`. Never hard-code a color in a component.

## Language

Everything in the repository is in **English**: code, comments, logs, UI texts, API messages, docs, commit messages, and the commits/PRs the bot generates.

## Commits in this repo

Small, **atomic** commits: one logical change each, and every commit must build on its own. Short one-line imperative messages ("Add fork mode retry"). No long descriptions and **no `Co-Authored-By` trailers or any AI assistant attribution**.

## Testing without GitHub

`scripts/mock-github.py` imitates the endpoints we use (repos, forks, pulls, `/user`) with local *bare* repositories as `clone_url` (`file://…`; without `https` no auth is injected). `scripts/e2e.sh` uses it to test: add repo → PR → identical pixels (ImageMagick) → merge → nothing pending → third-party repo in fork mode with manual approval. `scripts/demo.sh` reuses the same fake to show every repository state in the UI. Never test against real repositories without explicit permission: forks and PRs are visible to other people.
