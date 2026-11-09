use std::collections::{HashMap, HashSet};

use async_trait::async_trait;

use crate::values::{AccessToken, BlobId, BranchName, ByteSize, CloneUrl, CommitSha, Identity, RepoPath, RepoSlug};

/// One regular file of a commit's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: RepoPath,
    pub blob: BlobId,
    pub size: ByteSize,
}

/// A version control system (git today).
#[async_trait]
pub trait VersionControl: Send + Sync {
    /// The bot's private working copy of a repository. `token` authenticates
    /// HTTPS remotes; it must never end up in arguments, URLs or logs.
    fn workspace(&self, slug: &RepoSlug, token: Option<AccessToken>) -> Box<dyn Workspace>;
    /// Deletes a working copy (it is re-created on the next check).
    async fn remove_workspace(&self, slug: &RepoSlug) -> anyhow::Result<()>;
    /// Tool name and version, for the dashboard.
    async fn version(&self) -> anyhow::Result<String>;
    /// The developer's configured identity (`user.name`/`user.email`).
    async fn configured_identity(&self) -> Option<Identity>;
}

/// A working copy owned by the bot: it may be reset at will.
#[async_trait]
pub trait Workspace: Send + Sync {
    fn exists(&self) -> bool;
    /// Clones only `branch`, replacing anything already there.
    async fn clone_from(&self, url: &CloneUrl, branch: &BranchName) -> anyhow::Result<()>;
    async fn set_origin(&self, url: &CloneUrl) -> anyhow::Result<()>;
    async fn fetch(&self, branch: &BranchName) -> anyhow::Result<()>;
    /// Whether the working copy is usable (as opposed to broken).
    async fn is_healthy(&self) -> bool;
    /// Tip of the fetched remote branch.
    async fn remote_head(&self, branch: &BranchName) -> anyhow::Result<CommitSha>;
    /// Tip of a local branch, if it exists.
    async fn local_head(&self, branch: &BranchName) -> Option<CommitSha>;
    /// Regular files (no symlinks or submodules) with their sizes.
    async fn tree(&self, commit: &CommitSha) -> anyhow::Result<Vec<TreeEntry>>;
    /// Blobs written by any commit reachable from `commit` whose message
    /// contains `marker`.
    async fn blobs_written_by_marked_commits(&self, commit: &CommitSha, marker: &str) -> anyhow::Result<HashSet<BlobId>>;
    /// Points a fresh local `branch` at `start` and resets the working tree.
    async fn checkout_new_branch(&self, branch: &BranchName, start: &CommitSha) -> anyhow::Result<()>;
    /// Paths stored with Git LFS (never touched).
    async fn lfs_paths(&self, paths: &[RepoPath]) -> anyhow::Result<HashSet<RepoPath>>;
    async fn has_blob(&self, blob: &BlobId) -> bool;
    async fn read_blob(&self, blob: &BlobId) -> anyhow::Result<Vec<u8>>;
    async fn read_file(&self, path: &RepoPath) -> anyhow::Result<Vec<u8>>;
    async fn write_file(&self, path: &RepoPath, bytes: &[u8]) -> anyhow::Result<()>;
    /// Commits `paths` (hooks disabled). Returns the commit and the new blob
    /// of every changed path.
    async fn commit(
        &self,
        paths: &[RepoPath],
        message: &str,
        author: &Identity,
        sign: bool,
    ) -> anyhow::Result<(CommitSha, HashMap<RepoPath, BlobId>)>;
    /// Force-pushes the current branch to `branch` on `url`.
    async fn push(&self, url: &CloneUrl, branch: &BranchName) -> anyhow::Result<()>;
}
