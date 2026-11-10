# How it works

This page follows a repository from the moment it is added until its pull request is merged. For the code layout behind each step see [architecture.md](architecture.md).

## The big picture

```
                       ┌───────────────────────────── git-opt (one process) ─────────────────────────────┐
                       │                                                                                 │
  you ── web UI ──────▶│  API (axum) ──▶ SQLite: repos, sources, settings, runs, blob cache              │
                       │                     ▲                                                           │
                       │  scheduler ─────────┘  every 15 s: "which repos are due?" ──▶ job queue         │
                       │                                                                 │               │
                       │  dispatcher: runs up to N jobs at once (one per repo) ◀─────────┘               │
                       │        │                                                                        │
                       │        ▼                                                                        │
                       │  pipeline:  git fetch ─▶ find candidates ─▶ optimize + verify ─▶ commit ─▶ PR   │
                       └────────┬───────────────────────────────┬─────────────────────────────┬──────────┘
                                │ system git (your identity)    │ built-in optimizers+gifsicle│ GitHub REST API
                                ▼                               ▼                             ▼ (your token)
                       local clones in $DATA/repos      optimized bytes                pull requests, forks
```

Everything runs in a single process: the HTTP API, the web UI, the scheduler and the workers. There is no external queue or database server.

## 1. Adding repositories

A repository enters the watch list in one of two ways:

- **Individually** (`owner/name` or a GitHub URL). git-opt asks GitHub for the repository and decides the **mode**:
  - **direct** if your token can push to it: the bot branch is pushed to the repository itself;
  - **fork** if it cannot (third-party repositories): the branch is pushed to *your* fork and the PR goes from `you:branch` to the original. You can force a mode; forking your own repository or using direct mode without push access is rejected with a clear message.
- **Through a source**: a GitHub user, an organization, or "my repositories" (everything the token can access, including private repos and organizations you belong to). Every repository found is added with the mode that matches your permissions. Forks and archived repositories are skipped unless you ask for them. Sources are re-listed every 6 hours (configurable) so new repositories are picked up automatically; repositories that disappear are kept.

Newly added repositories are checked right away.

## 2. Scheduling

Each repository has a `next_check_at`. Every 15 seconds the scheduler queues the repositories that are due. After a check finishes the next one is scheduled at

```
now + interval + random(0 … jitter)
```

with `interval = 20 min` and `jitter = 20 min` by default (a repository can override the interval). The random part spreads the load so that hundreds of repositories don't all fetch at the same minute.

The job queue guarantees that:

- a repository is never processed by two jobs at the same time, and never queued twice;
- manual actions ("check now", "open PR") jump ahead of the scheduled backlog;
- at most `max_concurrent_jobs` jobs (default 2) run at once. The value is re-read continuously, so changing it in the UI applies immediately.

## 3. A check, step by step

### 3.1 Refresh what GitHub knows

- Read the repository metadata. If the default branch was renamed or the repository moved, the stored values are updated. Archived repositories fail with an explicit error (they cannot receive PRs).
- If the bot has a PR that was **open** last time, ask GitHub for its state:
  - **merged** → the savings of that PR are added to the repository's "saved in merged PRs" counter;
  - **closed without merging** → every original file of that PR is recorded as **rejected** for this repository and will never be proposed again (you can undo this with *Forget rejected files*).

### 3.2 Fetch

The first time, the repository is cloned with `git clone --single-branch --no-tags` into `$GIT_OPT_DATA_DIR/repos/<owner>/<name>`. Afterwards only the default branch is fetched. If a fetch fails because of the network or credentials, the clone is kept and the check fails; the clone is only re-created if it is actually broken.

If the default branch's HEAD is the same commit seen last time, the check ends here as **no changes** (scheduled checks with no changes leave no trace in the activity log). *Force check* skips this shortcut and re-scans everything.

### 3.3 Find candidates

`git ls-tree -r -l` lists every file in the default branch with its blob id and size. A file becomes a **candidate** only if all of these hold:

| Rule | Why |
|---|---|
| An enabled and installed optimizer handles its extension | `.png`, `.jpg/.jpeg/.jpe/.jfif`, `.gif`, `.webp`, `.ico/.cur`, `.icns`, `.svg`, `.svgz` |
| It does not match an ignored pattern (global or per repository) | you decide what the bot never touches |
| It is a regular file (not a symlink or submodule), not empty and not bigger than the size limit (30 MB) | safety and speed |
| Its blob was **not written by a bot commit** in the history | see below |
| Its blob was **not rejected** in a closed PR of this repository | respect maintainers' decisions |
| The blob cache does not already know it as *optimal* or *failed* with the current settings | do not repeat work |

Git LFS files (by `.gitattributes` or by their pointer content) are skipped as well.

#### Recognizing its own work in the history

Every commit the bot creates ends with the trailer

```
Git-Opt: lossless-optimization
```

and every PR body contains the same text. On each check git-opt runs

```
git log --format=%H --raw --no-abbrev --fixed-strings --grep "Git-Opt: lossless-optimization" <default branch>
```

and collects the **new blob ids** written by those commits. Since a blob id is a hash of the file content, this works regardless of how the PR was merged:

- **merge commit** and **rebase**: the bot's own commit is in the history;
- **squash**: GitHub's squash message includes the commit message (or the PR description, depending on the repository settings); both contain the marker.

It also works if git-opt's database is lost: a fresh install re-learns everything from the history. And because it compares content, an image that is edited after being optimized gets a new blob id and is evaluated again.

### 3.4 Optimize and verify

The bot checks out a fresh local branch (`git-opt/optimize-images` by default) at the default branch HEAD and processes the candidates one by one:

1. If the cache says this exact blob was optimized before (for example in a PR that is still open) and the result is still in the local clone, the result is reused without recomputing.
2. Otherwise the optimizer produces a candidate output.
3. The output is kept only if it is smaller by at least **1 %** and **256 bytes** (configurable). Otherwise the blob is cached as *optimal*.
4. The optimizer **proves** the output is equivalent: both versions are decoded and compared pixel by pixel (plus the format-specific checks in the table below). Any difference, or anything that cannot be checked, discards the output and caches the blob as *failed* with the reason.
5. The verified output overwrites the file in the working tree.

| Format | Optimizer | Verification |
|---|---|---|
| PNG | oxipng (built in): filters, deflate (libdeflate, or Zopfli when enabled), lossless bit-depth/color-type/palette reductions. Metadata kept unless *strip metadata* is on (then only chunks that don't affect rendering are removed). Transparent pixels keep their RGB values. | same width, height and RGBA values (compared in 16-bit if the color type changed) |
| APNG | oxipng, which also recompresses every animation frame | the above for the default image, plus the same animation control (frame and loop count) and identical composited frames and delays |
| JPEG | mozjpeg (built in): a lossless transcode of the DCT coefficients (`jpegtran -copy all -optimize`), progressive with mozjpeg's scan search and baseline; the smaller wins. libjpeg warnings (damaged data) abort the transcode. | same decoded pixels, and every APPn/COM segment (EXIF, ICC, XMP, comments) byte for byte and in order; the JFIF header may be added or moved first, as the standard requires |
| GIF | `gifsicle --optimize=3` (never `--lossy`) | same number of frames, same composited RGBA for every frame, same delays |
| WebP | libwebp (built in): only lossless (VP8L) files, re-encoded with `-lossless -z 9 -exact`; the new bitstream replaces the old one inside the original container. Lossy and animated WebP are skipped. | same pixels decoded by `image` (a decoder independent of libwebp), and every other chunk (VP8X, ICCP, EXIF, XMP…) byte for byte |
| ICNS | oxipng on each element holding a PNG, keeping its pixel format; every other element (RLE images, masks, JPEG 2000, `icnV`, `info`, nested dark-mode families) is copied, and the `TOC ` table is rebuilt with the new sizes. Files whose size or TOC don't match their contents are skipped. | same element list; PNG elements keep IHDR and pixels; other elements byte for byte; the new TOC matches the elements |
| ICO, CUR | oxipng on each embedded PNG, without changing its pixel format (some Windows loaders only accept 32-bit RGBA PNGs). BMP images are copied. Icons with gaps, overlaps or trailing data are skipped. | same header and directory fields, same IHDR and pixels for each PNG, identical BMP bytes |
| SVG | built-in cleanup: removes comments (except legal ones: `<!--! … -->` or mentioning license/copyright/SPDX), elements and attributes in editor namespaces (Inkscape, Sodipodi, Illustrator, Sketch, Figma, Serif, Visio…) with their declarations, whitespace-only text between tags (never inside `text`, `tspan`, `style`, `script`, `title`, `desc`, `metadata`, `foreignObject` or under `xml:space="preserve"`) and redundant whitespace inside tags. Raster images embedded as whole-attribute `data:image/(png\|jpeg\|gif\|webp);base64,` URIs go through their own optimizer (if enabled) and are re-encoded in base64 without line breaks. Data URIs inside CSS (`url(data:…)`) and embedded SVG documents are left alone. Nothing else is touched. | (1) roxmltree, a different XML parser, sees the same tree once those removals are applied to the original (embedded images compared by type); (2) every embedded image that changed passes its own format's verification; (3) resvg renders both, embedded images included, to identical pixels |
| SVGZ | decompress (single-member gzip only, checksum verified, up to 64 MiB), apply the SVG cleanup above (kept only if its proof passes, otherwise the content stays as is), recompress with Zopfli (zlib level 9 above 8 MiB). The gzip header (file name, comment, time, flags, extra field) is copied byte for byte. | gzip header byte for byte; the decompressed content is identical or passes the SVG proof |

If nothing was optimized the check ends as **up to date**.

### 3.5 Commit

The commit is created with the system `git`, with hooks disabled (`--no-verify`, `core.hooksPath=/dev/null`). The author is, in order:

1. the name and email set in **Settings**, if both are set;
2. `user.name` / `user.email` from the machine's `git config`;
3. the GitHub account behind the token: its display name (or login) and its private `id+login@users.noreply.github.com` address.

Commit signing follows your git config unless *Sign commits* is turned off.

The message lists every file with its size before and after:

```
[git-opt] Optimize images

*Total -- 421.87 KiB -> 56.78 KiB (86.54%)

img/big.png -- 352.22 KiB -> 1.30 KiB (99.63%)
img/icon.png -- 8.35 KiB -> 742 B (91.32%)
img/photo.jpg -- 61.30 KiB -> 54.75 KiB (10.68%)

Git-Opt: lossless-optimization
```

### 3.6 Push and pull request

With **Open PRs automatically** on (the default):

1. **direct mode**: force-push the branch to the repository; **fork mode**: reuse your fork or create it (waiting until GitHub has it ready) and force-push the branch there, retrying while a new fork warms up.
2. Look for an open PR from that branch. If there is one, update its title and body; otherwise open a new one against the default branch, with *allow edits by maintainers* enabled.

The branch is always rebuilt from the latest default branch, so an open PR never conflicts and always contains every pending optimization. The PR body has a table of files, the total savings and a short explanation of the tools used.

With **Open PRs automatically** off, the commit stays in the local clone and the repository shows **Awaiting approval**. Pressing **Open PR** pushes exactly that commit and opens the PR. If the default branch moves first, the next check rebuilds the branch and the approval applies to the new one.

### 3.7 After the PR

- **Merged**: the next check sees the merged blobs in the history, finds nothing to do and credits the savings.
- **Closed without merging**: those files are never proposed again for that repository.
- **Still open while new images arrive**: the branch is rebuilt with the old and the new optimizations and the PR is updated in place.

### 3.8 Deleting merged forks

Deleting a fork closes the pull requests that come from it, so git-opt only deletes your fork when nothing needs it any more. After each successful check of a repository in **fork mode**, with **Delete my fork once all its pull requests are merged** on (the default), it deletes the fork when all of this holds:

1. git-opt's latest pull request on that repository was **merged**, and no commit is waiting for approval;
2. GitHub confirms the repository is a **fork of that project** and belongs to **your account**;
3. no pull request from **any branch** of the fork (yours included, not just git-opt's) is still open;
4. every branch of the fork is either exactly what a merged pull request delivered, or has no commit that the project's default branch lacks.

If any of them fails, the fork stays and is looked at again on the next check. A pull request closed without merging keeps git-opt's branch unmerged, so the fork stays too. When the fork is deleted, the run is recorded as `fork_deleted` and the repository forgets the fork; if git-opt has something to propose later, it forks again.

Repositories in **direct mode** (your own, or any you can push to) are never touched, and the project itself is never deleted: the only repository git-opt can delete is your fork. Deleting needs a token with the `delete_repo` scope; without it, the attempt is recorded once as an error and the check itself still succeeds.

## 4. What gets recorded

Each check that does something (or that you trigger manually) is stored as a **run** with its trigger (`schedule`, `manual`, `publish`), its result, the default branch commit, the bot commit, the number of images scanned and optimized, bytes before and after, the PR, and a message for errors. Runs that changed files keep the per-file details. Errors are recorded too, but the same error repeating on every scheduled check is stored only once.

| Run result | Meaning |
|---|---|
| `no_changes` | the default branch did not move (only recorded for manual checks) |
| `up_to_date` | something changed but there is nothing left to optimize |
| `awaiting_approval` | a commit is ready locally, waiting for **Open PR** |
| `pr_opened` / `pr_updated` | the branch was pushed and the PR created or updated |
| `fork_deleted` | every pull request from your fork was merged and the fork was deleted ([3.8](#38-deleting-merged-forks)) |
| `error` | see the message; the repository will be retried at the next interval |

## 5. Security model

- git-opt acts **as you**. Anything it does on GitHub (forks, branches, PRs) appears under your account, with your token's permissions.
- The only repository it ever deletes is **your own fork** of a project in fork mode, after checking it is a fork of that project and that nothing in it would be lost ([3.8](#38-deleting-merged-forks)).
- The token is never passed on a command line or embedded in a URL: it is given to `git` as an HTTP `Authorization` header through `GIT_CONFIG_COUNT/KEY/VALUE` environment variables, and to the GitHub API as a bearer token. It is never logged or returned by the API.
- The web UI and API have no users. Bind them to localhost (the default) or set `GIT_OPT_ADMIN_TOKEN`, which then must be sent as `Authorization: Bearer …` (the UI asks for it).
- Clones are private to the bot; it never runs repository hooks.
