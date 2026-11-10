//! What the dashboard shows about this installation.

use git_opt_domain::{ports::TokenOrigin, ports::Totals, values::Identity};

use crate::{AppResult, Services, authorship::AuthorOrigin, catalog::StrategyStatus, jobs::RunningJob};

#[derive(Debug, Clone)]
pub struct ForgeStatus {
    pub token_origin: Option<TokenOrigin>,
    /// `Ok((login, display name))`, or why the forge cannot be used.
    pub account: Result<(String, Option<String>), String>,
}

#[derive(Debug, Clone)]
pub struct VcsStatus {
    pub version: Result<String, String>,
    pub author: Result<(Identity, AuthorOrigin), String>,
}

#[derive(Debug, Clone)]
pub struct Status {
    pub forge: ForgeStatus,
    pub vcs: VcsStatus,
    pub strategies: Vec<StrategyStatus>,
    pub running: Vec<RunningJob>,
    pub queued: Vec<git_opt_domain::values::RepoId>,
    pub totals: Totals,
}

impl Services {
    pub async fn status(&self) -> AppResult<Status> {
        let p = &self.ports;
        let settings = p.settings.load().await?;
        let forge = match p.forge.credentials().await {
            None => ForgeStatus {
                token_origin: None,
                account: Err("No GitHub token: set one in Settings, export GITHUB_TOKEN or run `gh auth login`.".into()),
            },
            Some((_, origin)) => ForgeStatus {
                token_origin: Some(origin),
                account: p
                    .forge
                    .account()
                    .await
                    .map(|a| (a.login.to_string(), a.display_name))
                    .map_err(|e| format!("{e:#}")),
            },
        };
        let vcs = VcsStatus {
            version: p.vcs.version().await.map_err(|e| format!("{e:#}")),
            author: self.resolve_author(&settings).await.map_err(|e| format!("{e:#}")),
        };
        let (running, queued) = self.jobs.snapshot();
        Ok(Status {
            forge,
            vcs,
            strategies: self.catalog.statuses(&settings).await,
            running,
            queued,
            totals: p.repositories.totals().await?,
        })
    }
}
