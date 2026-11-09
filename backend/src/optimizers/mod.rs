//! Catalog of the format strategies compiled into `git-opt-codecs`, with their
//! availability on this machine.

use std::{path::Path, sync::Arc};

use git_opt_domain::{ports::FormatStrategy, values::FileFormat};
pub use git_opt_domain::ports::Outcome;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::settings::Settings;

#[derive(Debug, Clone, Serialize)]
pub struct OptimizerStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub extensions: Vec<&'static str>,
    pub enabled: bool,
    pub available: bool,
    pub detail: String,
}

pub struct Registry {
    optimizers: Vec<Arc<dyn FormatStrategy>>,
    /// Availability per optimizer, refreshed by [`Registry::refresh`].
    available: RwLock<Vec<(bool, String)>>,
}

impl Registry {
    pub async fn new() -> Arc<Self> {
        let registry =
            Arc::new(Self { available: RwLock::new(Vec::new()), optimizers: git_opt_codecs::default_strategies() });
        registry.refresh().await;
        registry
    }

    pub async fn refresh(&self) {
        let mut probes = Vec::with_capacity(self.optimizers.len());
        for o in &self.optimizers {
            probes.push(match o.probe().await {
                Ok(detail) => (true, detail),
                Err(reason) => (false, reason),
            });
        }
        *self.available.write().await = probes;
    }

    pub async fn statuses(&self, settings: &Settings) -> Vec<OptimizerStatus> {
        let available = self.available.read().await;
        self.optimizers
            .iter()
            .zip(available.iter())
            .map(|(o, (ok, detail))| OptimizerStatus {
                id: o.id(),
                label: o.label(),
                category: "image",
                extensions: o.formats().iter().flat_map(|f| f.extensions().iter().copied()).collect(),
                enabled: settings.optimizer_enabled(o.id(), o.enabled_by_default()),
                available: *ok,
                detail: detail.clone(),
            })
            .collect()
    }

    /// Optimizers that are enabled and runnable, for building a per-run plan.
    pub async fn active(&self, settings: &Settings) -> Vec<Arc<dyn FormatStrategy>> {
        let available = self.available.read().await;
        self.optimizers
            .iter()
            .zip(available.iter())
            .filter(|(o, (ok, _))| *ok && settings.optimizer_enabled(o.id(), o.enabled_by_default()))
            .map(|(o, _)| o.clone())
            .collect()
    }
}

/// Picks the strategy for a path among `active` ones, by file format.
pub fn for_path<'a>(active: &'a [Arc<dyn FormatStrategy>], path: &str) -> Option<&'a Arc<dyn FormatStrategy>> {
    let format = FileFormat::from_extension(Path::new(path).extension()?.to_str()?)?;
    active.iter().find(|o| o.handles(format))
}

/// Optimizes `input` with the domain's attempt policy (threshold + proof).
pub async fn process(opt: &dyn FormatStrategy, input: Arc<Vec<u8>>, settings: &Settings) -> Outcome {
    git_opt_domain::ports::attempt(opt, input, &settings.codec_options()).await
}
