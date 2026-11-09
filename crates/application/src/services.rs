//! The application context: every port, injected by the composition root.

use std::sync::Arc;

use git_opt_domain::{
    ports::{
        Clock, CodeForge, Random, RejectionStore, RepositoryStore, RunStore, SecretStore, SettingsStore, SourceStore,
        VerdictCache, VersionControl, Workspace,
    },
    values::{RepoId, RepoSlug},
};

use crate::{catalog::StrategyCatalog, jobs::JobQueue};

/// Ports the use cases run against. Build one with real adapters in the
/// server, or with in-memory ones in tests.
pub struct Ports {
    pub repositories: Arc<dyn RepositoryStore>,
    pub sources: Arc<dyn SourceStore>,
    pub runs: Arc<dyn RunStore>,
    pub settings: Arc<dyn SettingsStore>,
    pub secrets: Arc<dyn SecretStore>,
    pub verdicts: Arc<dyn VerdictCache>,
    pub rejections: Arc<dyn RejectionStore>,
    pub vcs: Arc<dyn VersionControl>,
    pub forge: Arc<dyn CodeForge>,
    pub clock: Arc<dyn Clock>,
    pub random: Arc<dyn Random>,
}

/// Use cases are methods of this type, one module per area (`use_cases/`).
pub struct Services {
    pub(crate) ports: Ports,
    pub(crate) catalog: StrategyCatalog,
    pub(crate) jobs: JobQueue,
}

impl Services {
    pub fn new(ports: Ports, catalog: StrategyCatalog) -> Arc<Self> {
        Arc::new(Self { ports, catalog, jobs: JobQueue::default() })
    }

    pub fn jobs(&self) -> &JobQueue {
        &self.jobs
    }

    pub fn catalog(&self) -> &StrategyCatalog {
        &self.catalog
    }

    /// What a running job is doing, for the UI.
    pub(crate) fn progress(&self, repo: RepoId, slug: &RepoSlug, phase: impl Into<String>) {
        self.jobs.set_phase(repo, slug, phase);
    }

    /// The bot's working copy, authenticated with the developer's token.
    pub(crate) async fn workspace(&self, slug: &RepoSlug) -> Box<dyn Workspace> {
        let token = self.ports.forge.credentials().await.map(|(token, _)| token);
        self.ports.vcs.workspace(slug, token)
    }
}
