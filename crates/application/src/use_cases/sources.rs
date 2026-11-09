//! Sources: users, organizations or "everything I can access", imported and
//! kept in sync.

use git_opt_domain::{
    model::{NewRepository, NewSource, RepoMode, Source, SourceChanges, SourceKind},
    values::{OwnerName, SourceId},
};

use crate::{
    AppError, AppResult, Services,
    jobs::{Job, JobKey},
};

impl Services {
    pub async fn list_sources(&self) -> AppResult<Vec<Source>> {
        Ok(self.ports.sources.list().await?)
    }

    pub async fn source(&self, id: SourceId) -> AppResult<Source> {
        self.ports.sources.get(id).await?.ok_or(AppError::NotFound)
    }

    /// Adds a source and queues its first sync.
    pub async fn add_source(&self, mut source: NewSource) -> AppResult<Source> {
        let p = &self.ports;
        source.owner = match source.kind {
            SourceKind::Me => None,
            _ => Some(source.owner.ok_or_else(|| AppError::Invalid("a user or organization name is required".into()))?),
        };
        let duplicate = p.sources.list().await?.iter().any(|s| {
            s.kind == source.kind && s.owner.as_ref().map(OwnerName::as_str).map(str::to_ascii_lowercase)
                == source.owner.as_ref().map(OwnerName::as_str).map(str::to_ascii_lowercase)
        });
        if duplicate {
            return Err(AppError::Conflict("that source already exists".into()));
        }
        let id = p.sources.insert(&source, p.clock.now()).await?;
        self.jobs.enqueue(Job::SyncSource { source: id });
        self.source(id).await
    }

    pub async fn update_source(&self, id: SourceId, changes: SourceChanges) -> AppResult<Source> {
        self.source(id).await?;
        self.ports.sources.update(id, &changes).await?;
        self.source(id).await
    }

    /// Removes a source; with `with_repositories`, also its repositories.
    pub async fn remove_source(&self, id: SourceId, with_repositories: bool) -> AppResult<()> {
        if with_repositories && self.jobs.is_queued_or_running(JobKey::Source(id)) {
            return Err(AppError::Conflict("the source is syncing; try again in a moment".into()));
        }
        self.source(id).await?;
        for repo in self.ports.sources.delete(id, with_repositories).await? {
            if let Err(e) = self.ports.vcs.remove_workspace(&repo.slug).await {
                tracing::warn!(repo = %repo.slug, error = %e, "could not remove the working copy");
            }
        }
        Ok(())
    }

    pub async fn request_sync(&self, id: SourceId) -> AppResult<()> {
        self.source(id).await?;
        self.jobs.enqueue(Job::SyncSource { source: id });
        Ok(())
    }

    /// Lists the source on the forge and watches every new repository.
    pub async fn sync_source(&self, id: SourceId) -> anyhow::Result<()> {
        let p = &self.ports;
        let Some(source) = p.sources.get(id).await? else { return Ok(()) };
        let listed = match p.forge.list_repositories(source.kind, source.owner.as_ref()).await {
            Ok(list) => list,
            Err(e) => {
                let message = format!("{e:#}");
                p.sources.mark_synced(id, p.clock.now(), None, Some(&message)).await?;
                anyhow::bail!(message);
            }
        };
        let me = p.forge.account().await?;
        let mut count = 0;
        for remote in listed {
            if (remote.read_only && !source.include_archived) || (remote.is_fork && !source.include_forks) {
                continue;
            }
            count += 1;
            let own = remote.slug.owner().same_as(me.login.as_str());
            let mode = if remote.can_push || own { RepoMode::Direct } else { RepoMode::Fork };
            let new = NewRepository {
                slug: remote.slug,
                source_id: Some(id),
                mode,
                clone_url: remote.clone_url,
                default_branch: remote.default_branch,
                auto_pr: true,
            };
            p.repositories.insert(&new, p.clock.now()).await?;
        }
        p.sources.mark_synced(id, p.clock.now(), Some(count), None).await?;
        tracing::info!(source = %id, count, "source synced");
        Ok(())
    }
}
