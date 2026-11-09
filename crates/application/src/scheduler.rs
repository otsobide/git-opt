//! Background loops: a ticker that queues due work and a dispatcher that runs
//! queued jobs with bounded concurrency (re-read on every loop, so changing it
//! in the UI applies immediately).

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use git_opt_domain::model::RunTrigger;
use tokio::task::JoinSet;

use crate::{Services, jobs::Job};

const TICK: Duration = Duration::from_secs(15);

/// Starts both loops on the current runtime.
pub fn spawn(services: Arc<Services>) {
    tokio::spawn(dispatcher(services.clone()));
    tokio::spawn(ticker(services));
}

async fn ticker(services: Arc<Services>) {
    let mut interval = tokio::time::interval(TICK);
    loop {
        interval.tick().await;
        if let Err(e) = queue_due_work(&services).await {
            tracing::warn!(error = %format!("{e:#}"), "scheduler tick failed");
        }
    }
}

/// Queues every repository whose next check is due and every source whose
/// re-sync is due. Duplicates are ignored by the queue.
pub async fn queue_due_work(services: &Services) -> Result<()> {
    let p = &services.ports;
    let now = p.clock.now();
    for repo in p.repositories.due(now).await? {
        services.jobs.enqueue(Job::Check { repo, trigger: RunTrigger::Schedule, force: false });
    }
    let settings = p.settings.load().await?;
    let cutoff = now - chrono::Duration::minutes(i64::from(settings.source_sync_interval_minutes));
    for source in p.sources.due_for_sync(cutoff).await? {
        services.jobs.enqueue(Job::SyncSource { source });
    }
    Ok(())
}

async fn dispatcher(services: Arc<Services>) {
    let mut running = JoinSet::new();
    loop {
        let max = services.ports.settings.load().await.map(|s| s.max_concurrent_jobs as usize).unwrap_or(2);
        while running.len() < max {
            let Some(job) = services.jobs.next() else { break };
            let services = services.clone();
            running.spawn(async move {
                // Its own task, so a panic cannot leave the repository marked busy.
                let worker = tokio::spawn({
                    let services = services.clone();
                    let job = job.clone();
                    async move { run(&services, &job).await }
                });
                if let Err(e) = worker.await {
                    tracing::error!(?job, error = %e, "job panicked");
                }
                services.jobs.finished(&job);
            });
        }
        tokio::select! {
            _ = services.jobs.notify.notified() => {}
            Some(_) = running.join_next(), if !running.is_empty() => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run(services: &Services, job: &Job) {
    let result = match *job {
        Job::Check { repo, trigger, force } => services.check_repository(repo, trigger, force).await,
        Job::Publish { repo } => services.publish_pending(repo).await,
        Job::SyncSource { source } => services.sync_source(source).await,
    };
    if let Err(e) = result {
        tracing::warn!(?job, error = %format!("{e:#}"), "job failed");
    }
}
