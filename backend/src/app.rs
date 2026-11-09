//! Shared application state.

use std::sync::Arc;

use crate::{config::Config, db::Db, github::GitHub, jobs::Jobs, optimizers::Registry};

pub struct App {
    pub config: Config,
    pub db: Db,
    pub github: Arc<GitHub>,
    pub optimizers: Arc<Registry>,
    pub jobs: Jobs,
}

pub type AppState = Arc<App>;
