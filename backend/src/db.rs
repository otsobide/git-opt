//! SQLite persistence. Every query lives here so the schema has one owner.

use std::{collections::HashSet, path::Path, str::FromStr, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::{
    QueryBuilder, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::{
    models::{CacheEntry, NewRepo, Repo, RepoPatch, Run, RunFile, Source, SourceKind},
    settings::Settings,
};

const SETTINGS_KEY: &str = "settings";
pub const TOKEN_KEY: &str = "github_token";

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

/// Final numbers of a run, written once it finishes.
#[derive(Debug, Default, Clone)]
pub struct RunOutcome {
    pub status: String,
    pub head_sha: Option<String>,
    pub commit_sha: Option<String>,
    pub files_scanned: i64,
    pub files_optimized: i64,
    pub bytes_before: i64,
    pub bytes_after: i64,
    pub pr_number: Option<i64>,
    pub pr_url: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewRunFile {
    pub path: String,
    pub optimizer: String,
    pub original_blob: String,
    pub bytes_before: i64,
    pub bytes_after: i64,
}

impl Db {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let opts = SqliteConnectOptions::from_str("sqlite:")?
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new().max_connections(8).connect_with(opts).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    // ---------------------------------------------------------------- settings

    pub async fn settings(&self) -> Result<Settings> {
        let raw = self.kv_get(SETTINGS_KEY).await?;
        Ok(raw
            .and_then(|r| serde_json::from_str::<Settings>(&r).ok())
            .unwrap_or_default())
    }

    pub async fn save_settings(&self, s: &Settings) -> Result<()> {
        self.kv_set(SETTINGS_KEY, Some(&serde_json::to_string(s)?)).await
    }

    pub async fn kv_get(&self, key: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn kv_set(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => {
                sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                    .bind(key)
                    .bind(v)
                    .execute(&self.pool)
                    .await?;
            }
            None => {
                sqlx::query("DELETE FROM settings WHERE key = ?").bind(key).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    // ----------------------------------------------------------------- sources

    pub async fn sources(&self) -> Result<Vec<Source>> {
        Ok(sqlx::query_as("SELECT * FROM sources ORDER BY kind, name").fetch_all(&self.pool).await?)
    }

    pub async fn source(&self, id: i64) -> Result<Option<Source>> {
        Ok(sqlx::query_as("SELECT * FROM sources WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn insert_source(
        &self,
        kind: SourceKind,
        name: &str,
        include_forks: bool,
        include_archived: bool,
        auto_sync: bool,
    ) -> Result<i64> {
        let id = sqlx::query_scalar(
            "INSERT INTO sources (kind, name, include_forks, include_archived, auto_sync, created_at)
             VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(kind)
        .bind(name)
        .bind(include_forks)
        .bind(include_archived)
        .bind(auto_sync)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn update_source(
        &self,
        id: i64,
        include_forks: Option<bool>,
        include_archived: Option<bool>,
        auto_sync: Option<bool>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sources SET include_forks = COALESCE(?, include_forks),
                include_archived = COALESCE(?, include_archived), auto_sync = COALESCE(?, auto_sync)
             WHERE id = ?",
        )
        .bind(include_forks)
        .bind(include_archived)
        .bind(auto_sync)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_source(&self, id: i64, delete_repos: bool) -> Result<Vec<Repo>> {
        let mut tx = self.pool.begin().await?;
        let mut removed = Vec::new();
        if delete_repos {
            removed = sqlx::query_as("DELETE FROM repos WHERE source_id = ? RETURNING *")
                .bind(id)
                .fetch_all(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM sources WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(removed)
    }

    pub async fn mark_source_synced(&self, id: i64, repo_count: Option<i64>, error: Option<&str>) -> Result<()> {
        sqlx::query(
            "UPDATE sources SET last_synced_at = ?, last_error = ?, repo_count = COALESCE(?, repo_count) WHERE id = ?",
        )
        .bind(Utc::now())
        .bind(error)
        .bind(repo_count)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Sources whose periodic re-sync is due.
    pub async fn due_sources(&self, older_than: DateTime<Utc>) -> Result<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM sources WHERE auto_sync = 1 AND (last_synced_at IS NULL OR last_synced_at <= ?)",
        )
        .bind(older_than)
        .fetch_all(&self.pool)
        .await?)
    }

    // ------------------------------------------------------------------- repos

    pub async fn repos(&self) -> Result<Vec<Repo>> {
        Ok(sqlx::query_as("SELECT * FROM repos ORDER BY owner COLLATE NOCASE, name COLLATE NOCASE")
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn repo(&self, id: i64) -> Result<Option<Repo>> {
        Ok(sqlx::query_as("SELECT * FROM repos WHERE id = ?").bind(id).fetch_optional(&self.pool).await?)
    }

    pub async fn find_repo(&self, owner: &str, name: &str) -> Result<Option<Repo>> {
        Ok(
            sqlx::query_as("SELECT * FROM repos WHERE owner = ? COLLATE NOCASE AND name = ? COLLATE NOCASE")
                .bind(owner)
                .bind(name)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Inserts the repo unless it already exists. Returns the id and whether it was new.
    pub async fn insert_repo(&self, r: &NewRepo) -> Result<(i64, bool)> {
        if let Some(existing) = self.find_repo(&r.owner, &r.name).await? {
            return Ok((existing.id, false));
        }
        let id = sqlx::query_scalar(
            "INSERT INTO repos (owner, name, source_id, mode, clone_url, default_branch, auto_pr, next_check_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&r.owner)
        .bind(&r.name)
        .bind(r.source_id)
        .bind(r.mode)
        .bind(&r.clone_url)
        .bind(&r.default_branch)
        .bind(r.auto_pr)
        .bind(Utc::now())
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?;
        Ok((id, true))
    }

    pub async fn patch_repo(&self, id: i64, p: &RepoPatch) -> Result<()> {
        let mut q: QueryBuilder<Sqlite> = QueryBuilder::new("UPDATE repos SET id = id");
        if let Some(v) = p.enabled {
            q.push(", enabled = ").push_bind(v);
        }
        if let Some(v) = p.auto_pr {
            q.push(", auto_pr = ").push_bind(v);
        }
        if let Some(v) = p.mode {
            q.push(", mode = ").push_bind(v);
        }
        if let Some(v) = p.poll_interval_minutes {
            q.push(", poll_interval_minutes = ").push_bind(v.map(|m| m.clamp(1, 7 * 24 * 60)));
        }
        if let Some(v) = &p.ignore_globs {
            q.push(", ignore_globs = ").push_bind(v.clone());
        }
        q.push(" WHERE id = ").push_bind(id);
        q.build().execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_repo(&self, id: i64) -> Result<Option<Repo>> {
        Ok(sqlx::query_as("DELETE FROM repos WHERE id = ? RETURNING *")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Refreshes metadata that may change upstream (default branch, clone URL).
    pub async fn refresh_repo_meta(&self, id: i64, clone_url: &str, default_branch: &str) -> Result<()> {
        sqlx::query("UPDATE repos SET clone_url = ?, default_branch = ? WHERE id = ?")
            .bind(clone_url)
            .bind(default_branch)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_fork(&self, id: i64, full_name: &str, clone_url: &str) -> Result<()> {
        sqlx::query("UPDATE repos SET fork_full_name = ?, fork_clone_url = ? WHERE id = ?")
            .bind(full_name)
            .bind(clone_url)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn due_repos(&self, now: DateTime<Utc>) -> Result<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM repos WHERE enabled = 1 AND (next_check_at IS NULL OR next_check_at <= ?) ORDER BY next_check_at",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn set_next_check(&self, id: i64, at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE repos SET next_check_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records the result of a check.
    pub async fn finish_check(
        &self,
        id: i64,
        status: &str,
        error: Option<&str>,
        seen_sha: Option<&str>,
        next_check_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE repos SET status = ?, last_error = ?, last_seen_sha = COALESCE(?, last_seen_sha),
                last_checked_at = ?, next_check_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(error)
        .bind(seen_sha)
        .bind(Utc::now())
        .bind(next_check_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_pending_commit(&self, id: i64, commit: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE repos SET pending_commit = ? WHERE id = ?")
            .bind(commit)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_pr(&self, id: i64, number: Option<i64>, url: Option<&str>, state: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE repos SET pr_number = ?, pr_url = ?, pr_state = ? WHERE id = ?")
            .bind(number)
            .bind(url)
            .bind(state)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_bytes_saved(&self, id: i64, bytes: i64) -> Result<()> {
        sqlx::query("UPDATE repos SET bytes_saved = bytes_saved + ? WHERE id = ?")
            .bind(bytes)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // -------------------------------------------------------------------- runs

    pub async fn create_run(&self, repo_id: i64, trigger: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "INSERT INTO runs (repo_id, trigger, status, started_at) VALUES (?, ?, 'running', ?) RETURNING id",
        )
        .bind(repo_id)
        .bind(trigger)
        .bind(Utc::now())
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn finish_run(&self, id: i64, o: &RunOutcome) -> Result<()> {
        sqlx::query(
            "UPDATE runs SET status = ?, finished_at = ?, head_sha = ?, commit_sha = ?, files_scanned = ?,
                files_optimized = ?, bytes_before = ?, bytes_after = ?, pr_number = ?, pr_url = ?, message = ?
             WHERE id = ?",
        )
        .bind(&o.status)
        .bind(Utc::now())
        .bind(&o.head_sha)
        .bind(&o.commit_sha)
        .bind(o.files_scanned)
        .bind(o.files_optimized)
        .bind(o.bytes_before)
        .bind(o.bytes_after)
        .bind(o.pr_number)
        .bind(&o.pr_url)
        .bind(&o.message)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn add_run_files(&self, run_id: i64, files: &[NewRunFile]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for f in files {
            sqlx::query(
                "INSERT INTO run_files (run_id, path, optimizer, original_blob, bytes_before, bytes_after)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(run_id)
            .bind(&f.path)
            .bind(&f.optimizer)
            .bind(&f.original_blob)
            .bind(f.bytes_before)
            .bind(f.bytes_after)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn runs(&self, repo_id: Option<i64>, limit: i64) -> Result<Vec<Run>> {
        Ok(sqlx::query_as(
            "SELECT runs.*, repos.owner || '/' || repos.name AS repo_full_name
             FROM runs JOIN repos ON repos.id = runs.repo_id
             WHERE (? IS NULL OR runs.repo_id = ?) ORDER BY runs.id DESC LIMIT ?",
        )
        .bind(repo_id)
        .bind(repo_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn run(&self, id: i64) -> Result<Option<Run>> {
        Ok(sqlx::query_as(
            "SELECT runs.*, repos.owner || '/' || repos.name AS repo_full_name
             FROM runs JOIN repos ON repos.id = runs.repo_id WHERE runs.id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn run_files(&self, run_id: i64) -> Result<Vec<RunFile>> {
        Ok(sqlx::query_as("SELECT * FROM run_files WHERE run_id = ? ORDER BY bytes_before - bytes_after DESC")
            .bind(run_id)
            .fetch_all(&self.pool)
            .await?)
    }

    /// The most recent run that produced the branch behind a given PR.
    pub async fn last_run_for_pr(&self, repo_id: i64, pr_number: i64) -> Result<Option<Run>> {
        Ok(sqlx::query_as(
            "SELECT *, NULL AS repo_full_name FROM runs
             WHERE repo_id = ? AND pr_number = ? AND files_optimized > 0 ORDER BY id DESC LIMIT 1",
        )
        .bind(repo_id)
        .bind(pr_number)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Runs left in `running` by a crash or restart.
    pub async fn abort_stale_runs(&self) -> Result<u64> {
        Ok(sqlx::query(
            "UPDATE runs SET status = 'error', finished_at = ?, message = 'interrupted (server restart)'
             WHERE status = 'running'",
        )
        .bind(Utc::now())
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    pub async fn totals(&self) -> Result<Totals> {
        Ok(sqlx::query_as(
            "SELECT
                (SELECT COUNT(*) FROM repos) AS repos,
                (SELECT COUNT(*) FROM repos WHERE enabled = 1) AS repos_enabled,
                (SELECT COUNT(*) FROM repos WHERE pr_state = 'open') AS prs_open,
                (SELECT COUNT(*) FROM repos WHERE status = 'awaiting_approval') AS awaiting_approval,
                (SELECT COUNT(*) FROM repos WHERE status = 'error') AS errors,
                (SELECT COALESCE(SUM(bytes_saved), 0) FROM repos) AS bytes_saved,
                (SELECT COUNT(*) FROM runs WHERE status IN ('pr_opened', 'pr_updated')) AS prs_sent",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    // -------------------------------------------------------------- blob cache

    pub async fn cache_get(&self, blob: &str, profile: &str) -> Result<Option<CacheEntry>> {
        Ok(
            sqlx::query_as("SELECT result, optimized_blob FROM blob_cache WHERE blob_sha = ? AND profile = ?")
                .bind(blob)
                .bind(profile)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn cache_put(
        &self,
        blob: &str,
        profile: &str,
        result: &str,
        optimized_blob: Option<&str>,
        bytes_before: i64,
        bytes_after: i64,
        detail: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO blob_cache (blob_sha, profile, result, optimized_blob, bytes_before, bytes_after, detail, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (blob_sha, profile) DO UPDATE SET result = excluded.result,
                optimized_blob = excluded.optimized_blob, bytes_before = excluded.bytes_before,
                bytes_after = excluded.bytes_after, detail = excluded.detail, updated_at = excluded.updated_at",
        )
        .bind(blob)
        .bind(profile)
        .bind(result)
        .bind(optimized_blob)
        .bind(bytes_before)
        .bind(bytes_after)
        .bind(detail)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ignored_blobs(&self, repo_id: i64) -> Result<HashSet<String>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT blob_sha FROM repo_ignored_blobs WHERE repo_id = ?")
            .bind(repo_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// Remembers every original blob of a rejected PR so it is not proposed again.
    pub async fn ignore_run_blobs(&self, repo_id: i64, run_id: i64) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO repo_ignored_blobs (repo_id, blob_sha)
             SELECT ?, original_blob FROM run_files WHERE run_id = ?",
        )
        .bind(repo_id)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn clear_ignored_blobs(&self, repo_id: i64) -> Result<()> {
        sqlx::query("DELETE FROM repo_ignored_blobs WHERE repo_id = ?")
            .bind(repo_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct Totals {
    pub repos: i64,
    pub repos_enabled: i64,
    pub prs_open: i64,
    pub awaiting_approval: i64,
    pub errors: i64,
    pub bytes_saved: i64,
    pub prs_sent: i64,
}
