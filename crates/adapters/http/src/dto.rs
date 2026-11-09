//! The JSON contract with the web UI (mirrored in `frontend/src/types.ts`).
//! Domain types never leak into JSON directly: they are mapped here.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use git_opt_application::{
    catalog::StrategyStatus,
    jobs::RunningJob,
    use_cases::{repositories::RepositoryView, status::Status},
};
use git_opt_domain::{
    InvalidValue,
    model::{Run, Settings, Source},
    ports::{StoredFileChange, Totals},
    values::{BranchName, CommitSha, IgnoreRules},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Serialize)]
pub struct RepoDto {
    id: i64,
    owner: String,
    name: String,
    source_id: Option<i64>,
    mode: &'static str,
    clone_url: String,
    default_branch: String,
    fork_full_name: Option<String>,
    fork_clone_url: Option<String>,
    enabled: bool,
    auto_pr: bool,
    poll_interval_minutes: Option<u32>,
    ignore_globs: String,
    status: &'static str,
    last_error: Option<String>,
    last_seen_sha: Option<String>,
    last_checked_at: Option<DateTime<Utc>>,
    next_check_at: Option<DateTime<Utc>>,
    pending_commit: Option<String>,
    pr_number: Option<i64>,
    pr_url: Option<String>,
    pr_state: Option<&'static str>,
    bytes_saved: u64,
    created_at: DateTime<Utc>,
    running: bool,
    queued: bool,
    phase: Option<String>,
}

impl From<RepositoryView> for RepoDto {
    fn from(v: RepositoryView) -> Self {
        let r = v.repository;
        let pr = r.pull_request.as_ref();
        Self {
            id: r.id.get(),
            owner: r.slug.owner().to_string(),
            name: r.slug.name().to_string(),
            source_id: r.source_id.map(|s| s.get()),
            mode: r.mode.as_str(),
            clone_url: r.clone_url.to_string(),
            default_branch: r.default_branch.to_string(),
            fork_full_name: r.fork.as_ref().map(|f| f.slug.to_string()),
            fork_clone_url: r.fork.as_ref().map(|f| f.clone_url.to_string()),
            enabled: r.enabled,
            auto_pr: r.auto_pr,
            poll_interval_minutes: r.poll_interval_minutes,
            ignore_globs: r.ignore_rules.as_str().to_string(),
            status: r.status.as_str(),
            last_error: r.last_error.clone(),
            last_seen_sha: r.last_seen.as_ref().map(CommitSha::to_string),
            last_checked_at: r.last_checked_at,
            next_check_at: r.next_check_at,
            pending_commit: r.pending_commit.as_ref().map(CommitSha::to_string),
            pr_number: pr.map(|p| p.number.get()),
            pr_url: pr.map(|p| p.url.clone()),
            pr_state: pr.map(|p| p.state.as_str()),
            bytes_saved: r.bytes_saved.get(),
            created_at: r.created_at,
            running: v.running,
            queued: v.queued,
            phase: v.phase,
        }
    }
}

#[derive(Serialize)]
pub struct SourceDto {
    id: i64,
    kind: &'static str,
    name: String,
    include_forks: bool,
    include_archived: bool,
    auto_sync: bool,
    repo_count: u32,
    last_synced_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<Source> for SourceDto {
    fn from(s: Source) -> Self {
        Self {
            id: s.id.get(),
            kind: s.kind.as_str(),
            name: s.owner.map(|o| o.to_string()).unwrap_or_default(),
            include_forks: s.include_forks,
            include_archived: s.include_archived,
            auto_sync: s.auto_sync,
            repo_count: s.repo_count,
            last_synced_at: s.last_synced_at,
            last_error: s.last_error,
            created_at: s.created_at,
        }
    }
}

#[derive(Serialize)]
pub struct RunDto {
    id: i64,
    repo_id: i64,
    repo_full_name: Option<String>,
    trigger: &'static str,
    status: &'static str,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    head_sha: Option<String>,
    commit_sha: Option<String>,
    files_scanned: u32,
    files_optimized: u32,
    bytes_before: u64,
    bytes_after: u64,
    pr_number: Option<i64>,
    pr_url: Option<String>,
    message: Option<String>,
}

impl From<Run> for RunDto {
    fn from(r: Run) -> Self {
        let o = r.outcome;
        Self {
            id: r.id.get(),
            repo_id: r.repo_id.get(),
            repo_full_name: r.repo.map(|s| s.to_string()),
            trigger: r.trigger.as_str(),
            status: o.status.as_str(),
            started_at: r.started_at,
            finished_at: r.finished_at,
            head_sha: o.head.map(|c| c.to_string()),
            commit_sha: o.commit.map(|c| c.to_string()),
            files_scanned: o.files_scanned,
            files_optimized: o.files_optimized,
            bytes_before: o.savings.before.get(),
            bytes_after: o.savings.after.get(),
            pr_number: o.pull_request.as_ref().map(|(n, _)| n.get()),
            pr_url: o.pull_request.map(|(_, url)| url),
            message: o.message,
        }
    }
}

#[derive(Serialize)]
pub struct RunFileDto {
    id: i64,
    run_id: i64,
    path: String,
    optimizer: String,
    original_blob: String,
    bytes_before: u64,
    bytes_after: u64,
}

impl RunFileDto {
    pub fn new(run_id: i64, f: StoredFileChange) -> Self {
        let c = f.change;
        Self {
            id: f.id,
            run_id,
            path: c.path.to_string(),
            optimizer: c.optimizer.to_string(),
            original_blob: c.original_blob.to_string(),
            bytes_before: c.savings.before.get(),
            bytes_after: c.savings.after.get(),
        }
    }
}

/// Settings as the UI sends and receives them.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsDto {
    poll_interval_minutes: u32,
    poll_jitter_minutes: u32,
    max_concurrent_jobs: u32,
    source_sync_interval_minutes: u32,
    min_savings_percent: f64,
    min_savings_bytes: u64,
    max_file_size_mb: u32,
    png_level: u8,
    png_zopfli: bool,
    strip_metadata: bool,
    optimizers: BTreeMap<String, bool>,
    ignore_globs: String,
    branch_name: String,
    commit_title: String,
    commit_author_name: String,
    commit_author_email: String,
    sign_commits: bool,
}

impl Default for SettingsDto {
    fn default() -> Self {
        Self::from(Settings::default())
    }
}

impl From<Settings> for SettingsDto {
    fn from(s: Settings) -> Self {
        Self {
            poll_interval_minutes: s.poll_interval_minutes,
            poll_jitter_minutes: s.poll_jitter_minutes,
            max_concurrent_jobs: s.max_concurrent_jobs,
            source_sync_interval_minutes: s.source_sync_interval_minutes,
            min_savings_percent: s.min_savings_percent,
            min_savings_bytes: s.min_savings_bytes,
            max_file_size_mb: s.max_file_size_mb,
            png_level: s.png_level,
            png_zopfli: s.png_zopfli,
            strip_metadata: s.strip_metadata,
            optimizers: s.optimizers,
            ignore_globs: s.ignore_rules.as_str().to_string(),
            branch_name: s.branch.to_string(),
            commit_title: s.commit_title,
            commit_author_name: s.commit_author_name,
            commit_author_email: s.commit_author_email,
            sign_commits: s.sign_commits,
        }
    }
}

impl TryFrom<SettingsDto> for Settings {
    type Error = InvalidValue;

    fn try_from(d: SettingsDto) -> Result<Self, InvalidValue> {
        let branch = d.branch_name.trim();
        Ok(Settings {
            poll_interval_minutes: d.poll_interval_minutes,
            poll_jitter_minutes: d.poll_jitter_minutes,
            max_concurrent_jobs: d.max_concurrent_jobs,
            source_sync_interval_minutes: d.source_sync_interval_minutes,
            min_savings_percent: d.min_savings_percent,
            min_savings_bytes: d.min_savings_bytes,
            max_file_size_mb: d.max_file_size_mb,
            png_level: d.png_level,
            png_zopfli: d.png_zopfli,
            strip_metadata: d.strip_metadata,
            optimizers: d.optimizers,
            ignore_rules: IgnoreRules::new(d.ignore_globs)?,
            branch: if branch.is_empty() { Settings::default_branch() } else { BranchName::new(branch)? },
            commit_title: d.commit_title,
            commit_author_name: d.commit_author_name,
            commit_author_email: d.commit_author_email,
            sign_commits: d.sign_commits,
        })
    }
}

fn strategy_json(s: &StrategyStatus) -> Value {
    let extensions: Vec<&str> = s.formats.iter().flat_map(|f| f.extensions().iter().copied()).collect();
    json!({
        "id": s.id, "label": s.label, "category": "image", "extensions": extensions,
        "enabled": s.enabled, "available": s.available, "detail": s.detail,
    })
}

fn running_json(j: &RunningJob) -> Value {
    json!({ "repo_id": j.repo.get(), "full_name": j.slug.to_string(), "phase": j.phase, "started_at": j.started_at })
}

fn totals_json(t: &Totals) -> Value {
    json!({
        "repos": t.repos, "repos_enabled": t.repos_enabled, "prs_open": t.prs_open,
        "awaiting_approval": t.awaiting_approval, "errors": t.errors,
        "bytes_saved": t.bytes_saved.get(), "prs_sent": t.prs_sent,
    })
}

pub fn status_json(s: &Status, version: &str) -> Value {
    let github = match (&s.forge.account, s.forge.token_origin) {
        (Ok((login, name)), origin) => {
            json!({ "ok": true, "login": login, "name": name, "token_source": origin.map(|o| o.as_str()) })
        }
        (Err(error), Some(origin)) => json!({ "ok": false, "token_source": origin.as_str(), "error": error }),
        (Err(error), None) => json!({ "ok": false, "error": error }),
    };
    let mut git = json!({});
    match &s.vcs.version {
        Ok(v) => git["version"] = json!(v),
        Err(e) => git["error"] = json!(e),
    }
    match &s.vcs.author {
        Ok((id, origin)) => {
            git["identity"] = json!({ "name": id.name(), "email": id.email() });
            git["identity_source"] = json!(origin.as_str());
        }
        Err(e) if git.get("error").is_none() => git["error"] = json!(e),
        Err(_) => {}
    }
    json!({
        "version": version,
        "github": github,
        "git": git,
        "optimizers": s.strategies.iter().map(strategy_json).collect::<Vec<_>>(),
        "jobs": {
            "running": s.running.iter().map(running_json).collect::<Vec<_>>(),
            "queued": s.queued.iter().map(|r| r.get()).collect::<Vec<_>>(),
        },
        "totals": totals_json(&s.totals),
    })
}
