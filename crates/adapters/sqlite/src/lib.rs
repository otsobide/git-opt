//! The persistence ports implemented with SQLite through sqlx. Every SQL
//! statement of the application lives in this crate; rows are converted into
//! domain entities (with validated value objects) at the boundary.

mod repositories;
mod rows;
mod runs;
mod settings;
mod sources;

use std::{path::Path, str::FromStr, time::Duration};

use anyhow::{Context, Result};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

/// One store implementing every persistence port, over one pool.
#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Opens (creating if needed) the database file and runs migrations.
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let options = SqliteConnectOptions::from_str("sqlite:")?
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        Self::connect(options, 8).await
    }

    /// A private in-memory database (for tests and demos).
    pub async fn in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        // One connection: every connection would get its own empty database.
        Self::connect(options, 1).await
    }

    async fn connect(options: SqliteConnectOptions, connections: u32) -> Result<Self> {
        let pool = SqlitePoolOptions::new().max_connections(connections).connect_with(options).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }
}

#[cfg(test)]
mod tests;
