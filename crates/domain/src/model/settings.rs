use std::collections::BTreeMap;

use crate::values::{BranchName, ByteSize, Identity, IgnoreRules, OptimizerId, PollPolicy, SavingsThreshold};

/// Options handed to format strategies: only what affects their output.
#[derive(Debug, Clone, PartialEq)]
pub struct CodecOptions {
    /// oxipng preset, 0 (fastest) to 6 (smallest).
    pub png_level: u8,
    pub png_zopfli: bool,
    /// Remove PNG chunks that do not affect rendering.
    pub strip_metadata: bool,
    /// Optimizer id -> enabled; missing ids use the strategy's default.
    pub enabled: BTreeMap<String, bool>,
    /// Minimum gain worth a change (also for nested files, e.g. SVG images).
    pub threshold: SavingsThreshold,
}

impl Default for CodecOptions {
    fn default() -> Self {
        Settings::default().codec_options()
    }
}

impl CodecOptions {
    pub fn is_enabled(&self, id: &OptimizerId, default: bool) -> bool {
        self.enabled.get(id.as_str()).copied().unwrap_or(default)
    }
}

/// Global runtime settings, edited from the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub poll_interval_minutes: u32,
    pub poll_jitter_minutes: u32,
    pub max_concurrent_jobs: u32,
    pub source_sync_interval_minutes: u32,
    pub min_savings_percent: f64,
    pub min_savings_bytes: u64,
    pub max_file_size_mb: u32,
    pub png_level: u8,
    pub png_zopfli: bool,
    pub strip_metadata: bool,
    pub optimizers: BTreeMap<String, bool>,
    pub ignore_rules: IgnoreRules,
    pub branch: BranchName,
    pub commit_title: String,
    /// Both must be set to override the author (kept as typed by the user).
    pub commit_author_name: String,
    pub commit_author_email: String,
    pub sign_commits: bool,
    /// In fork mode, delete the developer's fork once every pull request from it is merged.
    pub delete_merged_forks: bool,
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
            png_zopfli: false,
            strip_metadata: false,
            optimizers: BTreeMap::new(),
            ignore_rules: IgnoreRules::default(),
            branch: Self::default_branch(),
            commit_title: Self::DEFAULT_TITLE.into(),
            commit_author_name: String::new(),
            commit_author_email: String::new(),
            sign_commits: true,
            delete_merged_forks: true,
        }
    }
}

impl Settings {
    pub const DEFAULT_TITLE: &'static str = "[git-opt] Optimize images";

    pub fn default_branch() -> BranchName {
        BranchName::new("git-opt/optimize-images").expect("valid default branch")
    }

    /// Clamps numbers into their valid ranges and normalizes text.
    pub fn sanitized(mut self) -> Self {
        let poll = PollPolicy::new(self.poll_interval_minutes, self.poll_jitter_minutes);
        self.poll_interval_minutes = poll.interval_minutes();
        self.poll_jitter_minutes = poll.jitter_minutes();
        self.max_concurrent_jobs = self.max_concurrent_jobs.clamp(1, 32);
        self.source_sync_interval_minutes = self.source_sync_interval_minutes.clamp(5, 7 * 24 * 60);
        self.min_savings_percent = self.min_savings_percent.clamp(0.0, 99.0);
        self.max_file_size_mb = self.max_file_size_mb.clamp(1, 1024);
        self.png_level = self.png_level.min(6);
        let title = self.commit_title.lines().next().unwrap_or_default().trim().to_string();
        self.commit_title = if title.is_empty() { Self::DEFAULT_TITLE.into() } else { title };
        self.commit_author_name = self.commit_author_name.trim().to_string();
        self.commit_author_email = self.commit_author_email.trim().to_string();
        self
    }

    pub fn poll_policy(&self) -> PollPolicy {
        PollPolicy::new(self.poll_interval_minutes, self.poll_jitter_minutes)
    }

    pub fn threshold(&self) -> SavingsThreshold {
        SavingsThreshold::new(self.min_savings_percent, self.min_savings_bytes)
    }

    pub fn max_file_size(&self) -> ByteSize {
        ByteSize::new(u64::from(self.max_file_size_mb) * 1024 * 1024)
    }

    pub fn codec_options(&self) -> CodecOptions {
        CodecOptions {
            png_level: self.png_level,
            png_zopfli: self.png_zopfli,
            strip_metadata: self.strip_metadata,
            enabled: self.optimizers.clone(),
            threshold: self.threshold(),
        }
    }

    /// The author set in the UI, when both name and email form a valid identity.
    pub fn author_override(&self) -> Option<Identity> {
        Identity::new(&self.commit_author_name, &self.commit_author_email).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_clamps_and_normalizes() {
        let s = Settings {
            poll_interval_minutes: 0,
            max_concurrent_jobs: 100,
            png_level: 9,
            commit_title: "  \n".into(),
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.poll_interval_minutes, 1);
        assert_eq!(s.max_concurrent_jobs, 32);
        assert_eq!(s.png_level, 6);
        assert_eq!(s.commit_title, Settings::DEFAULT_TITLE);
    }

    #[test]
    fn author_override_needs_a_valid_pair() {
        let mut s = Settings::default();
        assert!(s.author_override().is_none());
        s.commit_author_name = "Jane".into();
        assert!(s.author_override().is_none(), "email missing");
        s.commit_author_email = "jane@example.com".into();
        assert_eq!(s.author_override().unwrap().to_string(), "Jane <jane@example.com>");
    }
}
