use chrono::{DateTime, Utc};

use super::{Settings, named_enum};
use crate::values::{
    BranchName, ByteSize, CloneUrl, CommitSha, IgnoreRules, PollPolicy, PrNumber, RepoId, RepoSlug, SourceId,
};

/// How the bot's branch reaches the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepoMode {
    /// The developer can push: the branch goes to the repository itself.
    Direct,
    /// Third-party repository: the branch goes to the developer's fork.
    Fork,
}
named_enum!(RepoMode { Direct => "direct", Fork => "fork" });

/// Result of the last check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepoStatus {
    Pending,
    Ok,
    AwaitingApproval,
    Error,
}
named_enum!(RepoStatus { Pending => "pending", Ok => "ok", AwaitingApproval => "awaiting_approval", Error => "error" });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrState {
    Open,
    Merged,
    Closed,
}
named_enum!(PrState { Open => "open", Merged => "merged", Closed => "closed" });

/// The bot's latest pull request on a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: PrNumber,
    pub url: String,
    pub state: PrState,
}

/// The developer's fork used to contribute to a third-party repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fork {
    pub slug: RepoSlug,
    pub clone_url: CloneUrl,
}

/// A watched repository.
#[derive(Debug, Clone)]
pub struct Repository {
    pub id: RepoId,
    pub slug: RepoSlug,
    pub source_id: Option<SourceId>,
    pub mode: RepoMode,
    pub clone_url: CloneUrl,
    pub default_branch: BranchName,
    pub fork: Option<Fork>,
    pub enabled: bool,
    pub auto_pr: bool,
    /// Overrides the global interval when set.
    pub poll_interval_minutes: Option<u32>,
    pub ignore_rules: IgnoreRules,
    pub status: RepoStatus,
    pub last_error: Option<String>,
    pub last_seen: Option<CommitSha>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub next_check_at: Option<DateTime<Utc>>,
    /// A local commit waiting for the developer's approval.
    pub pending_commit: Option<CommitSha>,
    pub pull_request: Option<PullRequest>,
    /// Bytes saved by the bot's merged pull requests.
    pub bytes_saved: ByteSize,
    pub created_at: DateTime<Utc>,
}

impl Repository {
    /// The repository's own interval, or the global one, with the global jitter.
    pub fn poll_policy(&self, settings: &Settings) -> PollPolicy {
        let global = settings.poll_policy();
        match self.poll_interval_minutes {
            Some(minutes) => PollPolicy::new(minutes, global.jitter_minutes()),
            None => global,
        }
    }

    /// The pull request, if the last known state is open.
    pub fn open_pull_request(&self) -> Option<&PullRequest> {
        self.pull_request.as_ref().filter(|pr| pr.state == PrState::Open)
    }
}

/// What is needed to start watching a repository.
#[derive(Debug, Clone)]
pub struct NewRepository {
    pub slug: RepoSlug,
    pub source_id: Option<SourceId>,
    pub mode: RepoMode,
    pub clone_url: CloneUrl,
    pub default_branch: BranchName,
    pub auto_pr: bool,
}

/// Settings a user can change on a repository. `None` leaves a field alone;
/// `poll_interval_minutes: Some(None)` goes back to the global interval.
#[derive(Debug, Clone, Default)]
pub struct RepositoryChanges {
    pub enabled: Option<bool>,
    pub auto_pr: Option<bool>,
    pub mode: Option<RepoMode>,
    pub poll_interval_minutes: Option<Option<u32>>,
    pub ignore_rules: Option<IgnoreRules>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_their_names() {
        for s in [RepoStatus::Pending, RepoStatus::Ok, RepoStatus::AwaitingApproval, RepoStatus::Error] {
            assert_eq!(RepoStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(RepoMode::parse("fork"), Some(RepoMode::Fork));
        assert_eq!(PrState::parse("merged"), Some(PrState::Merged));
        assert_eq!(PrState::parse("weird"), None);
    }
}
