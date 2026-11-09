//! Process-level configuration read from environment variables.
//!
//! Runtime-tunable settings (intervals, thresholds…) live in the database and
//! are managed from the web UI (domain `Settings`, stored by the SQLite adapter).

use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    /// Address the HTTP server binds to.
    pub bind: SocketAddr,
    /// Root directory for the database and the repository clones.
    pub data_dir: PathBuf,
    /// Directory with the built frontend (`frontend/dist`). Optional.
    pub static_dir: Option<PathBuf>,
    /// If set, every `/api` request must carry `Authorization: Bearer <token>`.
    pub admin_token: Option<String>,
    /// GitHub REST API base URL (change it for GitHub Enterprise).
    pub github_api: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind = env("GIT_OPT_BIND")
            .unwrap_or_else(|| "127.0.0.1:8484".into())
            .parse()
            .context("GIT_OPT_BIND must be an address like 127.0.0.1:8484")?;

        let data_dir = env("GIT_OPT_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("git-opt")
        });

        // Explicit dir first, then sensible defaults for a repository checkout.
        let static_dir = env("GIT_OPT_STATIC_DIR")
            .map(PathBuf::from)
            .into_iter()
            .chain(["frontend/dist", "../frontend/dist"].map(PathBuf::from))
            .find(|p| p.join("index.html").is_file());

        Ok(Self {
            bind,
            data_dir,
            static_dir,
            admin_token: env("GIT_OPT_ADMIN_TOKEN"),
            github_api: env("GIT_OPT_GITHUB_API")
                .unwrap_or_else(|| "https://api.github.com".into())
                .trim_end_matches('/')
                .to_string(),
        })
    }

    pub fn repos_dir(&self) -> PathBuf {
        self.data_dir.join("repos")
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("git-opt.db")
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}
