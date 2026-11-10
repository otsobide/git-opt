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

1. the token saved in **Settings → GitHub** (validated against GitHub before saving; write-only; *Remove saved token* deletes it);
2. `GIT_OPT_GITHUB_TOKEN`, `GITHUB_TOKEN`, `GH_TOKEN`;
3. `gh auth token --hostname <host>`, i.e. the GitHub CLI session of the user running git-opt.

The resolved token is cached for 5 minutes. The Dashboard shows which source is in use and the account behind it.

| Token type | Works? | Notes |
|---|---|---|
| `gh auth login` (OAuth) | yes | Has `repo`, `read:org` and `workflow` scopes by default. Add `delete_repo` to delete merged forks: `gh auth refresh -s delete_repo`. |
| Classic PAT | yes | Needs `repo` and `workflow`. `workflow` is required to push a branch to a fork whose upstream contains GitHub Actions workflows. Add `delete_repo` to delete merged forks. |
| Fine-grained PAT | partly | Works for your own repositories with *Contents* and *Pull requests* read/write. Forking third-party repositories is generally not possible with fine-grained tokens; use a classic PAT or `gh` for fork mode. |

## Global settings

| Settings page → field | Key | Default | Range | Effect |
|---|---|---|---|---|
| Checks → Interval (minutes) | `poll_interval_minutes` | `20` | 1–10080 | Base time between two checks of a repository. |
| Checks → Max random delay (minutes) | `poll_jitter_minutes` | `20` | 0–1440 | A uniform random delay in `[0, jitter]` is added to every interval. |
| Checks → Repositories at once | `max_concurrent_jobs` | `2` | 1–32 | How many repositories are processed at the same time. |
| Checks → Re-list sources every (minutes) | `source_sync_interval_minutes` | `360` | 5–10080 | How often users/orgs are re-listed to find new repositories. |
| Optimization → Min savings (%) | `min_savings_percent` | `1.0` | 0–99 | A file is only changed if it shrinks at least this much… |
| Optimization → Min savings (bytes) | `min_savings_bytes` | `256` | ≥ 0 | …and at least this many bytes. |
| Optimization → Max file size (MB) | `max_file_size_mb` | `30` | 1–1024 | Bigger files are skipped. |
| Optimization → PNG compression effort | `png_level` | `3` | 0–6 | oxipng preset. Higher is smaller and slower. |
| Optimization → Use Zopfli for PNG | `png_zopfli` | off | | Zopfli instead of libdeflate for PNG and ICO: a few % smaller, many times slower. |
| Optimization → Strip PNG metadata that doesn't affect rendering | `strip_metadata` | off | | Removes PNG chunks that do not affect rendering (text, time…). Color-related chunks are kept. JPEG metadata is never removed. |
| Optimization → Formats | `optimizers` | all on | | Per-optimizer switch: `oxipng` (PNG/APNG), `mozjpeg` (JPEG), `gifsicle` (GIF), `webp`, `ico` (ICO/CUR), `icns`, `svg`, `svgz`. SVG and SVGZ also use the raster optimizers for embedded images, so turning one of those off also stops it inside SVGs. Only `gifsicle` is an external tool; if it is not installed GIF is reported as unavailable and skipped. |
| Ignored files | `ignore_globs` | empty | | Patterns ignored in every repository (see below). |
| Commits and PRs → Working branch | `branch_name` | `git-opt/optimize-images` | | Branch the bot pushes to. Changing it while a PR is open leaves the old PR alone and opens a new one. |
| Commits and PRs → Commit and PR title | `commit_title` | `[git-opt] Optimize images` | | Subject of the commit and title of the PR. |
| Commits and PRs → Commit author / Author email | `commit_author_name`, `commit_author_email` | empty | | Override the commit author. Both must be set. See [author resolution](how-it-works.md#35-commit). |
| Commits and PRs → Sign commits | `sign_commits` | on | | When off, passes `-c commit.gpgsign=false` to git. |
| Commits and PRs → Delete my fork once all its pull requests are merged | `delete_merged_forks` | on | | Fork mode only: deletes your fork when no pull request from it is open and none of its branches has unmerged commits. See [deleting merged forks](how-it-works.md#38-deleting-merged-forks). Needs the `delete_repo` token scope. |

Changing the interval or the jitter reschedules repositories whose next check would otherwise be later than the new interval allows.

Changing `png_level`, `png_zopfli` or `strip_metadata` changes the PNG (and ICO) optimizer's cache key, so files previously found optimal are evaluated again on the next forced or changed check.

## Per-repository settings

| Setting | Default | Effect |
|---|---|---|
| Watch this repository (*Pause* / *Resume*) | on | Off pauses scheduled checks (manual checks still work). Turning it back on checks immediately. |
| Open PRs automatically | on | Off: the bot prepares the commit and waits for **Open PR**. |
| How to send changes | from permissions | **Direct** (push to the repository) or **Fork (contribution)** (push to your fork). |
| Check frequency | *Use the global interval* | Untick it to set an interval for this repository. The global random delay still applies. |
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
