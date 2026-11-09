# git-opt

A **self-hosted** bot in the spirit of ImgBot: it watches your GitHub repositories, finds images that can be compressed **losslessly** and opens a pull request **in your name** with the result. Rust backend, Vue admin UI, all shipped as **a single Docker container**.

- **Proven lossless**: every optimized image is decoded and compared pixel by pixel with the original; if they are not identical, the file is left alone.
- **Runs in the background**: each repository is checked every **20 min + a random delay of up to 20 min** (configurable globally and per repo).
- **Knows what it already optimized by reading the commit history**, whether the PR was merged with *merge*, *rebase* or *squash*, and even if you wipe its database.
- **PRs and commits in the name of the developer who owns the machine**: it uses your GitHub token and your `git config`.
- **Third-party contributions**: for repositories you cannot push to, it forks them into your account and opens the PR against the original.
- Imports **every repository of a user, an organization, or everything your token can access**, and keeps the list in sync.
- Uses the **system `git`** to clone, commit and push.
- Built to grow: optimizers are plugins (`Optimizer`). Today they handle images; tomorrow they can handle other file types.

## How it works

```
every 20 min + rand(0..20 min), per repo
  │
  ├─ GitHub API: default branch, state of the previous PR (merged? closed?)
  ├─ git clone (first time) / git fetch origin <default branch>
  ├─ HEAD unchanged since last time? → nothing to do
  ├─ git ls-tree: files that have an optimizer (png, jpg/jpeg, gif)
  │    skips: ignored patterns, LFS files, files that are too large,
  │           blobs written by the bot's commits (history),
  │           blobs rejected in PRs closed without merging,
  │           blobs already analyzed that cannot be improved (cache)
  ├─ optimize → verify pixel by pixel → write the file
  ├─ git commit (author: you) with the marker  Git-Opt: lossless-optimization
  ├─ git push --force to branch git-opt/optimize-images (in the repo or your fork)
  └─ open the PR, or update the one that is already open
```

### How it recognizes its own work

Every bot commit ends with the trailer `Git-Opt: lossless-optimization` (the PR body carries it too, so it survives a squash that uses the PR description). On each check the bot runs

```
git log --grep "Git-Opt: lossless-optimization" --raw <default branch>
```

and collects the **blob hashes** those commits wrote. Because it compares content rather than commit ids, it works the same with *merge*, *rebase* or *squash*. If an image is modified later, its blob changes and it is evaluated again.

It also keeps a content-addressed cache in SQLite (`blob → result`) so images that are already optimal are not processed again, and it remembers the files of PRs that maintainers **closed without merging** so they are never proposed again (this can be reset from the UI).

### The lossless guarantee

| Format | Tool | What changes | What doesn't |
|---|---|---|---|
| PNG | [oxipng](https://github.com/shssoichiro/oxipng) (library, built in) | filters, deflate compression, lossless color-type reduction | pixels; metadata (unless you enable *strip metadata*) |
| JPEG | `jpegtran -optimize [-progressive] -copy all` | Huffman tables, progressive mode when smaller | DCT coefficients and **all** metadata (EXIF/orientation, ICC) |
| GIF | `gifsicle -O3` (never `--lossy`) | frame encoding | every composited frame and its delay |

After optimizing, the original and the result are **always** decoded and must match exactly in dimensions and RGBA values (for GIF, every frame and its delay). If verification fails or the format cannot be decoded, the file stays as it is. Animated PNGs (APNG) are skipped. A result is only kept if it saves at least 1% and 256 bytes (configurable).

## Running with Docker (recommended)

One container with the API, the web UI, `git`, `jpegtran` and `gifsicle`.

```bash
export GITHUB_TOKEN=$(gh auth token)                 # or a PAT (see "GitHub token")
export GIT_OPT_ADMIN_TOKEN=$(openssl rand -hex 24)   # protects the UI
docker compose up -d --build
```

Open <http://localhost:8484>. Data (SQLite + clones) lives in the `git-opt-data` volume.

To have commits use your git identity, uncomment the `~/.gitconfig` mount in `docker-compose.yml`; otherwise set the name and email in **Settings**, and if neither is set your GitHub account is used (`name <id+login@users.noreply.github.com>`). If your `.gitconfig` signs commits (`commit.gpgsign`) and the container has no key, turn off **Sign commits** in Settings.

Without compose:

```bash
docker build -t git-opt .
docker run -d --name git-opt -p 127.0.0.1:8484:8484 \
  -e GITHUB_TOKEN -e GIT_OPT_ADMIN_TOKEN \
  -v git-opt-data:/data git-opt
```

## Installing from source

Requirements: Rust ≥ 1.88, Node ≥ 20, `git` and, for JPEG/GIF, `jpegtran` (package `libjpeg-turbo`, `libjpeg-turbo-progs` on Debian) and `gifsicle`. PNG needs nothing external. If a tool is missing, that format is shown as *unavailable* in the UI and skipped.

```bash
(cd frontend && npm ci && npm run build)
cargo build --release
./target/release/git-opt          # serves API + UI on http://127.0.0.1:8484
```

As a systemd user service (so it uses your `git config` and your `gh` session):

```bash
install -Dm755 target/release/git-opt ~/.local/bin/git-opt
mkdir -p ~/.local/share/git-opt && cp -r frontend/dist ~/.local/share/git-opt/web
install -Dm644 contrib/git-opt.service ~/.config/systemd/user/git-opt.service
systemctl --user daemon-reload && systemctl --user enable --now git-opt
loginctl enable-linger "$USER"     # keep it running while logged out
```

## GitHub token

Every API call and every `push` uses **your** token, so PRs, forks and commits show up under your account. It is looked up in this order:

1. The token saved in the UI's **Settings** (write-only, never displayed).
2. Environment variables `GIT_OPT_GITHUB_TOKEN`, `GITHUB_TOKEN` or `GH_TOKEN`.
3. `gh auth token` (the GitHub CLI session of the user running the service).

Permissions: the token from `gh auth login` works as is. A classic PAT needs `repo` and `workflow` (`workflow` is required to push branches to a fork when the original repository uses GitHub Actions). The token never appears on a command line: it is handed to `git` as an HTTP header through `GIT_CONFIG_*` environment variables.

## Using the UI

- **Dashboard**: totals, running and queued jobs with their current phase, recent activity, which account PRs are sent as, and which optimizers are available.
- **Repositories**: add `owner/repo` or a URL. The mode is picked automatically: **direct** if you can push, **fork (contribution)** for third-party repositories. Turn off *Open PRs automatically* to review first: the bot prepares the branch and the PR is opened with **Open PR**. From here you can also check now, force a check, pause and delete.
- **Repository detail**: per-repo interval, mode, ignored files, run history and "forget rejected files".
- **Sources**: add a user, an organization or "my repositories" (everything the token can access). They are re-synced periodically to pick up new repositories.
- **Activity**: every run with per-file details.
- **Settings**: intervals, concurrency, thresholds, oxipng level, global ignored files, branch, commit/PR title, author, signing and token.

Ignored files use simplified `.gitignore` syntax: `*.min.png` (any depth), `docs/` (a directory), `assets/raw/*.jpg`. Lines starting with `#` are comments.

## Configuration

Environment variables (everything else is configured from the UI and stored in the database):

| Variable | Default | Description |
|---|---|---|
| `GIT_OPT_BIND` | `127.0.0.1:8484` (`0.0.0.0:8484` in Docker) | HTTP address |
| `GIT_OPT_DATA_DIR` | `~/.local/share/git-opt` (`/data` in Docker) | SQLite and clones |
| `GIT_OPT_STATIC_DIR` | `frontend/dist` if present | Built web UI |
| `GIT_OPT_ADMIN_TOKEN` | — | If set, the API requires `Authorization: Bearer <token>` |
| `GIT_OPT_GITHUB_API` | `https://api.github.com` | For GitHub Enterprise: `https://<host>/api/v3` |
| `GIT_OPT_GITHUB_TOKEN` / `GITHUB_TOKEN` / `GH_TOKEN` | — | GitHub token |
| `RUST_LOG` | `git_opt=info,tower_http=warn` | Log level |

## Development

```bash
cargo test                            # backend tests
cargo run                             # API on :8484
(cd frontend && npm run dev)          # Vite on :5173, proxying /api → :8484
scripts/e2e.sh                        # full flow against a fake GitHub and local repos
```

`scripts/e2e.sh` starts a fake GitHub API (`scripts/mock-github.py`) and local *bare* repositories, adds a repo, waits for the PR and checks with ImageMagick that the pixels are identical. It never touches the real GitHub.

### Layout

```
backend/src/
  main.rs         startup: config, DB, scheduler, HTTP server + static UI
  api.rs          JSON API (axum)
  scheduler.rs    check ticker and dispatcher with bounded concurrency
  jobs.rs         in-memory queue, per-repo deduplication and live progress
  pipeline.rs     what happens to each repo (fetch → candidates → optimize → commit → PR)
  git.rs          wrapper around the system git
  github.rs       GitHub REST client and token resolution
  optimizers/     Optimizer trait + oxipng, jpegtran, gifsicle and pixel verification
  report.rs       commit message and PR body
  db.rs           every SQLite query;  migrations/ holds the schema
frontend/         Vue 3 + Vite + TypeScript UI
```

### Adding a new file type

Implement `optimizers::Optimizer` (id, extensions, `profile`, `probe`, `optimize` and, above all, `verify`, which must **prove** the result is equivalent) and register it in `Registry::new`. It shows up in the UI with its own toggle automatically; the pipeline, cache and reports need no changes.

## Ideas for later

SVG (svgo with rasterized verification), lossless WebP, GitLab/Gitea, webhooks instead of polling, and an in-repo config file (`.git-opt.json`) so maintainers can tune the bot's behavior.
