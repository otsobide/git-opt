# AGENTS.md

Guide for agents (and people) working in this repository. Overview: `README.md`. Full documentation: `docs/` (start at `docs/README.md`).

## What this is

`git-opt` is a self-hosted ImgBot-like bot: it watches GitHub repositories, optimizes images **losslessly** and opens PRs with the token and git identity of the developer who owns the machine. Rust workspace in **hexagonal architecture** (`crates/`), Vue UI (`frontend/`), a single Docker image. Read `docs/architecture.md` before changing the structure.

## Commands

```bash
cargo build                         # from the root (workspace)
cargo test --workspace              # every crate's tests
cargo fmt --all                     # format (rustfmt.toml, 120 columns)
cargo clippy --workspace --all-targets   # lint (zero warnings)
cargo check -p git-opt-codecs --no-default-features --features png   # feature subsets must build
scripts/e2e.sh                      # full flow against a fake GitHub, every format; needs magick, rsvg-convert
scripts/demo.sh                     # UI with sample repos on :18484, fake GitHub, no github.com
scripts/demo.sh --screenshots docs/screenshots   # regenerate the README/docs screenshots (dark)
cd frontend && npm ci && npm run build   # type-check (vue-tsc) + build into frontend/dist
cd frontend && npm run dev          # Vite :5173, proxies /api -> 127.0.0.1:8484
docker compose up -d --build        # single image (API + UI + git/gifsicle)
```

Before calling something done: `cargo fmt --all --check`, `cargo test --workspace` and `cargo clippy --workspace --all-targets` with no warnings; if you touched `application`, an adapter or the server, also run `scripts/e2e.sh`; if you touched `frontend/`, run `npm run build`. CI (`.github/workflows/ci.yml`, described in `docs/development.md`) runs all of this, plus the minimum Rust version and the Docker image, on every pull request.

Keep the documentation true: behavior → `docs/how-it-works.md`, settings/env vars → `docs/configuration.md`, endpoints → `docs/api.md`, UI labels or screens → `docs/usage.md` (and regenerate screenshots), code layout → `docs/architecture.md`, tooling → `docs/development.md`.

## Crate map

| Crate | Responsibility | May depend on |
|---|---|---|
| `crates/domain` | value objects (`values/`), entities (`model/`), policies (`ports::attempt`, `report`, `PollPolicy`, `IgnoreMatcher`), **ports** (`ports/`) | nothing of ours; no I/O, no frameworks |
| `crates/application` | use cases (`use_cases/`), `Services` + `Ports`, `StrategyCatalog`, job queue, scheduler, `DeliveryStrategy`, author chain | domain only |
| `crates/codecs` | one `FormatStrategy` per format, each behind a Cargo feature; shared pixel verification | domain + codec libraries |
| `crates/adapters/sqlite` | all persistence ports; **every SQL statement**; `migrations/` | domain + sqlx |
| `crates/adapters/git-cli` | `VersionControl`/`Workspace`: the **only** place that runs `git` | domain |
| `crates/adapters/github` | `CodeForge` + token chain (UI → env → `gh auth token`) | domain + reqwest |
| `crates/adapters/http` | JSON API + static UI; DTOs (`dto.rs`) are the contract with `frontend/src/types.ts` | application + axum |
| `crates/server` | composition root: env config, builds adapters, injects them, starts scheduler and HTTP | everything |

## Invariants that must not break

- **Dependency rule.** Dependencies point inwards: domain ← application ← adapters/server. Never add an infrastructure crate (sqlx, reqwest, axum, tokio::process, image codecs…) to `domain` or `application`; add a port instead and implement it in an adapter. Value objects are valid by construction: build them with their constructors, don't pass raw strings through the core.
- **Truly lossless.** Every optimizer implements `verify` and must prove equivalence (decoded pixels/frames; plus metadata segments for JPEG, chunks for WebP, XML tree and resvg rendering for SVG). If it cannot be verified, the file is not touched. Never enable lossy options (`--lossy`, `optimize_alpha`, `scale_16`, dropping JPEG markers, WebP without `exact`, SVG number rounding…), never convert between formats, never touch lossy WebP.
- **FFI safety.** mozjpeg and libwebp are called through `unsafe` code. Keep every C object owned by a guard that frees it on all paths, keep libjpeg structs boxed (libjpeg stores pointers to them), and turn C errors into `Err`, never `exit()`/abort. Any change there needs a test with damaged input.
- **The marker** `Git-Opt: lossless-optimization` goes in every bot commit and PR. "Already optimized" detection relies on the blobs those commits introduce (`Workspace::blobs_written_by_marked_commits`, `report::BOT_MARKER`); changing the text breaks recognition of existing history.
- **The developer's identity.** Commits use the system `git config` (or what is set in Settings, or the GitHub account); API calls and pushes use their token. There is no "bot" account.
- **Secrets.** The token never goes into arguments or URLs: it reaches git through `GIT_CONFIG_COUNT/KEY/VALUE` (`GitAuth`). Do not log it or return it from the API (`PUT /api/settings/github-token` is write-only).
- **The clone belongs to the bot.** It lives in `$GIT_OPT_DATA_DIR/repos/<owner>/<name>`; `checkout --force`/`clean` are fine. Commits use `--no-verify` and `core.hooksPath=/dev/null`.
- **No noise in the history**: scheduled checks that find nothing new do not create `runs` rows, and neither do repeated identical errors.

## Adding an optimizer / file type

1. If it is a new format, add it to `FileFormat` (domain) with its extensions (and media type if it can be embedded).
2. New module in `crates/codecs/src/` implementing `FormatStrategy` (stable `id`, `formats`, `explanation` for the PR body, `profile` covering everything that changes the output, `probe`, `optimize`, `verify`), behind a new Cargo feature with its library as an optional dependency.
3. Register it in `default_strategies()` (and in `raster_strategies()` if SVG may embed it). Composite strategies get collaborators by injection, never by naming a concrete type from another module.
4. External tool? Use `external::run_on_bytes` and add the tool to the `Dockerfile` (runtime stage).
5. Tests in `crates/codecs/src/tests.rs` (or the module): it shrinks a file, `verify` rejects a minimal change, an already optimal file is left alone, damaged/unsupported input is refused. Check the feature builds alone.
6. Add it to the README format table and `docs/how-it-works.md`.

The use cases, cache (`blob_cache`, keyed by `blob + profile`), UI and reports pick it up automatically.

## Replacing an adapter

Write a crate that implements the port (storage, `VersionControl`, `CodeForge`, `FormatStrategy`) and build it in `crates/server/src/main.rs` instead of the current one. Nothing else changes. Test it like the existing adapter tests, then run `scripts/e2e.sh`.

## Schema and migrations

SQLite through `sqlx` with `sqlx::migrate!`. To change the schema add `crates/adapters/sqlite/migrations/000N_description.sql`; **never edit a migration that has been published**. Dates are `DateTime<Utc>` (RFC 3339 text, lexicographically comparable).

## Frontend

Vue 3 + Vite + TypeScript + vue-router, hand-written CSS without UI frameworks. The API contract lives in the types in `frontend/src/types.ts`; if you change a DTO in `crates/adapters/http/src/dto.rs`, update both sides.

Colors only come from the tokens in `src/styles/tokens.css`, each written once as `light-dark(light, dark)`; the System/Light/Dark switcher (`useTheme`) sets `data-theme` on `<html>`. Never hard-code a color in a component.

## Language

Everything in the repository is in **English**: code, comments, logs, UI texts, API messages, docs, commit messages, and the commits/PRs the bot generates.

## Commits in this repo

Small, **atomic** commits: one logical change each, and every commit must build on its own. Short one-line imperative messages ("Add fork mode retry"). No long descriptions and **no `Co-Authored-By` trailers or any AI assistant attribution**.

## Testing without GitHub

Use cases are tested in `crates/application/src/tests.rs` with real SQLite (in memory) and git (local repositories) plus an in-memory forge, a fixed clock and a fake format strategy: add tests there for new behavior; they need no network and run in well under a second.

`scripts/mock-github.py` imitates the endpoints we use (repos, forks, pulls, `/user`) with local *bare* repositories as `clone_url` (`file://…`; without `https` no auth is injected). `scripts/e2e.sh` uses it to test: add repo → PR → identical pixels (ImageMagick) → merge → nothing pending → third-party repo in fork mode with manual approval. `scripts/demo.sh` reuses the same fake to show every repository state in the UI. Never test against real repositories without explicit permission: forks and PRs are visible to other people.
