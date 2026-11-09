//! Watch, change, check and forget repositories.

use git_opt_domain::{
    model::{NewRepository, RepoMode, Repository, RepositoryChanges, RunTrigger},
    values::{RepoId, RepoSlug, SourceId},
};

use crate::{AppError, AppResult, Services, jobs::Job};

/// Requested delivery mode when adding a repository.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModeRequest {
    /// Direct if the developer can push, fork otherwise.
    #[default]
    Auto,
    Direct,
    Fork,
}

/// A repository with what the job queue knows about it right now.
#[derive(Debug, Clone)]
pub struct RepositoryView {
    pub repository: Repository,
    pub running: bool,
    pub queued: bool,
    pub phase: Option<String>,
}

impl Services {
    fn view(&self, repository: Repository) -> RepositoryView {
        let (running, queued, phase) = self.jobs.repo_state(repository.id);
        RepositoryView { repository, running, queued, phase }
    }

    pub async fn list_repositories(&self) -> AppResult<Vec<RepositoryView>> {
        Ok(self.ports.repositories.list().await?.into_iter().map(|r| self.view(r)).collect())
    }

    pub async fn repository(&self, id: RepoId) -> AppResult<RepositoryView> {
        let repo = self.ports.repositories.get(id).await?.ok_or(AppError::NotFound)?;
        Ok(self.view(repo))
    }

    /// Starts watching `reference` (`owner/name` or a URL) and queues a first check.
    pub async fn add_repository(&self, reference: &str, mode: ModeRequest, auto_pr: bool) -> AppResult<RepositoryView> {
        let slug = RepoSlug::parse(reference)?;
        let (id, created) =
            self.watch(&slug, mode, auto_pr, None).await.map_err(|e| AppError::Invalid(format!("{e:#}")))?;
        if !created {
            return Err(AppError::Conflict("that repository is already being watched".into()));
        }
        self.jobs.enqueue(Job::Check { repo: id, trigger: RunTrigger::Manual, force: false });
        self.repository(id).await
    }

    /// Inserts a repository after checking it on the forge and choosing its mode.
    pub(crate) async fn watch(
        &self,
        slug: &RepoSlug,
        mode: ModeRequest,
        auto_pr: bool,
        source: Option<SourceId>,
    ) -> anyhow::Result<(RepoId, bool)> {
        let p = &self.ports;
        let remote = p.forge.repository(slug).await?;
        if remote.read_only {
            anyhow::bail!("{} is archived: it cannot receive pull requests", remote.slug);
        }
        let me = p.forge.account().await?;
        let own = remote.slug.owner().same_as(me.login.as_str());
        let mode = match mode {
            ModeRequest::Auto if remote.can_push => RepoMode::Direct,
            ModeRequest::Auto => RepoMode::Fork,
            ModeRequest::Direct if !remote.can_push => {
                anyhow::bail!("no push access to {}: add it in fork (contribution) mode", remote.slug)
            }
            ModeRequest::Direct => RepoMode::Direct,
            ModeRequest::Fork if own => anyhow::bail!("{} is yours: GitHub does not let you fork your own repository", remote.slug),
            ModeRequest::Fork => RepoMode::Fork,
        };
        let new = NewRepository {
            slug: remote.slug,
            source_id: source,
            mode,
            clone_url: remote.clone_url,
            default_branch: remote.default_branch,
            auto_pr,
        };
        p.repositories.insert(&new, p.clock.now()).await
    }

    /// Applies user changes; reschedules when the interval or the pause changes.
    pub async fn update_repository(&self, id: RepoId, changes: RepositoryChanges) -> AppResult<RepositoryView> {
        let p = &self.ports;
        let before = p.repositories.get(id).await?.ok_or(AppError::NotFound)?;
        p.repositories.update(id, &changes).await?;
        let after = p.repositories.get(id).await?.ok_or(AppError::NotFound)?;
        let now = p.clock.now();
        if changes.poll_interval_minutes.is_some() {
            let settings = p.settings.load().await?;
            let next = after.poll_policy(&settings).next_check(now, |max| p.random.up_to(max));
            p.repositories.schedule(id, next).await?;
        }
        if after.enabled && !before.enabled {
            p.repositories.schedule(id, now).await?;
        }
        self.repository(id).await
    }

    /// Stops watching and deletes the working copy. GitHub is not touched.
    pub async fn remove_repository(&self, id: RepoId) -> AppResult<()> {
        if self.jobs.repo_state(id).0 {
            return Err(AppError::Conflict("the repository is being processed; try again in a moment".into()));
        }
        let repo = self.ports.repositories.delete(id).await?.ok_or(AppError::NotFound)?;
        if let Err(e) = self.ports.vcs.remove_workspace(&repo.slug).await {
            tracing::warn!(repo = %repo.slug, error = %e, "could not remove the working copy");
        }
        Ok(())
    }

    pub async fn request_check(&self, id: RepoId, force: bool) -> AppResult<()> {
        self.ports.repositories.get(id).await?.ok_or(AppError::NotFound)?;
        self.jobs.enqueue(Job::Check { repo: id, trigger: RunTrigger::Manual, force });
        Ok(())
    }

    pub async fn request_check_all(&self) -> AppResult<()> {
        for repo in self.ports.repositories.list().await?.into_iter().filter(|r| r.enabled) {
            self.jobs.enqueue(Job::Check { repo: repo.id, trigger: RunTrigger::Manual, force: false });
        }
        Ok(())
    }

    pub async fn request_publish(&self, id: RepoId) -> AppResult<()> {
        let repo = self.ports.repositories.get(id).await?.ok_or(AppError::NotFound)?;
        if repo.pending_commit.is_none() {
            return Err(AppError::Conflict("there is no optimization awaiting approval".into()));
        }
        self.jobs.enqueue(Job::Publish { repo: id });
        Ok(())
    }

    /// Lets files from PRs closed without merging be proposed again.
    pub async fn forget_rejections(&self, id: RepoId) -> AppResult<()> {
        self.ports.repositories.get(id).await?.ok_or(AppError::NotFound)?;
        self.ports.rejections.forget(id).await?;
        Ok(())
    }
}
