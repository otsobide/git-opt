//! Delete the developer's fork of a third-party repository once every pull
//! request from it is merged (the rules are in [`fork_cleanup`]).

use anyhow::Result;
use git_opt_domain::{
    fork_cleanup::{self, BranchVerdict, Keep},
    model::{Fork, PrState, RepoMode, Repository, RunOutcome, RunStatus, RunTrigger, Settings},
    ports::PullRequestHead,
};

use crate::Services;

impl Services {
    /// After a successful check: deletes the fork if its time has come. A
    /// failure (say, a token without the delete_repo scope) is recorded once
    /// as an error run, without failing the check.
    pub(crate) async fn tidy_fork(&self, repo: &Repository, settings: &Settings, trigger: RunTrigger) -> Result<()> {
        let Err(e) = self.delete_fork_if_done(repo, settings, trigger).await else { return Ok(()) };
        let p = &self.ports;
        let message = format!("could not delete the fork: {e:#}");
        tracing::warn!(repo = %repo.slug, error = %message, "fork cleanup failed");
        let latest = p.runs.list(Some(repo.id), 1).await?;
        if latest.first().and_then(|r| r.outcome.message.as_deref()) != Some(message.as_str()) {
            let now = p.clock.now();
            p.runs.record(repo.id, trigger, now, now, &RunOutcome::error(&message), &[]).await?;
        }
        Ok(())
    }

    /// Deletes the repository's fork if the setting is on, the repository is
    /// in fork mode, the bot's last pull request was merged, and nothing else
    /// needs the fork. Direct-mode repositories are never touched.
    async fn delete_fork_if_done(&self, repo: &Repository, settings: &Settings, trigger: RunTrigger) -> Result<()> {
        let Some(fork) = &repo.fork else { return Ok(()) };
        let merged = repo.pull_request.as_ref().is_some_and(|pr| pr.state == PrState::Merged);
        if !settings.delete_merged_forks || repo.mode != RepoMode::Fork || !merged || repo.pending_commit.is_some() {
            return Ok(());
        }
        let p = &self.ports;
        let started = p.clock.now();
        if let Some(reason) = self.reason_to_keep(repo, fork).await? {
            tracing::debug!(repo = %repo.slug, fork = %fork.slug, reason, "keeping the fork");
            return Ok(());
        }
        p.forge.delete_repository(&fork.slug).await?;
        p.repositories.set_fork(repo.id, None).await?;
        let message = format!("Deleted the fork {}: every pull request from it is merged.", fork.slug);
        tracing::info!(repo = %repo.slug, fork = %fork.slug, "fork deleted");
        let outcome = RunOutcome { message: Some(message), ..RunOutcome::new(RunStatus::ForkDeleted) };
        p.runs.record(repo.id, trigger, started, p.clock.now(), &outcome, &[]).await?;
        Ok(())
    }

    /// Why the fork must stay, if anything says so.
    async fn reason_to_keep(&self, repo: &Repository, fork: &Fork) -> Result<Option<Keep>> {
        let forge = &self.ports.forge;
        let me = forge.account().await?.login;
        let remote = match forge.repository(&fork.slug).await {
            Ok(remote) => remote,
            // Already deleted by hand, renamed, or GitHub is unreachable: try again next time.
            Err(e) => return Ok(Some(format!("cannot look at {}: {e:#}", fork.slug))),
        };
        if let Err(reason) = fork_cleanup::check_ownership(&remote, &repo.slug, &me) {
            return Ok(Some(reason));
        }
        for branch in forge.branches(&remote.slug).await? {
            let head = PullRequestHead {
                owner: remote.slug.owner().clone(),
                branch: branch.name.clone(),
                cross_repository: true,
            };
            let pulls = forge.pull_requests_from(&repo.slug, &head).await?;
            let kept = match fork_cleanup::judge_branch(&branch, &pulls) {
                BranchVerdict::Delivered => continue,
                BranchVerdict::Keep(reason) => Err(reason),
                BranchVerdict::NeedsComparison => {
                    let ahead = forge.commits_ahead(&repo.slug, &repo.default_branch, &head).await?;
                    fork_cleanup::compared(&branch, ahead)
                }
            };
            if let Err(reason) = kept {
                return Ok(Some(reason));
            }
        }
        Ok(None)
    }
}
