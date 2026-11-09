# Configuration

git-opt has three layers of configuration:

1. **Environment variables**: process-level things that need a restart (where to listen, where to store data, admin token).
2. **Global settings**: everything else, edited in the **Settings** page and stored in the database. Changes apply immediately.
3. **Per-repository settings**: edited in each repository's page.

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `GIT_OPT_BIND` | `127.0.0.1:8484` (Docker: `0.0.0.0:8484`) | Address and port of the HTTP server (API + UI). |
| `GIT_OPT_DATA_DIR` | `$XDG_DATA_HOME/git-opt`, usually `~/.local/share/git-opt` (Docker: `/data`) | Holds `git-opt.db` (SQLite) and `repos/<owner>/<name>` (clones). |
| `GIT_OPT_STATIC_DIR` | `frontend/dist` or `../frontend/dist` if they contain `index.html` (Docker: `/app/web`) | Built web UI. If none is found, only the API is served. |
| `GIT_OPT_ADMIN_TOKEN` | unset | When set, every `/api` call except `/api/health` and `/api/auth` requires `Authorization: Bearer <token>`. The UI shows a login screen. |
| `GIT_OPT_GITHUB_API` | `https://api.github.com` | GitHub REST base URL. For GitHub Enterprise Server use `https://<host>/api/v3`. |
| `GIT_OPT_GITHUB_TOKEN`, `GITHUB_TOKEN`, `GH_TOKEN` | unset | GitHub token, checked in this order (after the one saved in Settings). |
| `RUST_LOG` | `git_opt=info,tower_http=warn` | Log filter ([`tracing` syntax](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)). Use `git_opt=debug` to see why each file was or wasn't optimized. |

## GitHub token

The token decides **who** the PRs, forks and pushes belong to. git-opt uses the first one it finds:

1. the token saved in **Settings → GitHub token** (validated against GitHub before saving; write-only);
2. `GIT_OPT_GITHUB_TOKEN`, `GITHUB_TOKEN`, `GH_TOKEN`;
3. `gh auth token --hostname <host>`, i.e. the GitHub CLI session of the user running git-opt.

The resolved token is cached for 5 minutes. The Dashboard shows which source is in use and the account behind it.

| Token type | Works? | Notes |
|---|---|---|
| `gh auth login` (OAuth) | yes | Has `repo`, `read:org` and `workflow` scopes by default. |
| Classic PAT | yes | Needs `repo` and `workflow`. `workflow` is required to push a branch to a fork whose upstream contains GitHub Actions workflows. |
| Fine-grained PAT | partly | Works for your own repositories with *Contents* and *Pull requests* read/write. Forking third-party repositories is generally not possible with fine-grained tokens; use a classic PAT or `gh` for fork mode. |

## Global settings

| Setting (UI) | Key | Default | Range | Effect |
|---|---|---|---|---|
| Check interval | `poll_interval_minutes` | `20` | 1–10080 | Base time between two checks of a repository. |
| Random extra delay | `poll_jitter_minutes` | `20` | 0–1440 | A uniform random delay in `[0, jitter]` is added to every interval. |
| Concurrent repositories | `max_concurrent_jobs` | `2` | 1–32 | How many repositories are processed at the same time. |
| Source re-sync | `source_sync_interval_minutes` | `360` | 5–10080 | How often users/orgs are re-listed to find new repositories. |
| Minimum saving (%) | `min_savings_percent` | `1.0` | 0–99 | A file is only changed if it shrinks at least this much… |
| Minimum saving (bytes) | `min_savings_bytes` | `256` | ≥ 0 | …and at least this many bytes. |
| Maximum file size | `max_file_size_mb` | `30` | 1–1024 | Bigger files are skipped. |
| PNG compression effort | `png_level` | `3` | 0–6 | oxipng preset. Higher is smaller and slower. |
| Strip PNG metadata | `strip_metadata` | off | | Removes PNG chunks that do not affect rendering (text, time…). Color-related chunks are kept. JPEG metadata is never removed. |
| Formats | `optimizers` | all on | | Per-optimizer switch (`oxipng`, `jpegtran`, `gifsicle`). An optimizer whose tool is not installed is reported as unavailable and skipped. |
| Ignored files | `ignore_globs` | empty | | Patterns ignored in every repository (see below). |
| Working branch | `branch_name` | `git-opt/optimize-images` | | Branch the bot pushes to. Changing it while a PR is open leaves the old PR alone and opens a new one. |
| Commit and PR title | `commit_title` | `[git-opt] Optimize images` | | Subject of the commit and title of the PR. |
| Commit author name / email | `commit_author_name`, `commit_author_email` | empty | | Override the commit author. Both must be set. See [author resolution](how-it-works.md#35-commit). |
| Sign commits | `sign_commits` | on | | When off, passes `-c commit.gpgsign=false` to git. |

Changing the interval or the jitter reschedules repositories whose next check would otherwise be later than the new interval allows.

Changing `png_level` or `strip_metadata` changes the PNG optimizer's cache key, so PNGs previously found optimal are evaluated again on the next forced or changed check.

## Per-repository settings

| Setting | Default | Effect |
|---|---|---|
| Watch this repository | on | Off pauses scheduled checks (manual checks still work). Turning it back on checks immediately. |
| Open PRs automatically | on | Off: the bot prepares the commit and waits for **Open PR**. |
| Mode | from permissions | **Direct** (push to the repository) or **Fork (contribution)** (push to your fork). |
| Check interval | global | Overrides the global interval for this repository. The global jitter still applies. |
| Ignored files | empty | Added to the global patterns. |
| Forget rejected files | | Clears the list of files from PRs that were closed without merging. |

## Ignore patterns

One pattern per line, simplified `.gitignore` syntax, matched against the path from the repository root:

| Pattern | Matches |
|---|---|
| `*.min.png` | any `*.min.png` at any depth (a pattern without `/` matches anywhere) |
| `docs/` | everything under `docs/` |
| `assets/raw/*.jpg` | JPEGs directly inside `assets/raw/` (`*` does not cross `/`) |
| `vendor/**` | everything under `vendor/`, at any depth |
| `/logo.png` | a leading `/` is ignored: same as `logo.png` from the root |
| `# comment` | ignored |

There is no negation (`!pattern`).

## Data directory

```
$GIT_OPT_DATA_DIR/
├── git-opt.db          SQLite (WAL mode): settings, repositories, sources, runs, caches
├── git-opt.db-wal
├── git-opt.db-shm
└── repos/
    └── <owner>/<name>/  full clone of the default branch, owned by the bot
```

Clones can be deleted at any time; they are re-created on the next check. Deleting the database loses settings, the watch list and the activity log, but not the knowledge of what was optimized (that is re-learned from the commit history).
