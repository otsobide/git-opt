use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use git_opt_domain::{
    model::{Fork, NewRepository, PullRequest, RepoStatus, Repository, RepositoryChanges},
    ports::{CheckRecord, RepositoryStore, Totals},
    values::{BranchName, ByteSize, CloneUrl, CommitSha, RepoId, RepoSlug},
};
use sqlx::{QueryBuilder, Sqlite};

use crate::{
    SqliteStore,
    rows::{RepoRow, convert},
};

#[derive(sqlx::FromRow)]
struct TotalsRow {
    repos: i64,
    repos_enabled: i64,
    prs_open: i64,
    awaiting_approval: i64,
    errors: i64,
    bytes_saved: i64,
    prs_sent: i64,
}

#[async_trait]
impl RepositoryStore for SqliteStore {
    async fn list(&self) -> Result<Vec<Repository>> {
        let rows: Vec<RepoRow> = sqlx::query_as("SELECT * FROM repos ORDER BY owner COLLATE NOCASE, name COLLATE NOCASE")
            .fetch_all(&self.pool)
            .await?;
        convert(rows)
    }

    async fn get(&self, id: RepoId) -> Result<Option<Repository>> {
        let row: Option<RepoRow> =
            sqlx::query_as("SELECT * FROM repos WHERE id = ?").bind(id.get()).fetch_optional(&self.pool).await?;
        row.map(Repository::try_from).transpose()
    }

    async fn find(&self, slug: &RepoSlug) -> Result<Option<Repository>> {
        let row: Option<RepoRow> =
            sqlx::query_as("SELECT * FROM repos WHERE owner = ? COLLATE NOCASE AND name = ? COLLATE NOCASE")
                .bind(slug.owner().as_str())
                .bind(slug.name().as_str())
                .fetch_optional(&self.pool)
                .await?;
        row.map(Repository::try_from).transpose()
    }

    async fn insert(&self, r: &NewRepository, now: DateTime<Utc>) -> Result<(RepoId, bool)> {
        if let Some(existing) = RepositoryStore::find(self, &r.slug).await? {
            return Ok((existing.id, false));
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO repos (owner, name, source_id, mode, clone_url, default_branch, auto_pr, next_check_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(r.slug.owner().as_str())
        .bind(r.slug.name().as_str())
        .bind(r.source_id.map(|s| s.get()))
        .bind(r.mode.as_str())
        .bind(r.clone_url.as_str())
        .bind(r.default_branch.as_str())
        .bind(r.auto_pr)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok((RepoId::new(id), true))
    }

    async fn update(&self, id: RepoId, c: &RepositoryChanges) -> Result<()> {
        let mut q: QueryBuilder<Sqlite> = QueryBuilder::new("UPDATE repos SET id = id");
        if let Some(v) = c.enabled {
            q.push(", enabled = ").push_bind(v);
        }
        if let Some(v) = c.auto_pr {
            q.push(", auto_pr = ").push_bind(v);
        }
        if let Some(v) = c.mode {
            q.push(", mode = ").push_bind(v.as_str());
        }
        if let Some(v) = c.poll_interval_minutes {
            q.push(", poll_interval_minutes = ").push_bind(v.map(i64::from));
        }
        if let Some(v) = &c.ignore_rules {
            q.push(", ignore_globs = ").push_bind(v.as_str().to_string());
        }
        q.push(" WHERE id = ").push_bind(id.get());
        q.build().execute(&self.pool).await?;
        Ok(())
    }

    async fn delete(&self, id: RepoId) -> Result<Option<Repository>> {
        let row: Option<RepoRow> = sqlx::query_as("DELETE FROM repos WHERE id = ? RETURNING *")
            .bind(id.get())
            .fetch_optional(&self.pool)
            .await?;
        row.map(Repository::try_from).transpose()
    }

    async fn update_origin(&self, id: RepoId, clone_url: &CloneUrl, default_branch: &BranchName) -> Result<()> {
        sqlx::query("UPDATE repos SET clone_url = ?, default_branch = ? WHERE id = ?")
            .bind(clone_url.as_str())
            .bind(default_branch.as_str())
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_fork(&self, id: RepoId, fork: &Fork) -> Result<()> {
        sqlx::query("UPDATE repos SET fork_full_name = ?, fork_clone_url = ? WHERE id = ?")
            .bind(fork.slug.to_string())
            .bind(fork.clone_url.as_str())
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn due(&self, now: DateTime<Utc>) -> Result<Vec<RepoId>> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM repos WHERE enabled = 1 AND (next_check_at IS NULL OR next_check_at <= ?) ORDER BY next_check_at",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(RepoId::new).collect())
    }

    async fn schedule(&self, id: RepoId, at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE repos SET next_check_at = ? WHERE id = ?").bind(at).bind(id.get()).execute(&self.pool).await?;
        Ok(())
    }

    async fn record_check(&self, id: RepoId, r: &CheckRecord) -> Result<()> {
        sqlx::query(
            "UPDATE repos SET status = ?, last_error = ?, last_seen_sha = COALESCE(?, last_seen_sha),
                last_checked_at = ?, next_check_at = ? WHERE id = ?",
        )
        .bind(r.status.as_str())
        .bind(&r.error)
        .bind(r.seen.as_ref().map(CommitSha::as_str))
        .bind(r.checked_at)
        .bind(r.next_check_at)
        .bind(id.get())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn set_status(&self, id: RepoId, status: RepoStatus, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE repos SET status = ?, last_error = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(error)
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_pending_commit(&self, id: RepoId, commit: Option<&CommitSha>) -> Result<()> {
        sqlx::query("UPDATE repos SET pending_commit = ? WHERE id = ?")
            .bind(commit.map(CommitSha::as_str))
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_pull_request(&self, id: RepoId, pr: &PullRequest) -> Result<()> {
        sqlx::query("UPDATE repos SET pr_number = ?, pr_url = ?, pr_state = ? WHERE id = ?")
            .bind(pr.number.get())
            .bind(&pr.url)
            .bind(pr.state.as_str())
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn add_bytes_saved(&self, id: RepoId, bytes: ByteSize) -> Result<()> {
        sqlx::query("UPDATE repos SET bytes_saved = bytes_saved + ? WHERE id = ?")
            .bind(bytes.get() as i64)
            .bind(id.get())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn totals(&self) -> Result<Totals> {
        let t: TotalsRow = sqlx::query_as(
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
        .await?;
        let n = |v: i64| v.max(0) as u64;
        Ok(Totals {
            repos: n(t.repos),
            repos_enabled: n(t.repos_enabled),
            prs_open: n(t.prs_open),
            awaiting_approval: n(t.awaiting_approval),
            errors: n(t.errors),
            bytes_saved: ByteSize::new(n(t.bytes_saved)),
            prs_sent: n(t.prs_sent),
        })
    }
}
