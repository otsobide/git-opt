//! Runtime settings editable from the web UI, persisted as JSON in the DB.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Base time between `git fetch` of each repository.
    pub poll_interval_minutes: u32,
    /// Random extra delay (0..=jitter) added to every interval.
    pub poll_jitter_minutes: u32,
    /// How many repositories are processed at the same time.
    pub max_concurrent_jobs: u32,
    /// How often user/org sources are re-listed to discover new repos.
    pub source_sync_interval_minutes: u32,
    /// Only keep a result that saves at least this percentage…
    pub min_savings_percent: f64,
    /// …and at least this many bytes.
    pub min_savings_bytes: u64,
    /// Files bigger than this are ignored.
    pub max_file_size_mb: u32,
    /// oxipng preset (0 = fastest, 6 = smallest).
    pub png_level: u8,
    /// Remove metadata that does not affect rendering (EXIF text, PNG text
    /// chunks…). Pixels are never touched either way.
    pub strip_metadata: bool,
    /// Optimizer id -> enabled.
    pub optimizers: BTreeMap<String, bool>,
    /// Newline-separated globs ignored in every repository.
    pub ignore_globs: String,
    /// Branch the bot pushes its work to.
    pub branch_name: String,
    /// Commit subject and PR title.
    pub commit_title: String,
    /// Override the commit author. Empty = system `git config`, then the
    /// GitHub account behind the token.
    pub commit_author_name: String,
    pub commit_author_email: String,
    /// When false, passes `-c commit.gpgsign=false` to git (useful in Docker
    /// when the mounted gitconfig asks for signing but no key is available).
    pub sign_commits: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_interval_minutes: 20,
            poll_jitter_minutes: 20,
            max_concurrent_jobs: 2,
            source_sync_interval_minutes: 360,
            min_savings_percent: 1.0,
            min_savings_bytes: 256,
            max_file_size_mb: 30,
            png_level: 3,
            strip_metadata: false,
            optimizers: BTreeMap::new(),
            ignore_globs: String::new(),
            branch_name: "git-opt/optimize-images".into(),
            commit_title: "[git-opt] Optimize images".into(),
            commit_author_name: String::new(),
            commit_author_email: String::new(),
            sign_commits: true,
        }
    }
}

impl Settings {
    /// Clamp values coming from the UI into sane ranges.
    pub fn sanitized(mut self) -> Self {
        self.poll_interval_minutes = self.poll_interval_minutes.clamp(1, 7 * 24 * 60);
        self.poll_jitter_minutes = self.poll_jitter_minutes.min(24 * 60);
        self.max_concurrent_jobs = self.max_concurrent_jobs.clamp(1, 32);
        self.source_sync_interval_minutes = self.source_sync_interval_minutes.clamp(5, 7 * 24 * 60);
        self.min_savings_percent = self.min_savings_percent.clamp(0.0, 99.0);
        self.max_file_size_mb = self.max_file_size_mb.clamp(1, 1024);
        self.png_level = self.png_level.min(6);
        self.branch_name = self.branch_name.trim().to_string();
        if self.branch_name.is_empty() {
            self.branch_name = Self::default().branch_name;
        }
        self.commit_title = self.commit_title.trim().to_string();
        if self.commit_title.is_empty() {
            self.commit_title = Self::default().commit_title;
        }
        self
    }

    pub fn optimizer_enabled(&self, id: &str, default: bool) -> bool {
        self.optimizers.get(id).copied().unwrap_or(default)
    }
}
