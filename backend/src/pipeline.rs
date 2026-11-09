//! The work done for one repository: fetch, find unoptimized files, optimize,
//! commit, push and open/update the pull request.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Serialize;

use crate::{
    app::App,
    db::{NewRunFile, RunOutcome},
    git::{Git, GitAuth, Identity, TreeEntry, url_prefix},
    github::{ApiRepo, parse_repo_ref},
    models::{NewRepo, Repo, RepoMode, RunTrigger},
    optimizers::{self, Optimizer, Outcome},
    report::{self, FileReport},
    settings::Settings,
};

/// Requested mode when adding a repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeRequest {
    #[default]
    Auto,
    Direct,
    Fork,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentitySource {
    Settings,
    Git,
    Github,
}

struct Candidate {
    entry: TreeEntry,
    optimizer: Arc<dyn Optimizer>,
    profile: String,
    cached_blob: Option<String>,
}

struct Optimized {
    candidate: Candidate,
    after: u64,
}

struct PrResult {
    status: &'static str,
    number: i64,
    url: String,
}

// ------------------------------------------------------------------- helpers

fn progress(app: &App, repo: &Repo, phase: impl Into<String>) {
    app.jobs.set_phase(repo.id, &repo.full_name(), phase);
}

/// Git handle for a repository clone, authenticated with the developer's token.
pub async fn repo_git(app: &App, owner: &str, name: &str, clone_url: &str) -> Git {
    let auth = match (app.github.token().await, url_prefix(clone_url)) {
        (Some((token, _)), Some(url_prefix)) => Some(GitAuth { url_prefix, token }),
        _ => None,
    };
    Git::new(app.config.repos_dir().join(owner).join(name), auth)
}

pub fn next_check_at(settings: &Settings, repo_interval: Option<i64>) -> DateTime<Utc> {
    let base = repo_interval.map_or(settings.poll_interval_minutes as i64, |m| m.max(1)) * 60;
    let jitter = fastrand::i64(0..=settings.poll_jitter_minutes as i64 * 60);
    Utc::now() + chrono::Duration::seconds(base + jitter)
}

fn build_globs(lists: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for line in lists.iter().flat_map(|l| l.lines()) {
        let pattern = line.trim();
        if pattern.is_empty() || pattern.starts_with('#') {
            continue;
        }
        let pattern = pattern.trim_start_matches('/');
        let pattern = match pattern.strip_suffix('/') {
            Some(dir) => format!("{dir}/**"),
            None => pattern.to_string(),
        };
        // Like .gitignore: a pattern without a slash matches at any depth.
        let pattern = if pattern.contains('/') { pattern } else { format!("**/{pattern}") };
        builder.add(
            GlobBuilder::new(&pattern)
                .literal_separator(true)
                .build()
                .with_context(|| format!("invalid ignore pattern {line:?}"))?,
        );
    }
    Ok(builder.build()?)
}

/// Who authors the bot's commits: explicit setting, then the machine's
/// `git config`, then the GitHub account behind the token.
pub async fn resolve_identity(app: &App, settings: &Settings, git: Option<&Git>) -> Result<(Identity, IdentitySource)> {
    let (name, email) = (settings.commit_author_name.trim(), settings.commit_author_email.trim());
    if !name.is_empty() && !email.is_empty() {
        return Ok((Identity { name: name.into(), email: email.into() }, IdentitySource::Settings));
    }
    let probe = git.cloned().unwrap_or_else(|| Git::new(std::env::temp_dir(), None));
    if let (Some(name), Some(email)) = (probe.config_get("user.name").await, probe.config_get("user.email").await) {
        return Ok((Identity { name, email }, IdentitySource::Git));
    }
    let user = app.github.user().await.context("cannot determine a commit author")?;
    Ok((
        Identity { name: user.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| user.login.clone()), email: user.noreply_email() },
        IdentitySource::Github,
    ))
}

// ---------------------------------------------------------------- add repo

pub async fn add_repo(app: &App, input: &str, mode: ModeRequest, auto_pr: bool, source_id: Option<i64>) -> Result<(i64, bool)> {
    let (owner, name) = parse_repo_ref(input)?;
    let meta = app.github.repo(&owner, &name).await?;
    if meta.archived || meta.disabled {
        bail!("{} is archived: it cannot receive pull requests", meta.full_name);
    }
    let me = app.github.user().await?;
    let mode = match mode {
        ModeRequest::Auto if meta.can_push() => RepoMode::Direct,
        ModeRequest::Auto => RepoMode::Fork,
        ModeRequest::Direct if !meta.can_push() => {
            bail!("no push access to {}: add it in fork (contribution) mode", meta.full_name)
        }
        ModeRequest::Direct => RepoMode::Direct,
        ModeRequest::Fork if meta.owner.login.eq_ignore_ascii_case(&me.login) => {
            bail!("{} is yours: GitHub does not let you fork your own repository", meta.full_name)
        }
        ModeRequest::Fork => RepoMode::Fork,
    };
    app.db
        .insert_repo(&NewRepo {
            owner: meta.owner.login.clone(),
            name: meta.name.clone(),
            source_id,
            mode,
            clone_url: meta.clone_url.clone(),
            default_branch: meta.branch(),
            auto_pr,
        })
        .await
}

pub async fn sync_source(app: &App, source_id: i64) -> Result<()> {
    let Some(src) = app.db.source(source_id).await? else { return Ok(()) };
    let listed = match app.github.list_repos(src.kind, &src.name).await {
        Ok(list) => list,
        Err(e) => {
            let msg = format!("{e:#}");
            app.db.mark_source_synced(src.id, None, Some(&msg)).await?;
            bail!(msg);
        }
    };
    let me = app.github.user().await?;
    let mut count = 0;
    for r in listed {
        if r.disabled || (r.archived && !src.include_archived) || (r.fork && !src.include_forks) {
            continue;
        }
        count += 1;
        let mode = if r.can_push() || r.owner.login.eq_ignore_ascii_case(&me.login) {
            RepoMode::Direct
        } else {
            RepoMode::Fork
        };
        app.db
            .insert_repo(&NewRepo {
                owner: r.owner.login.clone(),
                name: r.name.clone(),
                source_id: Some(src.id),
                mode,
                clone_url: r.clone_url.clone(),
                default_branch: r.branch(),
                auto_pr: true,
            })
            .await?;
    }
    app.db.mark_source_synced(src.id, Some(count), None).await?;
    tracing::info!(source = src.id, count, "source synced");
    Ok(())
}

// ------------------------------------------------------------------- check

/// Entry point for a check job. Records the run and schedules the next one.
pub async fn check(app: &App, repo_id: i64, trigger: RunTrigger, force: bool) -> Result<()> {
    let Some(repo) = app.db.repo(repo_id).await? else { return Ok(()) };
    let settings = app.db.settings().await?;
    let started = Utc::now();
    let result = run_check(app, &repo, &settings, force).await;
    let next = next_check_at(&settings, repo.poll_interval_minutes);

    match result {
        Ok((outcome, files)) => {
            let fresh = app.db.repo(repo_id).await?;
            let status = if fresh.as_ref().is_some_and(|r| r.pending_commit.is_some()) { "awaiting_approval" } else { "ok" };
            app.db.finish_check(repo_id, status, None, outcome.head_sha.as_deref(), next).await?;
            // Routine polls with nothing new are not worth a history row.
            if outcome.status != "no_changes" || trigger != RunTrigger::Schedule {
                let run_id = app.db.insert_run(repo_id, trigger.as_str(), started, &outcome).await?;
                app.db.add_run_files(run_id, &files).await?;
            }
            tracing::info!(repo = %repo.full_name(), status = %outcome.status, "check finished");
        }
        Err(e) => {
            let msg = format!("{e:#}");
            tracing::warn!(repo = %repo.full_name(), error = %msg, "check failed");
            // Same error again on a schedule: update the repo, skip the duplicate row.
            if trigger != RunTrigger::Schedule || repo.last_error.as_deref() != Some(msg.as_str()) {
                let outcome = RunOutcome { status: "error".into(), message: Some(msg.clone()), ..Default::default() };
                app.db.insert_run(repo_id, trigger.as_str(), started, &outcome).await?;
            }
            app.db.finish_check(repo_id, "error", Some(&msg), None, next).await?;
        }
    }
    Ok(())
}

async fn run_check(app: &App, repo: &Repo, settings: &Settings, force: bool) -> Result<(RunOutcome, Vec<NewRunFile>)> {
    progress(app, repo, "Consultando GitHub");
    let meta = app.github.repo(&repo.owner, &repo.name).await?;
    if meta.archived || meta.disabled {
        bail!("repository is archived: disable it or remove it");
    }
    let branch = meta.branch();
    if meta.clone_url != repo.clone_url || branch != repo.default_branch {
        app.db.refresh_repo_meta(repo.id, &meta.clone_url, &branch).await?;
    }
    if let Err(e) = refresh_pr_state(app, repo).await {
        tracing::warn!(repo = %repo.full_name(), error = %format!("{e:#}"), "could not refresh PR state");
    }

    let git = repo_git(app, &repo.owner, &repo.name, &meta.clone_url).await;
    sync_clone(app, repo, &git, &meta.clone_url, &branch).await?;
    let upstream = format!("refs/remotes/origin/{branch}");
    let head = git.rev_parse(&upstream).await?;
    let mut outcome = RunOutcome { head_sha: Some(head.clone()), ..Default::default() };

    if !force && repo.last_seen_sha.as_deref() == Some(head.as_str()) {
        outcome.status = "no_changes".into();
        return Ok((outcome, Vec::new()));
    }

    progress(app, repo, "Revisando historial de commits");
    let (candidates, scanned) = find_candidates(app, repo, settings, &git, &upstream).await?;
    outcome.files_scanned = scanned as i64;
    if candidates.is_empty() {
        outcome.status = "up_to_date".into();
        return Ok((outcome, Vec::new()));
    }

    progress(app, repo, "Preparando rama");
    git.checkout_fresh(&settings.branch_name, &upstream).await?;
    let optimized = optimize_files(app, repo, settings, &git, candidates).await?;
    if optimized.is_empty() {
        outcome.status = "up_to_date".into();
        return Ok((outcome, Vec::new()));
    }

    progress(app, repo, "Creando commit");
    let reports: Vec<FileReport> = optimized
        .iter()
        .map(|o| FileReport {
            path: o.candidate.entry.path.clone(),
            optimizer: o.candidate.optimizer.id().into(),
            before: o.candidate.entry.size,
            after: o.after,
        })
        .collect();
    let (identity, _) = resolve_identity(app, settings, Some(&git)).await?;
    let message = report::commit_message(&settings.commit_title, &reports);
    let paths: Vec<String> = reports.iter().map(|r| r.path.clone()).collect();
    let (commit_sha, new_blobs) = git.commit(&paths, &message, &identity, settings.sign_commits).await?;
    let new_blobs: HashMap<String, String> = new_blobs.into_iter().collect();

    let mut files = Vec::with_capacity(optimized.len());
    for o in &optimized {
        let e = &o.candidate.entry;
        app.db
            .cache_put(&e.sha, &o.candidate.profile, "optimized", new_blobs.get(&e.path).map(String::as_str), e.size as i64, o.after as i64, None)
            .await?;
        files.push(NewRunFile {
            path: e.path.clone(),
            optimizer: o.candidate.optimizer.id().into(),
            original_blob: e.sha.clone(),
            bytes_before: e.size as i64,
            bytes_after: o.after as i64,
        });
    }

    let (before, after) = report::totals(&reports);
    outcome.commit_sha = Some(commit_sha.clone());
    outcome.files_optimized = files.len() as i64;
    outcome.bytes_before = before as i64;
    outcome.bytes_after = after as i64;

    if repo.auto_pr {
        let pr = publish_branch(app, repo, &meta, &git, settings, &reports).await?;
        app.db.set_pending_commit(repo.id, None).await?;
        outcome.status = pr.status.into();
        outcome.pr_number = Some(pr.number);
        outcome.pr_url = Some(pr.url);
    } else {
        app.db.set_pending_commit(repo.id, Some(&commit_sha)).await?;
        outcome.status = "awaiting_approval".into();
    }
    Ok((outcome, files))
}

/// Clones on first use and fetches afterwards; re-clones only a broken checkout.
async fn sync_clone(app: &App, repo: &Repo, git: &Git, url: &str, branch: &str) -> Result<()> {
    if git.is_repo() {
        progress(app, repo, "Descargando cambios");
        let fetched = async {
            git.set_origin(url).await?;
            git.fetch_branch(branch).await
        }
        .await;
        match fetched {
            Ok(()) => return Ok(()),
            // A healthy clone means the problem is upstream (network, auth): keep it.
            Err(e) if git.rev_parse("HEAD").await.is_ok() => return Err(e),
            Err(e) => tracing::warn!(repo = %repo.full_name(), error = %format!("{e:#}"), "broken clone, re-cloning"),
        }
    }
    progress(app, repo, "Clonando repositorio");
    git.clone_from(url, branch).await
}

/// Files that have an optimizer, are not ignored and were never optimized:
/// not by a bot commit in history, not rejected in a closed PR, and not
/// already found optimal with the current settings.
async fn find_candidates(
    app: &App,
    repo: &Repo,
    settings: &Settings,
    git: &Git,
    upstream: &str,
) -> Result<(Vec<Candidate>, usize)> {
    let active = app.optimizers.active(settings).await;
    if active.is_empty() {
        bail!("no optimizer is enabled and available");
    }
    let already_optimized = git.blobs_from_bot_commits(upstream).await?;
    let rejected = app.db.ignored_blobs(repo.id).await?;
    let ignore = build_globs(&[&settings.ignore_globs, &repo.ignore_globs])?;
    let max_size = settings.max_file_size_mb as u64 * 1024 * 1024;

    let mut scanned = 0;
    let mut candidates = Vec::new();
    for entry in git.ls_tree(upstream).await? {
        let Some(optimizer) = optimizers::for_path(&active, &entry.path) else { continue };
        if ignore.is_match(&entry.path) {
            continue;
        }
        scanned += 1;
        if entry.size == 0
            || entry.size > max_size
            || already_optimized.contains(&entry.sha)
            || rejected.contains(&entry.sha)
        {
            continue;
        }
        let profile = optimizer.profile(settings);
        let cached = app.db.cache_get(&entry.sha, &profile).await?;
        if cached.as_ref().is_some_and(|c| c.result != "optimized") {
            continue;
        }
        candidates.push(Candidate {
            optimizer: optimizer.clone(),
            profile,
            cached_blob: cached.and_then(|c| c.optimized_blob),
            entry,
        });
    }
    Ok((candidates, scanned))
}

async fn optimize_files(
    app: &App,
    repo: &Repo,
    settings: &Settings,
    git: &Git,
    mut candidates: Vec<Candidate>,
) -> Result<Vec<Optimized>> {
    let paths: Vec<&str> = candidates.iter().map(|c| c.entry.path.as_str()).collect();
    let lfs = git.lfs_paths(&paths).await?;
    candidates.retain(|c| !lfs.contains(&c.entry.path));

    let total = candidates.len();
    let mut optimized = Vec::new();
    for (i, c) in candidates.into_iter().enumerate() {
        progress(app, repo, format!("Optimizando {}/{}: {}", i + 1, total, c.entry.path));
        let path = git.dir().join(&c.entry.path);

        // Same input bytes as a previous run: reuse that verified output.
        if let Some(blob) = &c.cached_blob
            && git.has_object(blob).await
        {
            let data = git.cat_blob(blob).await?;
            tokio::fs::write(&path, &data).await?;
            optimized.push(Optimized { after: data.len() as u64, candidate: c });
            continue;
        }

        let data = tokio::fs::read(&path).await.with_context(|| format!("reading {}", c.entry.path))?;
        if data.starts_with(b"version https://git-lfs") {
            continue;
        }
        let before = data.len() as i64;
        match optimizers::process(c.optimizer.as_ref(), Arc::new(data), settings).await {
            Outcome::Optimized(out) => {
                tokio::fs::write(&path, &out).await?;
                optimized.push(Optimized { after: out.len() as u64, candidate: c });
            }
            Outcome::Optimal(detail) => {
                app.db.cache_put(&c.entry.sha, &c.profile, "optimal", None, before, before, detail.as_deref()).await?;
            }
            Outcome::Failed(err) => {
                tracing::debug!(repo = %repo.full_name(), path = %c.entry.path, error = %err, "not optimized");
                app.db.cache_put(&c.entry.sha, &c.profile, "failed", None, before, before, Some(&err)).await?;
            }
        }
    }
    Ok(optimized)
}

/// Updates the stored PR state; learns from merges and rejections.
async fn refresh_pr_state(app: &App, repo: &Repo) -> Result<()> {
    let (Some(number), Some("open")) = (repo.pr_number, repo.pr_state.as_deref()) else { return Ok(()) };
    let pr = app.github.pull(&repo.owner, &repo.name, number).await?;
    let state = pr.status();
    if state == "open" {
        return Ok(());
    }
    if let Some(run) = app.db.last_run_for_pr(repo.id, number).await? {
        if state == "merged" {
            app.db.add_bytes_saved(repo.id, run.bytes_before - run.bytes_after).await?;
        } else {
            // Closed without merging: the maintainers do not want these files touched.
            app.db.ignore_run_blobs(repo.id, run.id).await?;
        }
    }
    app.db.set_pr(repo.id, Some(number), Some(&pr.html_url), Some(state)).await
}

// ----------------------------------------------------------------- publish

/// Pushes the bot branch (to the repo or to the developer's fork) and opens
/// or updates the PR, always with the developer's token.
async fn publish_branch(
    app: &App,
    repo: &Repo,
    meta: &ApiRepo,
    git: &Git,
    settings: &Settings,
    reports: &[FileReport],
) -> Result<PrResult> {
    let branch = &settings.branch_name;
    let (push_url, head_owner) = match repo.mode {
        RepoMode::Direct => (meta.clone_url.clone(), meta.owner.login.clone()),
        RepoMode::Fork => {
            progress(app, repo, "Preparando fork");
            ensure_fork(app, repo).await?
        }
    };

    progress(app, repo, "Subiendo rama");
    let mut attempt = 0;
    loop {
        match git.push(&push_url, branch).await {
            Ok(()) => break,
            // A brand-new fork can take a moment to accept pushes.
            Err(e) if attempt < 3 && repo.mode == RepoMode::Fork => {
                attempt += 1;
                tracing::debug!(error = %e, attempt, "push to fork failed, retrying");
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
            }
            Err(e) if repo.mode == RepoMode::Direct => {
                return Err(e.context("push rejected: if you lack write access, switch the repo to fork mode"));
            }
            Err(e) => return Err(e),
        }
    }

    progress(app, repo, "Abriendo pull request");
    let title = &settings.commit_title;
    let body = report::pr_body(reports);
    let base = meta.branch();
    let (status, pr) = match app.github.find_open_pull(&repo.owner, &repo.name, &head_owner, branch).await? {
        Some(existing) => {
            ("pr_updated", app.github.update_pull(&repo.owner, &repo.name, existing.number, title, &body).await?)
        }
        None => {
            let head = match repo.mode {
                RepoMode::Direct => branch.clone(),
                RepoMode::Fork => format!("{head_owner}:{branch}"),
            };
            ("pr_opened", app.github.create_pull(&repo.owner, &repo.name, &head, &base, title, &body).await?)
        }
    };
    app.db.set_pr(repo.id, Some(pr.number), Some(&pr.html_url), Some("open")).await?;
    Ok(PrResult { status, number: pr.number, url: pr.html_url })
}

/// Returns (clone URL, owner login) of the developer's fork, creating it if needed.
async fn ensure_fork(app: &App, repo: &Repo) -> Result<(String, String)> {
    if let (Some(full), Some(url)) = (&repo.fork_full_name, &repo.fork_clone_url)
        && let Some((owner, name)) = full.split_once('/')
        && app.github.repo(owner, name).await.is_ok()
    {
        return Ok((url.clone(), owner.to_string()));
    }
    let fork = app.github.fork(&repo.owner, &repo.name).await?;
    app.db.set_fork(repo.id, &fork.full_name, &fork.clone_url).await?;
    Ok((fork.clone_url, fork.owner.login))
}

/// Manual approval: publish the branch prepared by a previous check.
pub async fn publish(app: &App, repo_id: i64) -> Result<()> {
    let Some(repo) = app.db.repo(repo_id).await? else { return Ok(()) };
    let started = Utc::now();
    let result = run_publish(app, &repo).await;
    match result {
        Ok((outcome, files)) => {
            let run_id = app.db.insert_run(repo.id, RunTrigger::Publish.as_str(), started, &outcome).await?;
            app.db.add_run_files(run_id, &files).await?;
            app.db.set_pending_commit(repo.id, None).await?;
            app.db.set_status(repo.id, "ok", None).await?;
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let outcome = RunOutcome { status: "error".into(), message: Some(msg.clone()), ..Default::default() };
            app.db.insert_run(repo.id, RunTrigger::Publish.as_str(), started, &outcome).await?;
            app.db.set_status(repo.id, &repo.status, Some(&msg)).await?;
        }
    }
    Ok(())
}

async fn run_publish(app: &App, repo: &Repo) -> Result<(RunOutcome, Vec<NewRunFile>)> {
    let settings = app.db.settings().await?;
    let Some(commit) = repo.pending_commit.clone() else { bail!("nothing is awaiting approval") };
    let git = repo_git(app, &repo.owner, &repo.name, &repo.clone_url).await;
    let branch_head = git.rev_parse(&format!("refs/heads/{}", settings.branch_name)).await.ok();
    if branch_head.as_deref() != Some(commit.as_str()) {
        bail!("the prepared branch changed; run a new check");
    }
    let source = app.db.run_by_commit(repo.id, &commit).await?.context("run for the prepared commit not found")?;
    let files = app.db.run_files(source.id).await?;
    let reports: Vec<FileReport> = files
        .iter()
        .map(|f| FileReport { path: f.path.clone(), optimizer: f.optimizer.clone(), before: f.bytes_before as u64, after: f.bytes_after as u64 })
        .collect();

    let meta = app.github.repo(&repo.owner, &repo.name).await?;
    let pr = publish_branch(app, repo, &meta, &git, &settings, &reports).await?;
    let outcome = RunOutcome {
        status: pr.status.into(),
        head_sha: source.head_sha.clone(),
        commit_sha: Some(commit),
        files_scanned: source.files_scanned,
        files_optimized: source.files_optimized,
        bytes_before: source.bytes_before,
        bytes_after: source.bytes_after,
        pr_number: Some(pr.number),
        pr_url: Some(pr.url),
        message: None,
    };
    let files = files
        .into_iter()
        .map(|f| NewRunFile { path: f.path, optimizer: f.optimizer, original_blob: f.original_blob, bytes_before: f.bytes_before, bytes_after: f.bytes_after })
        .collect();
    Ok((outcome, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_patterns_behave_like_gitignore() {
        let set = build_globs(&["*.min.png\ndocs/\n# comment\n/vendor/**", "assets/raw/*.jpg"]).unwrap();
        assert!(set.is_match("logo.min.png"));
        assert!(set.is_match("a/b/logo.min.png"));
        assert!(set.is_match("docs/img/x.png"));
        assert!(set.is_match("vendor/lib/x.png"));
        assert!(set.is_match("assets/raw/p.jpg"));
        assert!(!set.is_match("assets/p.jpg"));
        assert!(!set.is_match("logo.png"));
    }

    #[test]
    fn next_check_respects_interval_and_jitter() {
        let s = Settings { poll_interval_minutes: 20, poll_jitter_minutes: 20, ..Settings::default() };
        for _ in 0..50 {
            let delta = (next_check_at(&s, None) - Utc::now()).num_seconds();
            assert!((20 * 60 - 2..=40 * 60).contains(&delta), "{delta}");
        }
        let delta = (next_check_at(&s, Some(60)) - Utc::now()).num_seconds();
        assert!(delta >= 60 * 60 - 2);
    }
}
