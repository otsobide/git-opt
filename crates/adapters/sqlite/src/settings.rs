//! Settings (one JSON document) and the saved token, in the key/value table.

use std::collections::BTreeMap;

use anyhow::Result;
use async_trait::async_trait;
use git_opt_domain::{
    model::Settings,
    ports::{SecretStore, SettingsStore},
    values::{AccessToken, BranchName, IgnoreRules},
};
use serde::{Deserialize, Serialize};

use crate::SqliteStore;

const SETTINGS_KEY: &str = "settings";
const TOKEN_KEY: &str = "github_token";

/// The stored JSON shape (kept stable across versions; unknown or missing
/// fields fall back to defaults).
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct SettingsDoc {
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

impl Default for SettingsDoc {
    fn default() -> Self {
        Self::from(&Settings::default())
    }
}

impl From<&Settings> for SettingsDoc {
    fn from(s: &Settings) -> Self {
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
            optimizers: s.optimizers.clone(),
            ignore_globs: s.ignore_rules.as_str().to_string(),
            branch_name: s.branch.as_str().to_string(),
            commit_title: s.commit_title.clone(),
            commit_author_name: s.commit_author_name.clone(),
            commit_author_email: s.commit_author_email.clone(),
            sign_commits: s.sign_commits,
        }
    }
}

impl From<SettingsDoc> for Settings {
    fn from(d: SettingsDoc) -> Self {
        Settings {
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
            ignore_rules: IgnoreRules::new(d.ignore_globs).unwrap_or_default(),
            branch: BranchName::new(d.branch_name).unwrap_or_else(|_| Settings::default_branch()),
            commit_title: d.commit_title,
            commit_author_name: d.commit_author_name,
            commit_author_email: d.commit_author_email,
            sign_commits: d.sign_commits,
        }
        .sanitized()
    }
}

impl SqliteStore {
    async fn kv_get(&self, key: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT value FROM settings WHERE key = ?").bind(key).fetch_optional(&self.pool).await?)
    }

    async fn kv_set(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => {
                sqlx::query(
                    "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                )
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
}

#[async_trait]
impl SettingsStore for SqliteStore {
    async fn load(&self) -> Result<Settings> {
        let doc = self.kv_get(SETTINGS_KEY).await?.and_then(|raw| serde_json::from_str::<SettingsDoc>(&raw).ok());
        Ok(doc.map(Settings::from).unwrap_or_default())
    }

    async fn save(&self, settings: &Settings) -> Result<()> {
        self.kv_set(SETTINGS_KEY, Some(&serde_json::to_string(&SettingsDoc::from(settings))?)).await
    }
}

#[async_trait]
impl SecretStore for SqliteStore {
    async fn token(&self) -> Result<Option<AccessToken>> {
        Ok(self.kv_get(TOKEN_KEY).await?.and_then(|t| AccessToken::new(t).ok()))
    }

    async fn set_token(&self, token: Option<&AccessToken>) -> Result<()> {
        self.kv_set(TOKEN_KEY, token.map(AccessToken::expose)).await
    }
}
