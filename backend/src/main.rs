mod api;
mod app;
mod config;
mod db;
mod git;
mod github;
mod jobs;
mod models;
mod optimizers;
mod pipeline;
mod report;
mod scheduler;
mod settings;

use std::sync::Arc;

use anyhow::{Context, Result};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
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

    let db = db::Db::open(&config.db_path()).await?;
    let github = github::GitHub::new(config.github_api.clone(), db.clone())?;
    let optimizers = optimizers::Registry::new().await;
    for o in optimizers.statuses(&db.settings().await?).await {
        tracing::info!(optimizer = o.id, available = o.available, detail = %o.detail, "optimizer");
    }

    let app = Arc::new(app::App { config: config.clone(), db, github, optimizers, jobs: jobs::Jobs::default() });
    scheduler::spawn(app.clone());

    let mut router = api::router(app);
    match &config.static_dir {
        Some(dir) => {
            tracing::info!(dir = %dir.display(), "serving web UI");
            let spa = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
            router = router.fallback_service(spa);
        }
        None => tracing::warn!("web UI not found (build frontend/ or set GIT_OPT_STATIC_DIR); serving API only"),
    }
    let router = router.layer(TraceLayer::new_for_http());

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
