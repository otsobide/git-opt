//! Manual approval: publish the commit a previous check prepared.

use anyhow::{Context, Result, bail};
use git_opt_domain::{
    model::{FileChange, RepoStatus, Repository, RunOutcome, RunTrigger},
    values::RepoId,
};

use crate::Services;

impl Services {
    pub async fn publish_pending(&self, id: RepoId) -> Result<()> {
        let p = &self.ports;
        let Some(repo) = p.repositories.get(id).await? else { return Ok(()) };
        let started = p.clock.now();
        match self.run_publish(&repo).await {
            Ok((outcome, files)) => {
                p.runs.record(id, RunTrigger::Publish, started, p.clock.now(), &outcome, &files).await?;
                p.repositories.set_pending_commit(id, None).await?;
                p.repositories.set_status(id, RepoStatus::Ok, None).await?;
            }
            Err(e) => {
                let message = format!("{e:#}");
                p.runs
                    .record(id, RunTrigger::Publish, started, p.clock.now(), &RunOutcome::error(&message), &[])
                    .await?;
                p.repositories.set_status(id, repo.status, Some(&message)).await?;
            }
        }
        Ok(())
    }

    async fn run_publish(&self, repo: &Repository) -> Result<(RunOutcome, Vec<FileChange>)> {
        let p = &self.ports;
        let settings = p.settings.load().await?;
        let Some(commit) = repo.pending_commit.clone() else { bail!("nothing is awaiting approval") };
        let ws = self.workspace(&repo.slug).await;
        if ws.local_head(&settings.branch).await.as_ref() != Some(&commit) {
            bail!("the prepared branch changed; run a new check");
        }
        let source = p.runs.by_commit(repo.id, &commit).await?.context("run for the prepared commit not found")?;
        let files: Vec<FileChange> = p.runs.files(source.id).await?.into_iter().map(|f| f.change).collect();

        let remote = p.forge.repository(&repo.slug).await?;
        let pr = self.deliver(repo, &remote, ws.as_ref(), &settings, &files).await?;
        let outcome = RunOutcome {
            status: pr.status,
            commit: Some(commit),
            pull_request: Some((pr.number, pr.url)),
            message: None,
            ..source.outcome
        };
        Ok((outcome, files))
    }
}
