//! Who authors the bot's commits: a chain of sources, first match wins. It is
//! always the developer: their explicit setting, their git configuration, or
//! their forge account.

use async_trait::async_trait;
use git_opt_domain::{model::Settings, values::Identity};

use crate::Services;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorOrigin {
    Settings,
    Vcs,
    Forge,
}

impl AuthorOrigin {
    /// Stable names used by the API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Vcs => "git",
            Self::Forge => "github",
        }
    }
}

#[async_trait]
trait AuthorSource: Send + Sync {
    fn origin(&self) -> AuthorOrigin;
    async fn author(&self, services: &Services, settings: &Settings) -> Option<Identity>;
}

struct FromSettings;
struct FromVcsConfig;
struct FromForgeAccount;

#[async_trait]
impl AuthorSource for FromSettings {
    fn origin(&self) -> AuthorOrigin {
        AuthorOrigin::Settings
    }
    async fn author(&self, _: &Services, settings: &Settings) -> Option<Identity> {
        settings.author_override()
    }
}

#[async_trait]
impl AuthorSource for FromVcsConfig {
    fn origin(&self) -> AuthorOrigin {
        AuthorOrigin::Vcs
    }
    async fn author(&self, services: &Services, _: &Settings) -> Option<Identity> {
        services.ports.vcs.configured_identity().await
    }
}

#[async_trait]
impl AuthorSource for FromForgeAccount {
    fn origin(&self) -> AuthorOrigin {
        AuthorOrigin::Forge
    }
    async fn author(&self, services: &Services, _: &Settings) -> Option<Identity> {
        services.ports.forge.account().await.ok().map(|a| a.commit_identity)
    }
}

const CHAIN: [&dyn AuthorSource; 3] = [&FromSettings, &FromVcsConfig, &FromForgeAccount];

impl Services {
    pub async fn resolve_author(&self, settings: &Settings) -> anyhow::Result<(Identity, AuthorOrigin)> {
        for source in CHAIN {
            if let Some(identity) = source.author(self, settings).await {
                return Ok((identity, source.origin()));
            }
        }
        anyhow::bail!("cannot determine a commit author: set one in Settings, configure git or connect GitHub")
    }
}
