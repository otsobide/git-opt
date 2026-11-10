use chrono::{DateTime, Utc};

use super::named_enum;
use crate::values::{BlobId, CommitSha, OptimizerId, PrNumber, RepoId, RepoPath, RepoSlug, RunId, Savings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunTrigger {
    Schedule,
    Manual,
    Publish,
}
named_enum!(RunTrigger { Schedule => "schedule", Manual => "manual", Publish => "publish" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunStatus {
    /// The default branch did not move.
    NoChanges,
    /// Something changed but nothing is left to optimize.
    UpToDate,
    /// A commit is ready locally, waiting for the developer.
    AwaitingApproval,
    PrOpened,
    PrUpdated,
    /// The developer's fork was deleted: every pull request from it is merged.
    ForkDeleted,
    Error,
}
named_enum!(RunStatus {
    NoChanges => "no_changes",
    UpToDate => "up_to_date",
    AwaitingApproval => "awaiting_approval",
    PrOpened => "pr_opened",
    PrUpdated => "pr_updated",
    ForkDeleted => "fork_deleted",
    Error => "error",
});

/// One file the bot optimized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: RepoPath,
    pub optimizer: OptimizerId,
    /// The blob before optimization: remembered to honour rejected PRs.
    pub original_blob: BlobId,
    pub savings: Savings,
}

/// What a check or publish produced.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutcome {
    pub status: RunStatus,
    pub head: Option<CommitSha>,
    pub commit: Option<CommitSha>,
    pub files_scanned: u32,
    pub files_optimized: u32,
    pub savings: Savings,
    pub pull_request: Option<(PrNumber, String)>,
    pub message: Option<String>,
}

impl RunOutcome {
    pub fn new(status: RunStatus) -> Self {
        Self {
            status,
            head: None,
            commit: None,
            files_scanned: 0,
            files_optimized: 0,
            savings: Savings::default(),
            pull_request: None,
            message: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self { message: Some(message.into()), ..Self::new(RunStatus::Error) }
    }
}

/// A recorded run.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: RunId,
    pub repo_id: RepoId,
    pub repo: Option<RepoSlug>,
    pub trigger: RunTrigger,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: RunOutcome,
}
