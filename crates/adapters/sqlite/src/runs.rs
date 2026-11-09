//! Runs, their files, the blob verdict cache and rejected blobs.

use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use git_opt_domain::{
    model::{BlobVerdict, FileChange, Run, RunOutcome, RunTrigger},
    ports::{RejectionStore, RunStore, StoredFileChange, VerdictCache},
    values::{BlobId, CommitSha, PrNumber, Profile, RepoId, RunId, Savings},
};

use crate::{
    SqliteStore,
    rows::{RunFileRow, RunRow, convert},
};

#[async_trait]
impl RunStore for SqliteStore {
    async fn record(
        &self,
        repo: RepoId,
        trigger: RunTrigger,
        started_at: DateTime<Utc>,
        finished_at: DateTime<Utc>,
        o: &RunOutcome,
        files: &[FileChange],
    ) -> Result<RunId> {
        let mut tx = self.pool.begin().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO runs (repo_id, trigger, status, started_at, finished_at, head_sha, commit_sha, files_scanned,
                files_optimized, bytes_before, bytes_after, pr_number, pr_url, message)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(repo.get())
        .bind(trigger.as_str())
        .bind(o.status.as_str())
        .bind(started_at)
        .bind(finished_at)
        .bind(o.head.as_ref().map(CommitSha::as_str))
        .bind(o.commit.as_ref().map(CommitSha::as_str))
        .bind(i64::from(o.files_scanned))
        .bind(i64::from(o.files_optimized))
        .bind(o.savings.before.get() as i64)
        .bind(o.savings.after.get() as i64)
        .bind(o.pull_request.as_ref().map(|(n, _)| n.get()))
        .bind(o.pull_request.as_ref().map(|(_, url)| url.as_str()))
        .bind(&o.message)
        .fetch_one(&mut *tx)
        .await?;
        for f in files {
            sqlx::query(
                "INSERT INTO run_files (run_id, path, optimizer, original_blob, bytes_before, bytes_after)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(f.path.as_str())
            .bind(f.optimizer.as_str())
            .bind(f.original_blob.as_str())
            .bind(f.savings.before.get() as i64)
            .bind(f.savings.after.get() as i64)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(RunId::new(id))
    }

    async fn list(&self, repo: Option<RepoId>, limit: u32) -> Result<Vec<Run>> {
        let repo = repo.map(|r| r.get());
        let rows: Vec<RunRow> = sqlx::query_as(
            "SELECT runs.*, repos.owner || '/' || repos.name AS repo_full_name
             FROM runs JOIN repos ON repos.id = runs.repo_id
             WHERE (? IS NULL OR runs.repo_id = ?) ORDER BY runs.id DESC LIMIT ?",
        )
        .bind(repo)
        .bind(repo)
        .bind(i64::from(limit.clamp(1, 500)))
        .fetch_all(&self.pool)
        .await?;
        convert(rows)
    }

    async fn get(&self, id: RunId) -> Result<Option<Run>> {
        let row: Option<RunRow> = sqlx::query_as(
            "SELECT runs.*, repos.owner || '/' || repos.name AS repo_full_name
             FROM runs JOIN repos ON repos.id = runs.repo_id WHERE runs.id = ?",
        )
        .bind(id.get())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Run::try_from).transpose()
    }

    async fn files(&self, id: RunId) -> Result<Vec<StoredFileChange>> {
        let rows: Vec<RunFileRow> =
            sqlx::query_as("SELECT * FROM run_files WHERE run_id = ? ORDER BY bytes_before - bytes_after DESC")
                .bind(id.get())
                .fetch_all(&self.pool)
                .await?;
        convert(rows)
    }

    async fn latest_for_pull_request(&self, repo: RepoId, number: PrNumber) -> Result<Option<Run>> {
        let row: Option<RunRow> = sqlx::query_as(
            "SELECT * FROM runs WHERE repo_id = ? AND pr_number = ? AND files_optimized > 0 ORDER BY id DESC LIMIT 1",
        )
        .bind(repo.get())
        .bind(number.get())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Run::try_from).transpose()
    }

    async fn by_commit(&self, repo: RepoId, commit: &CommitSha) -> Result<Option<Run>> {
        let row: Option<RunRow> =
            sqlx::query_as("SELECT * FROM runs WHERE repo_id = ? AND commit_sha = ? ORDER BY id DESC LIMIT 1")
                .bind(repo.get())
                .bind(commit.as_str())
                .fetch_optional(&self.pool)
                .await?;
        row.map(Run::try_from).transpose()
    }
}

#[async_trait]
impl VerdictCache for SqliteStore {
    async fn get(&self, blob: &BlobId, profile: &Profile) -> Result<Option<BlobVerdict>> {
        let row: Option<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT result, optimized_blob, detail FROM blob_cache WHERE blob_sha = ? AND profile = ?")
                .bind(blob.as_str())
                .bind(profile.as_str())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(result, optimized, detail)| match result.as_str() {
            "optimal" => Some(BlobVerdict::Optimal { detail }),
            "optimized" => Some(BlobVerdict::Optimized { blob: optimized.and_then(|b| BlobId::new(b).ok()) }),
            "failed" => Some(BlobVerdict::Failed { reason: detail.unwrap_or_default() }),
            _ => None,
        }))
    }

    async fn put(&self, blob: &BlobId, profile: &Profile, verdict: &BlobVerdict, savings: Savings) -> Result<()> {
        let (result, optimized, detail) = match verdict {
            BlobVerdict::Optimal { detail } => ("optimal", None, detail.clone()),
            BlobVerdict::Optimized { blob } => ("optimized", blob.as_ref().map(|b| b.as_str().to_string()), None),
            BlobVerdict::Failed { reason } => ("failed", None, Some(reason.clone())),
        };
        sqlx::query(
            "INSERT INTO blob_cache (blob_sha, profile, result, optimized_blob, bytes_before, bytes_after, detail, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (blob_sha, profile) DO UPDATE SET result = excluded.result,
                optimized_blob = excluded.optimized_blob, bytes_before = excluded.bytes_before,
                bytes_after = excluded.bytes_after, detail = excluded.detail, updated_at = excluded.updated_at",
        )
        .bind(blob.as_str())
        .bind(profile.as_str())
        .bind(result)
        .bind(optimized)
        .bind(savings.before.get() as i64)
        .bind(savings.after.get() as i64)
        .bind(detail)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl RejectionStore for SqliteStore {
    async fn rejected(&self, repo: RepoId) -> Result<HashSet<BlobId>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT blob_sha FROM repo_ignored_blobs WHERE repo_id = ?")
            .bind(repo.get())
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().filter_map(|b| BlobId::new(b).ok()).collect())
    }

    async fn reject_run(&self, repo: RepoId, run: RunId) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO repo_ignored_blobs (repo_id, blob_sha) SELECT ?, original_blob FROM run_files WHERE run_id = ?",
        )
        .bind(repo.get())
        .bind(run.get())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn forget(&self, repo: RepoId) -> Result<()> {
        sqlx::query("DELETE FROM repo_ignored_blobs WHERE repo_id = ?").bind(repo.get()).execute(&self.pool).await?;
        Ok(())
    }
}
