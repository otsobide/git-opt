//! Database rows and API payloads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum RepoMode {
    /// The token can push: branches go to the repository itself.
    Direct,
    /// Third-party repository: push to a fork and open a cross-repo PR.
    Fork,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum SourceKind {
    User,
    Org,
    /// Every repository the configured token has access to.
    Me,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Source {
    pub id: i64,
    pub kind: SourceKind,
    pub name: String,
    pub include_forks: bool,
    pub include_archived: bool,
    pub auto_sync: bool,
    pub repo_count: i64,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Repo {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub source_id: Option<i64>,
    pub mode: RepoMode,
    pub clone_url: String,
    pub default_branch: String,
    pub fork_full_name: Option<String>,
    pub fork_clone_url: Option<String>,
    pub enabled: bool,
    pub auto_pr: bool,
    pub poll_interval_minutes: Option<i64>,
    pub ignore_globs: String,
    /// `pending` | `ok` | `awaiting_approval` | `error`
    pub status: String,
    pub last_error: Option<String>,
    pub last_seen_sha: Option<String>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub next_check_at: Option<DateTime<Utc>>,
    pub pending_commit: Option<String>,
    pub pr_number: Option<i64>,
    pub pr_url: Option<String>,
    /// `open` | `merged` | `closed`
    pub pr_state: Option<String>,
    pub bytes_saved: i64,
    pub created_at: DateTime<Utc>,
}

impl Repo {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Values needed to insert a repository discovered through the forge API.
#[derive(Debug, Clone)]
pub struct NewRepo {
    pub owner: String,
    pub name: String,
    pub source_id: Option<i64>,
    pub mode: RepoMode,
    pub clone_url: String,
    pub default_branch: String,
    pub auto_pr: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoPatch {
    pub enabled: Option<bool>,
    pub auto_pr: Option<bool>,
    pub mode: Option<RepoMode>,
    /// `Some(None)` clears the override and falls back to the global value.
    #[serde(default, with = "double_option")]
    pub poll_interval_minutes: Option<Option<i64>>,
    pub ignore_globs: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    Schedule,
    Manual,
    Publish,
}

impl RunTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Manual => "manual",
            Self::Publish => "publish",
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Run {
    pub id: i64,
    pub repo_id: i64,
    /// `schedule` | `manual` | `publish`
    pub trigger: String,
    /// `running` | `no_changes` | `up_to_date` | `awaiting_approval` |
    /// `pr_opened` | `pr_updated` | `error`
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub head_sha: Option<String>,
    pub commit_sha: Option<String>,
    pub files_scanned: i64,
    pub files_optimized: i64,
    pub bytes_before: i64,
    pub bytes_after: i64,
    pub pr_number: Option<i64>,
    pub pr_url: Option<String>,
    pub message: Option<String>,
    #[sqlx(default)]
    pub repo_full_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct RunFile {
    pub id: i64,
    pub run_id: i64,
    pub path: String,
    pub optimizer: String,
    pub original_blob: String,
    pub bytes_before: i64,
    pub bytes_after: i64,
}

/// What we learned about a blob for a given optimizer profile.
#[derive(Debug, Clone, FromRow)]
pub struct CacheEntry {
    pub result: String,
    pub optimized_blob: Option<String>,
}

/// Distinguishes "field absent" from "field set to null" in PATCH bodies.
mod double_option {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(d).map(Some)
    }
}
