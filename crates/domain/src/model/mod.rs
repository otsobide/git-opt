//! Entities and the enums that describe their state.

mod repository;
mod run;
mod settings;
mod source;
mod verdict;

pub use repository::{Fork, NewRepository, PrState, PullRequest, RepoMode, RepoStatus, Repository, RepositoryChanges};
pub use run::{FileChange, Run, RunOutcome, RunStatus, RunTrigger};
pub use settings::{CodecOptions, Settings};
pub use source::{NewSource, Source, SourceChanges, SourceKind};
pub use verdict::BlobVerdict;

/// Implements `as_str`/`parse` for a unit-only enum with stable names.
macro_rules! named_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
            pub fn parse(value: &str) -> Option<Self> {
                match value { $($text => Some(Self::$variant),)+ _ => None }
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
pub(crate) use named_enum;
