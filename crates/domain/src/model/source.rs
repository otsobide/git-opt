use chrono::{DateTime, Utc};

use super::named_enum;
use crate::values::{OwnerName, SourceId};

/// Where repositories are imported from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    User,
    Org,
    /// Every repository the developer's token can access.
    Me,
}
named_enum!(SourceKind { User => "user", Org => "org", Me => "me" });

#[derive(Debug, Clone)]
pub struct Source {
    pub id: SourceId,
    pub kind: SourceKind,
    /// `None` for [`SourceKind::Me`].
    pub owner: Option<OwnerName>,
    pub include_forks: bool,
    pub include_archived: bool,
    pub auto_sync: bool,
    pub repo_count: u32,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewSource {
    pub kind: SourceKind,
    pub owner: Option<OwnerName>,
    pub include_forks: bool,
    pub include_archived: bool,
    pub auto_sync: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SourceChanges {
    pub include_forks: Option<bool>,
    pub include_archived: Option<bool>,
    pub auto_sync: Option<bool>,
}
