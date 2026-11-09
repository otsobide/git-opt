//! Persistence ports. They are shaped by the use cases: targeted updates
//! rather than "save the whole aggregate", so a check running in the
//! background never overwrites what the user changed meanwhile.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{
    model::{
        BlobVerdict, FileChange, Fork, NewRepository, NewSource, PullRequest, RepoStatus, Repository, RepositoryChanges,
        Run, RunOutcome, RunTrigger, Settings, Source, SourceChanges,
    },
    values::{
        AccessToken, BlobId, BranchName, ByteSize, CloneUrl, CommitSha, PrNumber, Profile, RepoId, RepoSlug, RunId,
        Savings, SourceId,
    },
};

type Result<T> = anyhow::Result<T>;

#[async_trait]
pub trait SettingsStore: Send + Sync {
    /// Stored settings, or the defaults.
    async fn load(&self) -> Result<Settings>;
    async fn save(&self, settings: &Settings) -> Result<()>;
}

/// The forge token saved from the UI (write-only from the outside).
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn token(&self) -> Result<Option<AccessToken>>;
    async fn set_token(&self, token: Option<&AccessToken>) -> Result<()>;
}

#[async_trait]
pub trait SourceStore: Send + Sync {
    async fn list(&self) -> Result<Vec<Source>>;
    async fn get(&self, id: SourceId) -> Result<Option<Source>>;
    async fn insert(&self, source: &NewSource, now: DateTime<Utc>) -> Result<SourceId>;
    async fn update(&self, id: SourceId, changes: &SourceChanges) -> Result<()>;
    /// Removes the source; with `with_repositories` also its repositories,
    /// which are returned so their working copies can be removed.
    async fn delete(&self, id: SourceId, with_repositories: bool) -> Result<Vec<Repository>>;
    async fn mark_synced(&self, id: SourceId, at: DateTime<Utc>, repo_count: Option<u32>, error: Option<&str>) -> Result<()>;
    /// Sources with auto-sync whose last sync is older than `before`.
    async fn due_for_sync(&self, before: DateTime<Utc>) -> Result<Vec<SourceId>>;
}

/// What a finished check records on its repository.
#[derive(Debug, Clone)]
pub struct CheckRecord {
    pub status: RepoStatus,
    pub error: Option<String>,
    /// Only set on success: a failed check is retried from the same commit.
    pub seen: Option<CommitSha>,
    pub checked_at: DateTime<Utc>,
    pub next_check_at: DateTime<Utc>,
}

/// Numbers for the dashboard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Totals {
    pub repos: u64,
    pub repos_enabled: u64,
    pub prs_open: u64,
    pub awaiting_approval: u64,
    pub errors: u64,
    pub bytes_saved: ByteSize,
    pub prs_sent: u64,
}

#[async_trait]
pub trait RepositoryStore: Send + Sync {
    async fn list(&self) -> Result<Vec<Repository>>;
    async fn get(&self, id: RepoId) -> Result<Option<Repository>>;
    /// Case-insensitive, like the forge.
    async fn find(&self, slug: &RepoSlug) -> Result<Option<Repository>>;
    /// Inserts unless it exists. Returns the id and whether it was inserted.
    async fn insert(&self, repository: &NewRepository, now: DateTime<Utc>) -> Result<(RepoId, bool)>;
    async fn update(&self, id: RepoId, changes: &RepositoryChanges) -> Result<()>;
    async fn delete(&self, id: RepoId) -> Result<Option<Repository>>;
    async fn update_origin(&self, id: RepoId, clone_url: &CloneUrl, default_branch: &BranchName) -> Result<()>;
    async fn set_fork(&self, id: RepoId, fork: &Fork) -> Result<()>;
    /// Enabled repositories whose next check is due.
    async fn due(&self, now: DateTime<Utc>) -> Result<Vec<RepoId>>;
    async fn schedule(&self, id: RepoId, at: DateTime<Utc>) -> Result<()>;
    async fn record_check(&self, id: RepoId, record: &CheckRecord) -> Result<()>;
    async fn set_status(&self, id: RepoId, status: RepoStatus, error: Option<&str>) -> Result<()>;
    async fn set_pending_commit(&self, id: RepoId, commit: Option<&CommitSha>) -> Result<()>;
    async fn set_pull_request(&self, id: RepoId, pull_request: &PullRequest) -> Result<()>;
    async fn add_bytes_saved(&self, id: RepoId, bytes: ByteSize) -> Result<()>;
    async fn totals(&self) -> Result<Totals>;
}

/// A file change as stored, with its row id.
#[derive(Debug, Clone)]
pub struct StoredFileChange {
    pub id: i64,
    pub change: FileChange,
}

#[async_trait]
pub trait RunStore: Send + Sync {
    async fn record(
        &self,
        repo: RepoId,
        trigger: RunTrigger,
        started_at: DateTime<Utc>,
        finished_at: DateTime<Utc>,
        outcome: &RunOutcome,
        files: &[FileChange],
    ) -> Result<RunId>;
    /// Newest first.
    async fn list(&self, repo: Option<RepoId>, limit: u32) -> Result<Vec<Run>>;
    async fn get(&self, id: RunId) -> Result<Option<Run>>;
    /// Biggest saving first.
    async fn files(&self, id: RunId) -> Result<Vec<StoredFileChange>>;
    /// The latest run that changed files for a pull request.
    async fn latest_for_pull_request(&self, repo: RepoId, number: PrNumber) -> Result<Option<Run>>;
    /// The run that produced a local commit.
    async fn by_commit(&self, repo: RepoId, commit: &CommitSha) -> Result<Option<Run>>;
}

/// Content-addressed memory of what each strategy profile did with a blob.
#[async_trait]
pub trait VerdictCache: Send + Sync {
    async fn get(&self, blob: &BlobId, profile: &Profile) -> Result<Option<BlobVerdict>>;
    async fn put(&self, blob: &BlobId, profile: &Profile, verdict: &BlobVerdict, savings: Savings) -> Result<()>;
}

/// Files maintainers rejected (PR closed without merging), per repository.
#[async_trait]
pub trait RejectionStore: Send + Sync {
    async fn rejected(&self, repo: RepoId) -> Result<HashSet<BlobId>>;
    /// Rejects every original file of a run.
    async fn reject_run(&self, repo: RepoId, run: RunId) -> Result<()>;
    async fn forget(&self, repo: RepoId) -> Result<()>;
}
