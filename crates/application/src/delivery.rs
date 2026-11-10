//! How the bot's branch reaches a repository: one strategy per [`RepoMode`].

use anyhow::Result;
use async_trait::async_trait;
use git_opt_domain::{
    model::{Fork, RepoMode, Repository},
    ports::{ForgeRepository, PullRequestHead},
    values::{BranchName, CloneUrl},
};

use crate::Services;

/// Where to push and which head the pull request comes from.
#[derive(Debug, Clone)]
pub struct Destination {
    pub push_url: CloneUrl,
    pub head: PullRequestHead,
}

#[async_trait]
pub trait DeliveryStrategy: Send + Sync {
    /// Short phase shown while preparing the destination, if any.
    fn preparing_phase(&self) -> Option<&'static str>;
    async fn destination(
        &self,
        services: &Services,
        repo: &Repository,
        remote: &ForgeRepository,
        branch: &BranchName,
    ) -> Result<Destination>;
    /// A push may need a few retries (a brand-new fork warming up).
    fn retries_push(&self) -> bool;
    /// Added to a push error to suggest a fix.
    fn push_hint(&self) -> Option<&'static str>;
}

/// The developer can push: the branch goes to the repository itself.
pub struct Direct;

#[async_trait]
impl DeliveryStrategy for Direct {
    fn preparing_phase(&self) -> Option<&'static str> {
        None
    }
    async fn destination(
        &self,
        _: &Services,
        _: &Repository,
        remote: &ForgeRepository,
        branch: &BranchName,
    ) -> Result<Destination> {
        Ok(Destination {
            push_url: remote.clone_url.clone(),
            head: PullRequestHead {
                owner: remote.slug.owner().clone(),
                branch: branch.clone(),
                cross_repository: false,
            },
        })
    }
    fn retries_push(&self) -> bool {
        false
    }
    fn push_hint(&self) -> Option<&'static str> {
        Some("push rejected: if you lack write access, switch the repo to fork mode")
    }
}

/// Third-party repository: the branch goes to the developer's fork.
pub struct ViaFork;

#[async_trait]
impl DeliveryStrategy for ViaFork {
    fn preparing_phase(&self) -> Option<&'static str> {
        Some("Preparing fork")
    }
    async fn destination(
        &self,
        services: &Services,
        repo: &Repository,
        _: &ForgeRepository,
        branch: &BranchName,
    ) -> Result<Destination> {
        let fork = ensure_fork(services, repo).await?;
        Ok(Destination {
            push_url: fork.clone_url,
            head: PullRequestHead { owner: fork.slug.owner().clone(), branch: branch.clone(), cross_repository: true },
        })
    }
    fn retries_push(&self) -> bool {
        true
    }
    fn push_hint(&self) -> Option<&'static str> {
        None
    }
}

/// The known fork if it still exists, otherwise a new (or found) one.
async fn ensure_fork(services: &Services, repo: &Repository) -> Result<Fork> {
    let forge = &services.ports.forge;
    if let Some(fork) = &repo.fork
        && forge.repository(&fork.slug).await.is_ok()
    {
        return Ok(fork.clone());
    }
    let remote = forge.fork(&repo.slug).await?;
    let fork = Fork { slug: remote.slug, clone_url: remote.clone_url };
    services.ports.repositories.set_fork(repo.id, Some(&fork)).await?;
    Ok(fork)
}

pub fn for_mode(mode: RepoMode) -> &'static dyn DeliveryStrategy {
    match mode {
        RepoMode::Direct => &Direct,
        RepoMode::Fork => &ViaFork,
    }
}
