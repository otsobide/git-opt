# Getting started

This guide takes you from nothing to your first pull request. It takes about five minutes.

## What you need

- A GitHub account and a token for it. The easiest is the GitHub CLI: `gh auth login`, then `gh auth token` prints a token with the right scopes. A classic personal access token with `repo` and `workflow` works too (see [configuration.md](configuration.md#github-token)).
- Either **Docker** (recommended), or **Rust ≥ 1.88** with a C compiler, **Node ≥ 20**, **git** and, for GIF support, `gifsicle` (every other format is built in).

git-opt acts as you: commits carry your git identity and PRs are opened by your account. Nothing is ever pushed to a repository's default branch; the bot only pushes its own branch (`git-opt/optimize-images`) and opens a PR.

## Option A: Docker

```bash
git clone https://github.com/otsobide/git-opt.git && cd git-opt
export GITHUB_TOKEN=$(gh auth token)
export GIT_OPT_ADMIN_TOKEN=$(openssl rand -hex 24)   # keep it: the UI will ask for it
docker compose up -d --build
```

The image bundles everything (API, UI, every optimizer, `git`, `gifsicle`) and stores its data in the `git-opt-data` volume. Open <http://localhost:8484> and paste the admin token.

Optional, in `docker-compose.yml`:

- uncomment the `~/.gitconfig` mount so commits use your usual name and email;
- change the published port, or keep `127.0.0.1:` to make it reachable only from this machine.

Logs: `docker compose logs -f git-opt`. Upgrade: `git pull && docker compose up -d --build` (migrations run automatically).

## Option B: from source

```bash
git clone https://github.com/otsobide/git-opt.git && cd git-opt
(cd frontend && npm ci && npm run build)
cargo build --release
./target/release/git-opt
```

It listens on <http://127.0.0.1:8484>, picks up your `git config` and, if no token is configured, your `gh` session. Data goes to `~/.local/share/git-opt`.

To keep it running in the background as your user:

```bash
install -Dm755 target/release/git-opt ~/.local/bin/git-opt
mkdir -p ~/.local/share/git-opt && cp -r frontend/dist ~/.local/share/git-opt/web
install -Dm644 contrib/git-opt.service ~/.config/systemd/user/git-opt.service
systemctl --user daemon-reload && systemctl --user enable --now git-opt
loginctl enable-linger "$USER"     # keep running while you are logged out
journalctl --user -u git-opt -f    # logs
```

## First steps in the UI

1. **Check the Dashboard.** The *Identity* panel must show your GitHub login under *PRs* and your name under *Commits*. The *Optimizers* panel lists which formats are available. Any problem (no token, missing tool) is shown as a banner at the top.

2. **Add a first repository with review.** Go to **Repositories**, type `owner/name` (or paste its URL), untick **Open PRs automatically** and press **Add**. The first check starts right away: the row shows what it is doing (cloning, scanning, optimizing…).

3. **Look at the result.** When the row says **Awaiting approval**, open the repository and then the latest run in its *History*: you see every file, its size before and after and the savings. Nothing has left your machine yet.

4. **Open the PR.** Press **Open PR**. git-opt pushes the branch and opens the pull request in your name. From then on it updates that PR while it stays open.

5. **Let it run.** Turn **Open PRs automatically** back on if you like, or import many repositories at once from **Sources** (a user, an organization, or every repository you can access). Every repository is checked every 20 minutes plus a random delay of up to 20 minutes.

Next: [usage.md](usage.md) explains every screen and the common workflows; [how-it-works.md](how-it-works.md) explains what happens behind them.
