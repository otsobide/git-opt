# Development

Everything needed to work on git-opt: setup, the dev loop, tests, the fake GitHub, and how to extend it. The code map is in [architecture.md](architecture.md); conventions for agents are in [`AGENTS.md`](../AGENTS.md).

## Prerequisites

| Tool | Why |
|---|---|
| Rust ≥ 1.88 (edition 2024) | every crate |
| Node ≥ 22 and npm | frontend; the screenshot script uses Node's built-in `WebSocket` |
| git | runtime dependency and tests |
| a C compiler (gcc/clang) | builds the bundled SQLite, libdeflate, mozjpeg and libwebp (no nasm or libclang needed) |
| `gifsicle` | GIF optimizer, the only external tool; its test is skipped when missing |
| python3, curl, ImageMagick (`magick`), librsvg (`rsvg-convert`) | `scripts/e2e.sh` and `scripts/demo.sh` build fixtures, render SVGs and compare pixels |
| Chromium or Chrome | `scripts/screenshots.mjs` |
| Docker | building the image |

The C parts (bundled SQLite, libdeflate for oxipng, mozjpeg, libwebp) are compiled by `cargo` with the system C compiler. Dependencies are built with `opt-level = 3` even in dev builds (see the root `Cargo.toml`), otherwise image tests are very slow.

## Dev loop

```bash
cargo run                         # API on 127.0.0.1:8484 (uses ~/.local/share/git-opt)
cd frontend && npm ci && npm run dev   # UI on :5173 with hot reload, /api proxied to :8484
```

Use a throwaway data directory so you don't touch your real instance: `GIT_OPT_DATA_DIR=/tmp/git-opt-dev GIT_OPT_BIND=127.0.0.1:8484 cargo run`. Add `RUST_LOG=git_opt=debug` to see per-file decisions and failed git commands.

To work on the UI without a GitHub account (or without touching real repositories), run the demo below instead of `cargo run`; Vite proxies to it the same way.

## Tests

| Command | What it covers | Time |
|---|---|---|
| `cargo test --workspace` | every crate: domain value objects and policies (pure); every format strategy (it shrinks, the result is proven equivalent, `verify` rejects a minimal change, already optimal files are left alone, damaged/lossy/unusual inputs are refused: PNG, Zopfli, APNG frames, JPEG pixels and metadata, GIF frames, WebP pixels and chunks, ICO, ICNS, SVG from Inkscape/Illustrator, embedded images, SVGZ); the SQLite, git and GitHub adapters; use cases end to end (see below); the job queue | seconds |
| `cargo clippy --workspace --all-targets` | lints; keep it at zero warnings | seconds |
| `cargo check -p git-opt-codecs --no-default-features --features <list>` | each codec feature must build on its own | seconds |
| `cd frontend && npm run build` | `vue-tsc` type-check + production build | seconds |
| `scripts/e2e.sh` | the real binary against a fake GitHub and local bare repositories: add repo → PR with PNG, APNG, JPEG, WebP, ICO, ICNS, SVG, SVG with an embedded PNG and SVGZ fixtures → pixels identical (ImageMagick `compare -metric AE`; SVG/SVGZ rendered by librsvg, independently of resvg; the ICNS PNG element extracted and compared, other elements byte for byte) → merge → nothing left → third-party repo in fork mode with manual approval → publish | ~30 s |

Use cases (`crates/application/src/tests.rs`) run against real SQLite (in memory) and real git (local repositories), with an in-memory `CodeForge`, a fixed `Clock` and a fake `FormatStrategy`: they cover the full check → PR → merge cycle, quiet polls, fork + manual approval and rejected PRs without any network, in a fraction of a second. Add a test there for any new behavior of a use case.

The e2e script builds the debug binary, picks ports `18484`/`18999` (override with `PORT`/`API_PORT`) and deletes everything it created on exit.

## The fake GitHub

`scripts/mock-github.py <port> <config.json>` implements the endpoints git-opt calls (`/user`, `/user/repos`, `/users/{u}/repos`, `/orgs/{o}/repos`, `/repos/{o}/{r}`, forks and pulls) on top of local bare repositories:

```json
{ "token": "fake-token", "user": { "id": 42, "login": "jdev", "name": "Jane Developer" },
  "repos": {
    "octo/demo":        { "push": true,  "clone": "/abs/demo.git" },
    "other/thirdparty": { "push": false, "clone": "/abs/third.git", "fork_clone": "/abs/fork.git" },
    "octo/gone":        { "push": true,  "clone": "/abs/gone.git", "deleted": true } } }
```

Point git-opt at it with `GIT_OPT_GITHUB_API=http://127.0.0.1:<port>` and `GITHUB_TOKEN=fake-token`. Clone URLs are `file://`, so no credentials are injected. Helpers without auth: `POST /_mock/merge/<n>`, `POST /_mock/close/<n>`, `GET /_mock/pulls`.

## Demo and screenshots

```bash
scripts/demo.sh                              # http://127.0.0.1:18484, Ctrl+C to stop
scripts/demo.sh --screenshots docs/screenshots
```

The demo creates seven sample repositories and drives them into every state: an open PR that gets updated, a merged PR with credited savings, a repository awaiting approval, a third-party contribution through a fork, a repository deleted on GitHub (error), a paused one and one with a custom interval. It sets a fake commit identity, so no personal data ends up in screenshots.

`scripts/screenshots.mjs` drives Chromium through the DevTools protocol with no npm dependencies:

```bash
SCHEME=dark WIDTH=1440 HEIGHT=900 SCALE=1 \
  node scripts/screenshots.mjs http://127.0.0.1:18484 out/ dashboard=/ settings=/settings
```

`CHROME` selects the browser binary; `SETTLE_MS` is how long to wait for data after each page load. Regenerate the README screenshots after visible UI changes.

## Adding a new optimizer

1. New format? Add it to `FileFormat` in `crates/domain/src/values/format.rs` with its extensions (and its media type if documents can embed it).
2. Create `crates/codecs/src/<name>.rs` implementing `FormatStrategy`, behind a new Cargo feature whose libraries are optional dependencies:
   - `id`: stable, used in settings, cache keys and reports (never rename it);
   - `formats`: the `FileFormat`s it handles;
   - `explanation`: one Markdown line for the pull request ("what changes, what is kept");
   - `profile(options)`: changes whenever the output for the same input could change (tool, version family, every relevant option). It is part of the cache key;
   - `probe`: `Ok(version)` if it can run here, `Err(reason)` otherwise (shown in the UI);
   - `optimize`: return the candidate bytes; thresholds and verification are applied by `ports::attempt`;
   - `verify`: return `Err` unless the candidate is provably equivalent. For images, reuse `verify::same_pixels`.
3. Register it in `default_strategies()` in `crates/codecs/src/lib.rs` (and in `raster_strategies()` if SVG may embed it). If it needs another strategy, take it as a constructor argument.
4. External tool? Use `external::run_on_bytes(program, args, input, ext)` (with `{in}`/`{out}` placeholders) and `external::probe_version`, and install the tool in the runtime stage of the `Dockerfile`.
5. Tests: it shrinks a sample, `verify` rejects a minimally changed file, an already optimal file is left alone, unsupported input is refused; skip gracefully when an external tool is missing. Check `cargo check -p git-opt-codecs --no-default-features --features <name>`.
6. Describe it in the README format table and in [how-it-works.md](how-it-works.md#34-optimize-and-verify).

The use cases, cache, API and UI pick it up automatically (it gets a toggle under Settings → Formats).

## Replacing an adapter

Every external dependency sits behind a port (see [architecture.md](architecture.md#swapping-a-dependency)). Write a crate implementing it, build it in `crates/server/src/main.rs` instead of the current adapter, and run the workspace tests plus `scripts/e2e.sh`.

## Changing the database

Add `crates/adapters/sqlite/migrations/000N_<description>.sql`. Migrations are embedded at compile time and run on startup; never edit one that has been released. All SQL lives in the SQLite adapter; rows are mapped to domain entities in `rows.rs`. If a port needs a new query, add the method to the port trait in the domain first.

## Changing the API

Routes and handlers live in `crates/adapters/http/src/lib.rs`, the JSON shapes in `dto.rs`. Handlers only call `Services` methods (the use cases). The frontend mirrors the DTOs in `frontend/src/types.ts` and calls them through `frontend/src/api.ts`; update both sides together and document the change in [api.md](api.md).

## Frontend notes

- Vue 3 `<script setup>` + TypeScript, vue-router in history mode, no UI framework.
- Colors are tokens in `src/styles/tokens.css`, each defined once as `light-dark(light, dark)`. The theme follows `color-scheme`, which `useTheme()` forces through `data-theme` on `<html>` (an inline script in `index.html` applies it before first paint). Never hard-code colors in components.
- Data is refreshed with `usePolling` while the tab is visible; actions go through `useAction`, which handles busy state and toasts.
- Check narrow screens (≈390 px) after layout changes: nothing should scroll horizontally except tables and tab strips.

## Docker image

```bash
docker build -t git-opt .
docker run --rm -p 127.0.0.1:8484:8484 -e GITHUB_TOKEN git-opt
```

Stages: UI build (`node:24-slim`) → release binary (`rust:1.98-slim-trixie`, with cache mounts) → runtime (`debian:trixie-slim` + git, gifsicle, tini, user `gitopt`). To run the tests with gifsicle available:

```bash
docker run --rm -v "$PWD":/src:ro -w /tmp rust:1.98-slim-trixie sh -c \
  "apt-get update -qq && apt-get install -y -qq gifsicle >/dev/null \
   && cp -r /src/Cargo.toml /src/Cargo.lock /src/crates . && cargo test --workspace"
```

## Commits

Small, atomic commits (one logical change, each one builds and passes tests), short imperative one-line English messages, no AI attribution trailers. Everything in the repository is written in English.
