//! Composition root: reads the configuration, builds every adapter, plugs
//! them into the application's ports and starts the scheduler and the HTTP
//! server. Swapping an adapter (another database, forge or codec) happens
//! here and nowhere else.

mod config;
mod system;

use std::sync::Arc;

use anyhow::{Context, Result};
use git_opt_application::{Ports, Services, catalog::StrategyCatalog, scheduler};
use git_opt_domain::ports::SettingsStore;
use git_opt_git_cli::GitCli;
use git_opt_github::GitHub;
use git_opt_http::HttpConfig;
use git_opt_sqlite::SqliteStore;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("git_opt=info,tower_http=warn")),
        )
        .init();

    let config = config::Config::from_env()?;
    tracing::info!(bind = %config.bind, data_dir = %config.data_dir.display(), "starting git-opt");

    // Driven adapters.
    let store = Arc::new(SqliteStore::open(&config.db_path()).await?);
    let forge = Arc::new(GitHub::with_standard_tokens(config.github_api.clone(), store.clone())?);
    let vcs = Arc::new(GitCli::new(config.repos_dir()));
    let catalog = StrategyCatalog::new(git_opt_codecs::default_strategies()).await;
    for s in catalog.statuses(&store.load().await?).await {
        tracing::info!(optimizer = s.id, available = s.available, detail = %s.detail, "format strategy");
    }

    let services = Services::new(
        Ports {
            repositories: store.clone(),
            sources: store.clone(),
            runs: store.clone(),
            settings: store.clone(),
            secrets: store.clone(),
            verdicts: store.clone(),
            rejections: store.clone(),
            vcs,
            forge,
            clock: Arc::new(system::SystemClock),
            random: Arc::new(system::FastRandom),
        },
        catalog,
    );
    scheduler::spawn(services.clone());

    // Driving adapter.
    let router = git_opt_http::router(
        services,
        HttpConfig {
            admin_token: config.admin_token.clone(),
            static_dir: config.static_dir.clone(),
            version: env!("CARGO_PKG_VERSION"),
        },
    );
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot bind {}", config.bind))?;
    tracing::info!("listening on http://{}", config.bind);
    axum::serve(listener, router).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}
