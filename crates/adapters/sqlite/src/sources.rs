use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use git_opt_domain::{
    model::{NewSource, Repository, Source, SourceChanges},
    ports::SourceStore,
    values::SourceId,
};

use crate::{
    SqliteStore,
    rows::{RepoRow, SourceRow, convert},
};

#[async_trait]
impl SourceStore for SqliteStore {
    async fn list(&self) -> Result<Vec<Source>> {
        let rows: Vec<SourceRow> =
            sqlx::query_as("SELECT * FROM sources ORDER BY kind, name").fetch_all(&self.pool).await?;
        convert(rows)
    }

    async fn get(&self, id: SourceId) -> Result<Option<Source>> {
        let row: Option<SourceRow> =
            sqlx::query_as("SELECT * FROM sources WHERE id = ?").bind(id.get()).fetch_optional(&self.pool).await?;
        row.map(Source::try_from).transpose()
    }

    async fn insert(&self, source: &NewSource, now: DateTime<Utc>) -> Result<SourceId> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO sources (kind, name, include_forks, include_archived, auto_sync, created_at)
             VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(source.kind.as_str())
        .bind(source.owner.as_ref().map_or("", |o| o.as_str()))
        .bind(source.include_forks)
        .bind(source.include_archived)
        .bind(source.auto_sync)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(SourceId::new(id))
    }

    async fn update(&self, id: SourceId, changes: &SourceChanges) -> Result<()> {
        sqlx::query(
            "UPDATE sources SET include_forks = COALESCE(?, include_forks),
                include_archived = COALESCE(?, include_archived), auto_sync = COALESCE(?, auto_sync)
             WHERE id = ?",
        )
        .bind(changes.include_forks)
        .bind(changes.include_archived)
        .bind(changes.auto_sync)
        .bind(id.get())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete(&self, id: SourceId, with_repositories: bool) -> Result<Vec<Repository>> {
        let mut tx = self.pool.begin().await?;
        let mut removed: Vec<RepoRow> = Vec::new();
        if with_repositories {
            removed = sqlx::query_as("DELETE FROM repos WHERE source_id = ? RETURNING *")
                .bind(id.get())
                .fetch_all(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM sources WHERE id = ?").bind(id.get()).execute(&mut *tx).await?;
        tx.commit().await?;
        convert(removed)
    }

    async fn mark_synced(
        &self,
        id: SourceId,
        at: DateTime<Utc>,
        repo_count: Option<u32>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE sources SET last_synced_at = ?, last_error = ?, repo_count = COALESCE(?, repo_count) WHERE id = ?",
        )
        .bind(at)
        .bind(error)
        .bind(repo_count.map(i64::from))
        .bind(id.get())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn due_for_sync(&self, before: DateTime<Utc>) -> Result<Vec<SourceId>> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM sources WHERE auto_sync = 1 AND (last_synced_at IS NULL OR last_synced_at <= ?)",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(SourceId::new).collect())
    }
}
