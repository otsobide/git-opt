//! Background loops: a ticker that queues due work and a dispatcher that runs
//! queued jobs with bounded, runtime-adjustable concurrency.

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use tokio::task::JoinSet;

use crate::{
    app::AppState,
    jobs::Job,
    models::RunTrigger,
    pipeline,
};

const TICK: Duration = Duration::from_secs(15);

pub fn spawn(app: AppState) {
    tokio::spawn(dispatcher(app.clone()));
    tokio::spawn(ticker(app));
}

async fn ticker(app: AppState) {
    let mut interval = tokio::time::interval(TICK);
    loop {
        interval.tick().await;
        if let Err(e) = tick(&app).await {
            tracing::warn!(error = %format!("{e:#}"), "scheduler tick failed");
        }
    }
}

/// Queues every repository whose next check is due and every source whose
/// periodic re-sync is due. Duplicates are ignored by the queue.
async fn tick(app: &AppState) -> Result<()> {
    for repo_id in app.db.due_repos(Utc::now()).await? {
        app.jobs.enqueue(Job::Check { repo_id, trigger: RunTrigger::Schedule, force: false });
    }
    let settings = app.db.settings().await?;
    let cutoff = Utc::now() - chrono::Duration::minutes(settings.source_sync_interval_minutes as i64);
    for source_id in app.db.due_sources(cutoff).await? {
        app.jobs.enqueue(Job::SyncSource { source_id });
    }
    Ok(())
}

async fn dispatcher(app: AppState) {
    let mut running = JoinSet::new();
    loop {
        // Re-read every loop so concurrency changes apply without a restart.
        let max = app.db.settings().await.map(|s| s.max_concurrent_jobs as usize).unwrap_or(2);
        while running.len() < max {
            let Some(job) = app.jobs.next() else { break };
            let app = app.clone();
            running.spawn(async move {
                // Run in its own task so a panic cannot leave the repo marked busy.
                let worker = tokio::spawn({
                    let app = app.clone();
                    let job = job.clone();
                    async move { run(&app, &job).await }
                });
                if let Err(e) = worker.await {
                    tracing::error!(?job, error = %e, "job panicked");
                }
                app.jobs.finished(&job);
            });
        }
        tokio::select! {
            _ = app.jobs.notify.notified() => {}
            Some(_) = running.join_next(), if !running.is_empty() => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run(app: &AppState, job: &Job) {
    let result = match *job {
        Job::Check { repo_id, trigger, force } => pipeline::check(app, repo_id, trigger, force).await,
        Job::Publish { repo_id } => pipeline::publish(app, repo_id).await,
        Job::SyncSource { source_id } => pipeline::sync_source(app, source_id).await,
    };
    if let Err(e) = result {
        tracing::warn!(?job, error = %format!("{e:#}"), "job failed");
    }
}
