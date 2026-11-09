//! Row types and their conversion into domain entities.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use git_opt_domain::{
    model::{
        FileChange, Fork, PrState, PullRequest, RepoMode, RepoStatus, Repository, Run, RunOutcome, RunStatus, RunTrigger,
        Source, SourceKind,
    },
    ports::StoredFileChange,
    values::{
        BlobId, BranchName, ByteSize, CloneUrl, CommitSha, IgnoreRules, OptimizerId, OwnerName, PrNumber, RepoId,
        RepoName, RepoPath, RepoSlug, RunId, Savings, SourceId,
    },
};
use sqlx::FromRow;

fn bytes(n: i64) -> ByteSize {
    ByteSize::new(n.max(0) as u64)
}

#[derive(FromRow)]
pub(crate) struct RepoRow {
    id: i64,
    owner: String,
    name: String,
    source_id: Option<i64>,
    mode: String,
    clone_url: String,
    default_branch: String,
    fork_full_name: Option<String>,
    fork_clone_url: Option<String>,
    enabled: bool,
    auto_pr: bool,
    poll_interval_minutes: Option<i64>,
    ignore_globs: String,
    status: String,
    last_error: Option<String>,
    last_seen_sha: Option<String>,
    last_checked_at: Option<DateTime<Utc>>,
    next_check_at: Option<DateTime<Utc>>,
    pending_commit: Option<String>,
    pr_number: Option<i64>,
    pr_url: Option<String>,
    pr_state: Option<String>,
    bytes_saved: i64,
    created_at: DateTime<Utc>,
}

impl TryFrom<RepoRow> for Repository {
    type Error = anyhow::Error;

    fn try_from(r: RepoRow) -> Result<Self> {
        let context = format!("repository row {}", r.id);
        (|| -> Result<Self> {
            let fork = match (r.fork_full_name, r.fork_clone_url) {
                (Some(full), Some(url)) => Some(Fork { slug: RepoSlug::parse(&full)?, clone_url: CloneUrl::new(url)? }),
                _ => None,
            };
            let pull_request = r.pr_number.map(|number| PullRequest {
                number: PrNumber::new(number),
                url: r.pr_url.unwrap_or_default(),
                state: r.pr_state.as_deref().and_then(PrState::parse).unwrap_or(PrState::Open),
            });
            Ok(Repository {
                id: RepoId::new(r.id),
                slug: RepoSlug::new(OwnerName::new(r.owner)?, RepoName::new(r.name)?),
                source_id: r.source_id.map(SourceId::new),
                mode: RepoMode::parse(&r.mode).unwrap_or(RepoMode::Direct),
                clone_url: CloneUrl::new(r.clone_url)?,
                default_branch: BranchName::new(r.default_branch)?,
                fork,
                enabled: r.enabled,
                auto_pr: r.auto_pr,
                poll_interval_minutes: r.poll_interval_minutes.map(|m| m.clamp(1, u32::MAX as i64) as u32),
                // Patterns are validated on the way in; never fail a read over them.
                ignore_rules: IgnoreRules::new(r.ignore_globs).unwrap_or_default(),
                status: RepoStatus::parse(&r.status).unwrap_or(RepoStatus::Pending),
                last_error: r.last_error,
                last_seen: r.last_seen_sha.and_then(|s| CommitSha::new(s).ok()),
                last_checked_at: r.last_checked_at,
                next_check_at: r.next_check_at,
                pending_commit: r.pending_commit.and_then(|s| CommitSha::new(s).ok()),
                pull_request,
                bytes_saved: bytes(r.bytes_saved),
                created_at: r.created_at,
            })
        })()
        .context(context)
    }
}

#[derive(FromRow)]
pub(crate) struct SourceRow {
    id: i64,
    kind: String,
    name: String,
    include_forks: bool,
    include_archived: bool,
    auto_sync: bool,
    repo_count: i64,
    last_synced_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    created_at: DateTime<Utc>,
}

impl TryFrom<SourceRow> for Source {
    type Error = anyhow::Error;

    fn try_from(r: SourceRow) -> Result<Self> {
        Ok(Source {
            id: SourceId::new(r.id),
            kind: SourceKind::parse(&r.kind).with_context(|| format!("unknown source kind {:?}", r.kind))?,
            owner: if r.name.is_empty() { None } else { Some(OwnerName::new(r.name)?) },
            include_forks: r.include_forks,
            include_archived: r.include_archived,
            auto_sync: r.auto_sync,
            repo_count: r.repo_count.max(0) as u32,
            last_synced_at: r.last_synced_at,
            last_error: r.last_error,
            created_at: r.created_at,
        })
    }
}

#[derive(FromRow)]
pub(crate) struct RunRow {
    id: i64,
    repo_id: i64,
    trigger: String,
    status: String,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    head_sha: Option<String>,
    commit_sha: Option<String>,
    files_scanned: i64,
    files_optimized: i64,
    bytes_before: i64,
    bytes_after: i64,
    pr_number: Option<i64>,
    pr_url: Option<String>,
    message: Option<String>,
    #[sqlx(default)]
    repo_full_name: Option<String>,
}

impl TryFrom<RunRow> for Run {
    type Error = anyhow::Error;

    fn try_from(r: RunRow) -> Result<Self> {
        Ok(Run {
            id: RunId::new(r.id),
            repo_id: RepoId::new(r.repo_id),
            repo: r.repo_full_name.and_then(|s| RepoSlug::parse(&s).ok()),
            trigger: RunTrigger::parse(&r.trigger).unwrap_or(RunTrigger::Schedule),
            started_at: r.started_at,
            finished_at: r.finished_at,
            outcome: RunOutcome {
                status: RunStatus::parse(&r.status).unwrap_or(RunStatus::Error),
                head: r.head_sha.and_then(|s| CommitSha::new(s).ok()),
                commit: r.commit_sha.and_then(|s| CommitSha::new(s).ok()),
                files_scanned: r.files_scanned.max(0) as u32,
                files_optimized: r.files_optimized.max(0) as u32,
                savings: Savings::new(bytes(r.bytes_before), bytes(r.bytes_after)),
                pull_request: r.pr_number.map(|n| (PrNumber::new(n), r.pr_url.unwrap_or_default())),
                message: r.message,
            },
        })
    }
}

#[derive(FromRow)]
pub(crate) struct RunFileRow {
    id: i64,
    path: String,
    optimizer: String,
    original_blob: String,
    bytes_before: i64,
    bytes_after: i64,
}

impl TryFrom<RunFileRow> for StoredFileChange {
    type Error = anyhow::Error;

    fn try_from(r: RunFileRow) -> Result<Self> {
        Ok(StoredFileChange {
            id: r.id,
            change: FileChange {
                path: RepoPath::new(r.path)?,
                optimizer: OptimizerId::new(r.optimizer)?,
                original_blob: BlobId::new(r.original_blob)?,
                savings: Savings::new(bytes(r.bytes_before), bytes(r.bytes_after)),
            },
        })
    }
}

pub(crate) fn convert<R, T>(rows: Vec<R>) -> Result<Vec<T>>
where
    T: TryFrom<R, Error = anyhow::Error>,
{
    rows.into_iter().map(T::try_from).collect()
}
