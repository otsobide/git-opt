use async_trait::async_trait;

use crate::{
    model::{PrState, SourceKind},
    values::{AccessToken, BranchName, CloneUrl, CommitSha, Identity, OwnerName, PrNumber, RepoSlug},
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
    /// For a fork, the repository it was forked from (when the forge says so).
    pub parent: Option<RepoSlug>,
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

/// A branch and the commit it points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeBranch {
    pub name: BranchName,
    pub head: CommitSha,
}

/// A pull request, as seen from the branch it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPullRequest {
    pub number: PrNumber,
    pub state: PrState,
    /// Its last commit (it stops following the branch once merged or closed).
    pub head: CommitSha,
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
    /// Every branch of a repository, with its current commit.
    async fn branches(&self, slug: &RepoSlug) -> anyhow::Result<Vec<ForgeBranch>>;
    /// Every pull request into `slug` from `head`, in any state.
    async fn pull_requests_from(
        &self,
        slug: &RepoSlug,
        head: &PullRequestHead,
    ) -> anyhow::Result<Vec<BranchPullRequest>>;
    /// How many commits of `head` (a branch in `slug`'s fork network) are not in `base`.
    async fn commits_ahead(&self, slug: &RepoSlug, base: &BranchName, head: &PullRequestHead) -> anyhow::Result<u64>;
    /// Deletes a repository. Only ever used for the developer's own forks.
    async fn delete_repository(&self, slug: &RepoSlug) -> anyhow::Result<()>;
}
