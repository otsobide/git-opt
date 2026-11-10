//! Check a repository: fetch, find files never optimized, optimize them with
//! their format strategy, commit as the developer, and deliver a pull request
//! (or wait for approval).

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use git_opt_domain::{
    model::{
        BlobVerdict, FileChange, PrState, PullRequest, RepoStatus, Repository, RunOutcome, RunStatus, RunTrigger,
        Settings,
    },
    ports::{CheckRecord, ForgeRepository, FormatStrategy, Outcome, TreeEntry, Workspace, attempt},
    report::{self, BOT_MARKER},
    values::{
        BlobId, BranchName, ByteSize, CloneUrl, CommitSha, FileFormat, IgnoreMatcher, PrNumber, Profile, RepoId,
        Savings,
    },
};

use crate::{Services, delivery};

/// A file worth trying, with the strategy that will handle it.
struct Candidate {
    entry: TreeEntry,
    strategy: Arc<dyn FormatStrategy>,
    profile: Profile,
    /// Verified output from an earlier run (e.g. while its PR is still open).
    cached_output: Option<BlobId>,
}

struct Optimized {
    candidate: Candidate,
    after: ByteSize,
}

/// A check that ran to the end.
pub(crate) struct Checked {
    pub outcome: RunOutcome,
    pub files: Vec<FileChange>,
}

/// A pull request that was opened or updated.
pub(crate) struct Delivered {
    pub status: RunStatus,
    pub number: PrNumber,
    pub url: String,
}

impl Services {
    /// Runs a check, records it and schedules the next one.
    pub async fn check_repository(&self, id: RepoId, trigger: RunTrigger, force: bool) -> Result<()> {
        let p = &self.ports;
        let Some(repo) = p.repositories.get(id).await? else { return Ok(()) };
        let settings = p.settings.load().await?;
        let started = p.clock.now();
        let result = self.run_check(&repo, &settings, force).await;
        let now = p.clock.now();
        let next = repo.poll_policy(&settings).next_check(now, |max| p.random.up_to(max));

        match result {
            Ok(Checked { outcome, files }) => {
                let pending = p.repositories.get(id).await?.is_some_and(|r| r.pending_commit.is_some());
                let status = if pending { RepoStatus::AwaitingApproval } else { RepoStatus::Ok };
                let record = CheckRecord {
                    status,
                    error: None,
                    seen: outcome.head.clone(),
                    checked_at: now,
                    next_check_at: next,
                };
                p.repositories.record_check(id, &record).await?;
                // Routine polls with nothing new are not worth a history row.
                if outcome.status != RunStatus::NoChanges || trigger != RunTrigger::Schedule {
                    p.runs.record(id, trigger, started, now, &outcome, &files).await?;
                }
                tracing::info!(repo = %repo.slug, status = %outcome.status, "check finished");
            }
            Err(e) => {
                let message = format!("{e:#}");
                tracing::warn!(repo = %repo.slug, error = %message, "check failed");
                // The same error again on a schedule: update the repo, skip the duplicate row.
                if trigger != RunTrigger::Schedule || repo.last_error.as_deref() != Some(message.as_str()) {
                    p.runs.record(id, trigger, started, now, &RunOutcome::error(&message), &[]).await?;
                }
                let record = CheckRecord {
                    status: RepoStatus::Error,
                    error: Some(message),
                    seen: None,
                    checked_at: now,
                    next_check_at: next,
                };
                p.repositories.record_check(id, &record).await?;
            }
        }
        Ok(())
    }

    async fn run_check(&self, repo: &Repository, settings: &Settings, force: bool) -> Result<Checked> {
        let p = &self.ports;
        self.progress(repo.id, &repo.slug, "Querying GitHub");
        let remote = p.forge.repository(&repo.slug).await?;
        if remote.read_only {
            bail!("repository is archived: disable it or remove it");
        }
        if remote.clone_url != repo.clone_url || remote.default_branch != repo.default_branch {
            p.repositories.update_origin(repo.id, &remote.clone_url, &remote.default_branch).await?;
        }
        if let Err(e) = self.refresh_pull_request(repo).await {
            tracing::warn!(repo = %repo.slug, error = %format!("{e:#}"), "could not refresh PR state");
        }

        let ws = self.workspace(&repo.slug).await;
        self.sync_workspace(repo, ws.as_ref(), &remote.clone_url, &remote.default_branch).await?;
        let head = ws.remote_head(&remote.default_branch).await?;
        let mut outcome = RunOutcome { head: Some(head.clone()), ..RunOutcome::new(RunStatus::NoChanges) };
        if !force && repo.last_seen.as_ref() == Some(&head) {
            return Ok(Checked { outcome, files: Vec::new() });
        }

        self.progress(repo.id, &repo.slug, "Scanning commit history");
        let (candidates, scanned) = self.find_candidates(repo, settings, ws.as_ref(), &head).await?;
        outcome.files_scanned = scanned;
        outcome.status = RunStatus::UpToDate;
        if candidates.is_empty() {
            return Ok(Checked { outcome, files: Vec::new() });
        }

        self.progress(repo.id, &repo.slug, "Preparing branch");
        ws.checkout_new_branch(&settings.branch, &head).await?;
        let optimized = self.optimize_files(repo, settings, ws.as_ref(), candidates).await?;
        if optimized.is_empty() {
            return Ok(Checked { outcome, files: Vec::new() });
        }

        self.progress(repo.id, &repo.slug, "Committing");
        let changes: Vec<FileChange> = optimized
            .iter()
            .map(|o| FileChange {
                path: o.candidate.entry.path.clone(),
                optimizer: o.candidate.strategy.optimizer_id(),
                original_blob: o.candidate.entry.blob.clone(),
                savings: Savings::new(o.candidate.entry.size, o.after),
            })
            .collect();
        let (author, _) = self.resolve_author(settings).await?;
        let message = report::commit_message(&settings.commit_title, &changes);
        let paths: Vec<_> = changes.iter().map(|c| c.path.clone()).collect();
        let (commit, new_blobs) = ws.commit(&paths, &message, &author, settings.sign_commits).await?;
        for (o, change) in optimized.iter().zip(&changes) {
            let verdict = BlobVerdict::Optimized { blob: new_blobs.get(&change.path).cloned() };
            p.verdicts.put(&change.original_blob, &o.candidate.profile, &verdict, change.savings).await?;
        }

        outcome.commit = Some(commit.clone());
        outcome.files_optimized = changes.len() as u32;
        outcome.savings = changes.iter().map(|c| c.savings).sum();
        if repo.auto_pr {
            let pr = self.deliver(repo, &remote, ws.as_ref(), settings, &changes).await?;
            p.repositories.set_pending_commit(repo.id, None).await?;
            outcome.status = pr.status;
            outcome.pull_request = Some((pr.number, pr.url));
        } else {
            p.repositories.set_pending_commit(repo.id, Some(&commit)).await?;
            outcome.status = RunStatus::AwaitingApproval;
        }
        Ok(Checked { outcome, files: changes })
    }

    /// Clones on first use and fetches afterwards; re-clones only a broken checkout.
    async fn sync_workspace(
        &self,
        repo: &Repository,
        ws: &dyn Workspace,
        url: &CloneUrl,
        branch: &BranchName,
    ) -> Result<()> {
        if ws.exists() {
            self.progress(repo.id, &repo.slug, "Fetching");
            let fetched = async {
                ws.set_origin(url).await?;
                ws.fetch(branch).await
            }
            .await;
            match fetched {
                Ok(()) => return Ok(()),
                // A healthy clone means the problem is upstream (network, auth): keep it.
                Err(e) if ws.is_healthy().await => return Err(e),
                Err(e) => tracing::warn!(repo = %repo.slug, error = %format!("{e:#}"), "broken clone, re-cloning"),
            }
        }
        self.progress(repo.id, &repo.slug, "Cloning");
        ws.clone_from(url, branch).await
    }

    /// Files with an active strategy that are not ignored and were never
    /// optimized: not by a bot commit in history, not rejected in a closed PR,
    /// and not already found optimal with the current options.
    async fn find_candidates(
        &self,
        repo: &Repository,
        settings: &Settings,
        ws: &dyn Workspace,
        head: &CommitSha,
    ) -> Result<(Vec<Candidate>, u32)> {
        let p = &self.ports;
        let active = self.catalog.active(settings).await;
        if active.is_empty() {
            bail!("no optimizer is enabled and available");
        }
        let already_optimized = ws.blobs_written_by_marked_commits(head, BOT_MARKER).await?;
        let rejected = p.rejections.rejected(repo.id).await?;
        let ignore = IgnoreMatcher::compile(&[&settings.ignore_rules, &repo.ignore_rules])?;
        let max_size = settings.max_file_size();
        let options = settings.codec_options();

        let mut scanned = 0;
        let mut candidates = Vec::new();
        for entry in ws.tree(head).await? {
            let Some(format) = FileFormat::from_path(&entry.path) else { continue };
            let Some(strategy) = active.iter().find(|s| s.handles(format)) else { continue };
            if ignore.is_ignored(&entry.path) {
                continue;
            }
            scanned += 1;
            if entry.size.get() == 0
                || entry.size > max_size
                || already_optimized.contains(&entry.blob)
                || rejected.contains(&entry.blob)
            {
                continue;
            }
            let profile = strategy.profile(&options);
            let verdict = p.verdicts.get(&entry.blob, &profile).await?;
            if verdict.as_ref().is_some_and(BlobVerdict::skips_file) {
                continue;
            }
            let cached_output = match verdict {
                Some(BlobVerdict::Optimized { blob }) => blob,
                _ => None,
            };
            candidates.push(Candidate { entry, strategy: strategy.clone(), profile, cached_output });
        }
        Ok((candidates, scanned))
    }

    async fn optimize_files(
        &self,
        repo: &Repository,
        settings: &Settings,
        ws: &dyn Workspace,
        mut candidates: Vec<Candidate>,
    ) -> Result<Vec<Optimized>> {
        let paths: Vec<_> = candidates.iter().map(|c| c.entry.path.clone()).collect();
        let lfs = ws.lfs_paths(&paths).await?;
        candidates.retain(|c| !lfs.contains(&c.entry.path));
        let options = settings.codec_options();

        let total = candidates.len();
        let mut optimized = Vec::new();
        for (i, c) in candidates.into_iter().enumerate() {
            self.progress(repo.id, &repo.slug, format!("Optimizing {}/{}: {}", i + 1, total, c.entry.path));

            // Same input bytes as an earlier run: reuse that verified output.
            if let Some(blob) = &c.cached_output
                && ws.has_blob(blob).await
            {
                let data = ws.read_blob(blob).await?;
                ws.write_file(&c.entry.path, &data).await?;
                optimized.push(Optimized { after: ByteSize::new(data.len() as u64), candidate: c });
                continue;
            }

            let data = ws.read_file(&c.entry.path).await?;
            if data.starts_with(b"version https://git-lfs") {
                continue;
            }
            let before = ByteSize::new(data.len() as u64);
            let unchanged = Savings::new(before, before);
            match attempt(c.strategy.as_ref(), Arc::new(data), &options).await {
                Outcome::Optimized(out) => {
                    ws.write_file(&c.entry.path, &out).await?;
                    optimized.push(Optimized { after: ByteSize::new(out.len() as u64), candidate: c });
                }
                Outcome::Optimal(detail) => {
                    self.ports
                        .verdicts
                        .put(&c.entry.blob, &c.profile, &BlobVerdict::Optimal { detail }, unchanged)
                        .await?;
                }
                Outcome::Failed(reason) => {
                    tracing::debug!(repo = %repo.slug, path = %c.entry.path, error = %reason, "not optimized");
                    self.ports
                        .verdicts
                        .put(&c.entry.blob, &c.profile, &BlobVerdict::Failed { reason }, unchanged)
                        .await?;
                }
            }
        }
        Ok(optimized)
    }

    /// Updates the stored state of the last open PR; learns from merges
    /// (credits the savings) and rejections (never proposes those files again).
    async fn refresh_pull_request(&self, repo: &Repository) -> Result<()> {
        let p = &self.ports;
        let Some(pr) = repo.open_pull_request() else { return Ok(()) };
        let remote = p.forge.pull_request(&repo.slug, pr.number).await?;
        if remote.state == PrState::Open {
            return Ok(());
        }
        if let Some(run) = p.runs.latest_for_pull_request(repo.id, pr.number).await? {
            match remote.state {
                PrState::Merged => p.repositories.add_bytes_saved(repo.id, run.outcome.savings.saved()).await?,
                _ => p.rejections.reject_run(repo.id, run.id).await?,
            }
        }
        let updated = PullRequest { number: pr.number, url: remote.url, state: remote.state };
        p.repositories.set_pull_request(repo.id, &updated).await
    }

    /// Pushes the bot branch with the repository's delivery strategy and
    /// opens or updates the pull request, always as the developer.
    pub(crate) async fn deliver(
        &self,
        repo: &Repository,
        remote: &ForgeRepository,
        ws: &dyn Workspace,
        settings: &Settings,
        changes: &[FileChange],
    ) -> Result<Delivered> {
        let p = &self.ports;
        let strategy = delivery::for_mode(repo.mode);
        if let Some(phase) = strategy.preparing_phase() {
            self.progress(repo.id, &repo.slug, phase);
        }
        let destination = strategy.destination(self, repo, remote, &settings.branch).await?;

        self.progress(repo.id, &repo.slug, "Pushing branch");
        let mut attempt = 0;
        loop {
            match ws.push(&destination.push_url, &settings.branch).await {
                Ok(()) => break,
                Err(e) if strategy.retries_push() && attempt < 3 => {
                    attempt += 1;
                    tracing::debug!(error = %e, attempt, "push failed, retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
                }
                Err(e) => {
                    return Err(match strategy.push_hint() {
                        Some(hint) => e.context(hint),
                        None => e,
                    });
                }
            }
        }

        self.progress(repo.id, &repo.slug, "Opening pull request");
        let title = &settings.commit_title;
        let body = report::pull_request_body(changes, |id| self.catalog.explanation(id));
        let (status, pr) = match p.forge.find_open_pull_request(&repo.slug, &destination.head).await? {
            Some(existing) => {
                (RunStatus::PrUpdated, p.forge.update_pull_request(&repo.slug, existing.number, title, &body).await?)
            }
            None => (
                RunStatus::PrOpened,
                p.forge
                    .open_pull_request(&repo.slug, &destination.head, &remote.default_branch, title, &body)
                    .await
                    .context("opening the pull request")?,
            ),
        };
        let stored = PullRequest { number: pr.number, url: pr.url.clone(), state: PrState::Open };
        p.repositories.set_pull_request(repo.id, &stored).await?;
        Ok(Delivered { status, number: pr.number, url: pr.url })
    }
}
