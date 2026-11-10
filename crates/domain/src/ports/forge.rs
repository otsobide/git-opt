use async_trait::async_trait;

use crate::{
    model::{PrState, SourceKind},
    values::{AccessToken, BranchName, CloneUrl, Identity, OwnerName, PrNumber, RepoSlug},
};

/// Where the developer's token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenOrigin {
    /// Saved from the web UI.
    Settings,
    /// An environment variable.
    Environment,
    /// The forge's CLI session (`gh auth token`).
    Cli,
}

impl TokenOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Environment => "env",
            Self::Cli => "gh",
        }
    }
}

/// The developer's account on the forge.
#[derive(Debug, Clone)]
pub struct ForgeAccount {
    pub login: OwnerName,
    pub display_name: Option<String>,
    /// Identity to commit with when nothing else is configured (for GitHub,
    /// the private `id+login@users.noreply.github.com` address).
    pub commit_identity: Identity,
}

#[derive(Debug, Clone)]
pub struct ForgeRepository {
    pub slug: RepoSlug,
    pub clone_url: CloneUrl,
    pub default_branch: BranchName,
    pub is_fork: bool,
    /// Archived or disabled: it cannot receive pull requests.
    pub read_only: bool,
    /// The developer's token can push to it.
    pub can_push: bool,
}

#[derive(Debug, Clone)]
pub struct ForgePullRequest {
    pub number: PrNumber,
    pub url: String,
    pub state: PrState,
}

/// The branch a pull request comes from.
#[derive(Debug, Clone)]
pub struct PullRequestHead {
    pub owner: OwnerName,
    pub branch: BranchName,
    /// From a fork rather than from the target repository itself.
    pub cross_repository: bool,
}

/// A code hosting service (GitHub today). Every call acts as the developer.
#[async_trait]
pub trait CodeForge: Send + Sync {
    /// The developer's token and where it was found, if any.
    async fn credentials(&self) -> Option<(AccessToken, TokenOrigin)>;
    /// Drops cached credentials and account data (after the token changes).
    async fn forget_credentials(&self);
    async fn account(&self) -> anyhow::Result<ForgeAccount>;
    async fn repository(&self, slug: &RepoSlug) -> anyhow::Result<ForgeRepository>;
    /// `owner` is `None` for [`SourceKind::Me`].
    async fn list_repositories(
        &self,
        kind: SourceKind,
        owner: Option<&OwnerName>,
    ) -> anyhow::Result<Vec<ForgeRepository>>;
    /// Creates (or returns) the developer's fork, once it is ready to use.
    async fn fork(&self, slug: &RepoSlug) -> anyhow::Result<ForgeRepository>;
    async fn pull_request(&self, slug: &RepoSlug, number: PrNumber) -> anyhow::Result<ForgePullRequest>;
    async fn find_open_pull_request(
        &self,
        slug: &RepoSlug,
        head: &PullRequestHead,
    ) -> anyhow::Result<Option<ForgePullRequest>>;
    async fn open_pull_request(
        &self,
        slug: &RepoSlug,
        head: &PullRequestHead,
        base: &BranchName,
        title: &str,
        body: &str,
    ) -> anyhow::Result<ForgePullRequest>;
    async fn update_pull_request(
        &self,
        slug: &RepoSlug,
        number: PrNumber,
        title: &str,
        body: &str,
    ) -> anyhow::Result<ForgePullRequest>;
}
