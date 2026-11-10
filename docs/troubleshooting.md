# Troubleshooting

Start with the **Dashboard**: it shows whether a GitHub token was found and which account it belongs to, the commit identity, and which optimizers are available. For a failing repository, open it; the last error and every failed run's message are shown there. For more detail run with `RUST_LOG=git_opt=debug`.

## GitHub and tokens

**"No GitHub token: set one in Settings, export GITHUB_TOKEN or run `gh auth login`."**
None of the [token sources](configuration.md#github-token) produced a token. In Docker the `gh` CLI of the host is not available: pass `GITHUB_TOKEN=$(gh auth token)` or save a token in Settings.

**"GitHub API 401 Unauthorized (token invalid or expired)"**
The token was revoked or expired. Replace it; tokens are re-read every 5 minutes, or immediately when saved from Settings.

**"GitHub API 403 Forbidden (insufficient permissions or rate limited)"**
Either the token lacks a scope (see the table in [configuration.md](configuration.md#github-token)) or the account hit the API rate limit (5000 requests/hour). git-opt makes very few API calls per check (one or two), so rate limits usually come from other tools sharing the token.

**"GitHub API 404 Not Found (not found or no access)"** when adding a repository or a source
The name is wrong, or the token cannot see a private repository/organization. For organizations with SSO, the token must be authorized for that organization.

## Pushing

**"push rejected: if you lack write access, switch the repo to fork mode"**
The repository is in direct mode but the token cannot push (permissions changed, or branch protection forbids pushing new branches). Switch it to **Fork (contribution)** in the repository page.

**"refusing to allow an OAuth App to create or update workflow … without `workflow` scope"**
Pushing to a fork that is behind an upstream that changed `.github/workflows` requires the `workflow` scope. Use a classic PAT with `repo` + `workflow`, or `gh auth refresh -s workflow`.

**"could not delete the fork: the token needs the delete_repo scope"**
Every pull request from your fork was merged, but the token cannot delete repositories. Run `gh auth refresh -s delete_repo` (or add the scope to your classic PAT), or untick **Delete my fork once all its pull requests are merged** in Settings. The error is recorded once; the next check tries again.

**"fork owner/name was not ready after 60s"**
GitHub creates forks asynchronously and can be slow for big repositories. The next check reuses the fork once it exists.

**Commits fail with a GPG / signing error**
Your git config asks for signed commits (`commit.gpgsign=true`) but no key/agent is reachable (typical in Docker or a headless service). Turn off **Sign commits** in Settings, or make the agent available.

**"Author identity unknown"** or commits with an unexpected author
See [author resolution](how-it-works.md#35-commit). In Docker, mount your `~/.gitconfig` read-only or set the author in Settings.

## Optimization

**GIF shows as unavailable**
GIF is the only format that needs an external tool: install `gifsicle`, then save Settings (which re-probes the tools) or restart. The Docker image includes it. Every other optimizer is built into the binary.

**An image I expected is not optimized**
Run a forced check with `RUST_LOG=git_opt=debug` and look for its path. Usual reasons:

- the saving is below the thresholds (1 % and 256 bytes by default);
- it matches an ignored pattern (global or per repository);
- it is in Git LFS, is a symlink, or exceeds the size limit;
- the format cannot be improved without loss: lossy or animated WebP;
- the file is unusual or damaged: a JPEG that makes libjpeg emit warnings, an ICO/ICNS with gaps, trailing bytes or an inconsistent table of contents, an SVG that is not UTF-8 or that resvg cannot read, an SVGZ with several gzip members, trailing data or a bad checksum;
- the image is embedded in an SVG through CSS (`url(data:…)`) rather than an attribute such as `href`: those are not touched;
- it was in a PR that was closed without merging (use *Forget rejected files*);
- verification failed or the file could not be decoded; the reason is cached (`blob_cache.detail`) and the file will not be retried with the same settings.

**The bot proposes an image again after I merged its PR**
That only happens if the merge did not keep the marker `Git-Opt: lossless-optimization` (for example a squash with a hand-written message). The second proposal will find the image already optimal and do nothing, unless the content changed.

**I want to re-evaluate everything** (for example after raising the PNG level)
Changing PNG settings already invalidates the PNG cache. Use **Force check** to re-scan a repository whose default branch did not move.

## Operations

**Where is the data?** `$GIT_OPT_DATA_DIR` (default `~/.local/share/git-opt`, `/data` in Docker). See [configuration.md](configuration.md#data-directory).

**Disk usage grows**: every watched repository has a full clone of its default branch. Removing a repository deletes its clone; clones can also be deleted by hand and are re-created on the next check.

**Upgrading**: stop, replace the binary (or `docker compose up -d --build`), start. Database migrations run automatically on startup.

**The UI says "web UI not found"** (or only the API answers): build the frontend (`cd frontend && npm run build`) or point `GIT_OPT_STATIC_DIR` at a directory containing `index.html`.

**Exposing the UI on a network**: set `GIT_OPT_ADMIN_TOKEN` and put the service behind a TLS reverse proxy. Anyone with access to the UI can make the bot act on GitHub as you.
