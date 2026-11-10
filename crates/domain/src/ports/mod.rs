//! Ports: what the core needs from the outside world, as traits. Adapters
//! (SQLite, system git, GitHub, codecs…) implement them; swapping one is a
//! matter of writing another adapter.
//!
//! Port methods return `anyhow::Result`: failures come from infrastructure
//! and are reported, not matched on.

mod codec;
mod forge;
mod storage;
mod system;
mod vcs;

pub use codec::{FormatStrategy, Outcome, attempt};
pub use forge::{
    BranchPullRequest, CodeForge, ForgeAccount, ForgeBranch, ForgePullRequest, ForgeRepository, PullRequestHead,
    TokenOrigin,
};
pub use storage::{
    CheckRecord, RejectionStore, RepositoryStore, RunStore, SecretStore, SettingsStore, SourceStore, StoredFileChange,
    Totals, VerdictCache,
};
pub use system::{Clock, Random};
pub use vcs::{TreeEntry, VersionControl, Workspace};
