//! In-memory job queue and live progress, shared by the scheduler and the API.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Mutex,
};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::Notify;

use crate::models::RunTrigger;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Check { repo_id: i64, trigger: RunTrigger, force: bool },
    Publish { repo_id: i64 },
    SyncSource { source_id: i64 },
}

impl Job {
    /// Jobs with the same key never run concurrently nor queue twice.
    fn key(&self) -> JobKey {
        match self {
            Self::Check { repo_id, .. } | Self::Publish { repo_id } => JobKey::Repo(*repo_id),
            Self::SyncSource { source_id } => JobKey::Source(*source_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKey {
    Repo(i64),
    Source(i64),
}

#[derive(Debug, Clone, Serialize)]
pub struct RunningJob {
    pub repo_id: i64,
    pub full_name: String,
    pub phase: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Default)]
struct Inner {
    queue: VecDeque<Job>,
    queued: HashSet<JobKey>,
    running: HashSet<JobKey>,
    progress: HashMap<i64, RunningJob>,
}

#[derive(Default)]
pub struct Jobs {
    inner: Mutex<Inner>,
    pub(crate) notify: Notify,
}

impl Jobs {
    /// Queues a job unless an equivalent one is already queued. A manual or
    /// forced request upgrades a queued scheduled check. Returns false if it
    /// was a duplicate.
    pub fn enqueue(&self, job: Job) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let key = job.key();
        if inner.queued.contains(&key) {
            if let Job::Check { trigger: RunTrigger::Manual, .. } | Job::Publish { .. } = job
                && let Some(slot) = inner.queue.iter_mut().find(|j| j.key() == key)
            {
                *slot = job;
            }
            return false;
        }
        // Manual requests jump ahead of the scheduled backlog.
        let urgent = !matches!(job, Job::Check { trigger: RunTrigger::Schedule, .. } | Job::SyncSource { .. });
        inner.queued.insert(key);
        if urgent {
            inner.queue.push_front(job);
        } else {
            inner.queue.push_back(job);
        }
        drop(inner);
        self.notify.notify_one();
        true
    }

    /// Pops the first job whose key is not already running.
    pub(crate) fn next(&self) -> Option<Job> {
        let mut inner = self.inner.lock().unwrap();
        let pos = inner.queue.iter().position(|j| !inner.running.contains(&j.key()))?;
        let job = inner.queue.remove(pos)?;
        let key = job.key();
        inner.queued.remove(&key);
        inner.running.insert(key);
        Some(job)
    }

    pub(crate) fn finished(&self, job: &Job) {
        let mut inner = self.inner.lock().unwrap();
        let key = job.key();
        inner.running.remove(&key);
        if let JobKey::Repo(id) = key {
            inner.progress.remove(&id);
        }
        drop(inner);
        self.notify.notify_one();
    }

    pub fn set_phase(&self, repo_id: i64, full_name: &str, phase: impl Into<String>) {
        let mut inner = self.inner.lock().unwrap();
        let phase = phase.into();
        inner
            .progress
            .entry(repo_id)
            .and_modify(|p| p.phase.clone_from(&phase))
            .or_insert_with(|| RunningJob {
                repo_id,
                full_name: full_name.to_string(),
                phase,
                started_at: Utc::now(),
            });
    }

    /// (running repo jobs with their phase, queued repo ids)
    pub fn snapshot(&self) -> (Vec<RunningJob>, Vec<i64>) {
        let inner = self.inner.lock().unwrap();
        let mut running: Vec<RunningJob> = inner.progress.values().cloned().collect();
        running.sort_by_key(|r| r.started_at);
        let queued = inner
            .queue
            .iter()
            .filter_map(|j| match j.key() {
                JobKey::Repo(id) => Some(id),
                JobKey::Source(_) => None,
            })
            .collect();
        (running, queued)
    }

    /// (running, queued) for one repository.
    pub fn repo_state(&self, repo_id: i64) -> (bool, bool, Option<String>) {
        let inner = self.inner.lock().unwrap();
        let key = JobKey::Repo(repo_id);
        (
            inner.running.contains(&key),
            inner.queued.contains(&key),
            inner.progress.get(&repo_id).map(|p| p.phase.clone()),
        )
    }

    pub fn is_queued_or_running(&self, key: JobKey) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.running.contains(&key) || inner.queued.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(repo_id: i64, trigger: RunTrigger) -> Job {
        Job::Check { repo_id, trigger, force: false }
    }

    #[test]
    fn deduplicates_and_serializes_per_repo() {
        let jobs = Jobs::default();
        assert!(jobs.enqueue(check(1, RunTrigger::Schedule)));
        assert!(!jobs.enqueue(check(1, RunTrigger::Schedule)));
        assert!(jobs.enqueue(check(2, RunTrigger::Schedule)));

        let first = jobs.next().unwrap();
        assert_eq!(first, check(1, RunTrigger::Schedule));
        // Repo 1 is running: a new request queues but is not handed out.
        assert!(jobs.enqueue(check(1, RunTrigger::Manual)));
        assert_eq!(jobs.next().unwrap(), check(2, RunTrigger::Schedule));
        assert!(jobs.next().is_none());

        jobs.finished(&first);
        assert_eq!(jobs.next().unwrap(), check(1, RunTrigger::Manual));
    }

    #[test]
    fn manual_requests_jump_the_queue_and_upgrade() {
        let jobs = Jobs::default();
        jobs.enqueue(check(1, RunTrigger::Schedule));
        jobs.enqueue(check(2, RunTrigger::Schedule));
        jobs.enqueue(check(3, RunTrigger::Manual));
        assert!(!jobs.enqueue(Job::Check { repo_id: 2, trigger: RunTrigger::Manual, force: true }));
        assert_eq!(jobs.next().unwrap(), check(3, RunTrigger::Manual));
        assert_eq!(jobs.next().unwrap(), check(1, RunTrigger::Schedule));
        assert_eq!(jobs.next().unwrap(), Job::Check { repo_id: 2, trigger: RunTrigger::Manual, force: true });
    }
}
