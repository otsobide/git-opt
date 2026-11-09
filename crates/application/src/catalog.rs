//! The format strategies available to the use cases, with their availability
//! on this machine (probed at startup and after settings change).

use std::sync::Arc;

use git_opt_domain::{
    model::Settings,
    ports::FormatStrategy,
    values::{FileFormat, OptimizerId},
};
use tokio::sync::RwLock;

/// A strategy as shown in the UI.
#[derive(Debug, Clone)]
pub struct StrategyStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub formats: &'static [FileFormat],
    pub enabled: bool,
    pub available: bool,
    pub detail: String,
}

pub struct StrategyCatalog {
    strategies: Vec<Arc<dyn FormatStrategy>>,
    availability: RwLock<Vec<Result<String, String>>>,
}

impl StrategyCatalog {
    pub async fn new(strategies: Vec<Arc<dyn FormatStrategy>>) -> Self {
        let catalog = Self { strategies, availability: RwLock::new(Vec::new()) };
        catalog.refresh().await;
        catalog
    }

    /// Probes every strategy again (e.g. after installing a tool).
    pub async fn refresh(&self) {
        let mut probes = Vec::with_capacity(self.strategies.len());
        for s in &self.strategies {
            probes.push(s.probe().await);
        }
        *self.availability.write().await = probes;
    }

    fn enabled(settings: &Settings, s: &dyn FormatStrategy) -> bool {
        settings.optimizers.get(s.id()).copied().unwrap_or(s.enabled_by_default())
    }

    pub async fn statuses(&self, settings: &Settings) -> Vec<StrategyStatus> {
        let availability = self.availability.read().await;
        self.strategies
            .iter()
            .zip(availability.iter())
            .map(|(s, probe)| StrategyStatus {
                id: s.id(),
                label: s.label(),
                formats: s.formats(),
                enabled: Self::enabled(settings, s.as_ref()),
                available: probe.is_ok(),
                detail: probe.clone().unwrap_or_else(|reason| reason),
            })
            .collect()
    }

    /// Enabled and runnable strategies.
    pub async fn active(&self, settings: &Settings) -> Vec<Arc<dyn FormatStrategy>> {
        let availability = self.availability.read().await;
        self.strategies
            .iter()
            .zip(availability.iter())
            .filter(|(s, probe)| probe.is_ok() && Self::enabled(settings, s.as_ref()))
            .map(|(s, _)| s.clone())
            .collect()
    }

    /// The pull request line of a strategy.
    pub fn explanation(&self, id: &OptimizerId) -> Option<String> {
        self.strategies.iter().find(|s| s.id() == id.as_str()).map(|s| s.explanation().to_string())
    }
}
