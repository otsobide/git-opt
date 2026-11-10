//! Use cases end to end, without network or codecs: real SQLite (in memory)
//! and real git (local repositories), with an in-memory forge, a fixed clock
//! and a fake format strategy.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use git_opt_domain::{model::*, ports::*, report::BOT_MARKER, values::*};
use git_opt_git_cli::GitCli;
use git_opt_sqlite::SqliteStore;

use crate::{Ports, Services, catalog::StrategyCatalog, use_cases::repositories::ModeRequest};

// ------------------------------------------------------------------ doubles

/// "Optimizes" PNG files by trimming trailing zero bytes.
struct TrimPadding;

#[async_trait]
impl FormatStrategy for TrimPadding {
    fn id(&self) -> &'static str {
        "trim"
    }
    fn label(&self) -> &'static str {
        "Trim padding"
    }
    fn formats(&self) -> &'static [FileFormat] {
        &[FileFormat::Png]
    }
    fn explanation(&self) -> &'static str {
        "**Test**: trailing padding removed."
    }
    fn profile(&self, _: &CodecOptions) -> Profile {
        Profile::new("trim:v1")
    }
    async fn probe(&self) -> Result<String, String> {
        Ok("test".into())
    }
    async fn optimize(&self, input: Arc<Vec<u8>>, _: &CodecOptions) -> Result<Vec<u8>> {
        let end = input.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
        Ok(input[..end].to_vec())
    }
    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        anyhow::ensure!(original.starts_with(&candidate) && original[candidate.len()..].iter().all(|b| *b == 0));
        Ok(())
    }
}

struct FixedClock(DateTime<Utc>);
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct NoJitter;
impl Random for NoJitter {
    fn up_to(&self, _: u64) -> u64 {
        0
    }
}

#[derive(Default)]
struct FakeForge {
    repos: Mutex<HashMap<String, ForgeRepository>>,
    /// upstream slug -> fork to create
    forks: Mutex<HashMap<String, ForgeRepository>>,
    pulls: Mutex<Vec<(RepoSlug, PullRequestHead, ForgePullRequest)>>,
}

impl FakeForge {
    fn add(&self, repo: ForgeRepository) {
        self.repos.lock().unwrap().insert(repo.slug.to_string().to_lowercase(), repo);
    }
    fn pulls(&self) -> Vec<(RepoSlug, PullRequestHead, ForgePullRequest)> {
        self.pulls.lock().unwrap().clone()
    }
    fn set_pull_state(&self, state: PrState) {
        for pr in self.pulls.lock().unwrap().iter_mut() {
            pr.2.state = state;
        }
    }
}

#[async_trait]
impl CodeForge for FakeForge {
    async fn credentials(&self) -> Option<(AccessToken, TokenOrigin)> {
        Some((AccessToken::new("fake").unwrap(), TokenOrigin::Environment))
    }
    async fn forget_credentials(&self) {}
    async fn account(&self) -> Result<ForgeAccount> {
        Ok(ForgeAccount {
            login: OwnerName::new("jdev")?,
            display_name: Some("J Dev".into()),
            commit_identity: Identity::new("J Dev", "42+jdev@users.noreply.github.com")?,
        })
    }
    async fn repository(&self, slug: &RepoSlug) -> Result<ForgeRepository> {
        self.repos.lock().unwrap().get(&slug.to_string().to_lowercase()).cloned().ok_or_else(|| anyhow!("404"))
    }
    async fn list_repositories(&self, _: SourceKind, owner: Option<&OwnerName>) -> Result<Vec<ForgeRepository>> {
        let repos = self.repos.lock().unwrap();
        Ok(repos.values().filter(|r| owner.is_none_or(|o| r.slug.owner().same_as(o.as_str()))).cloned().collect())
    }
    async fn fork(&self, slug: &RepoSlug) -> Result<ForgeRepository> {
        let fork = self.forks.lock().unwrap().get(&slug.to_string()).cloned().ok_or_else(|| anyhow!("no fork"))?;
        self.add(fork.clone());
        Ok(fork)
    }
    async fn pull_request(&self, slug: &RepoSlug, number: PrNumber) -> Result<ForgePullRequest> {
        let pulls = self.pulls.lock().unwrap();
        pulls.iter().find(|p| &p.0 == slug && p.2.number == number).map(|p| p.2.clone()).ok_or_else(|| anyhow!("404"))
    }
    async fn find_open_pull_request(
        &self,
        slug: &RepoSlug,
        head: &PullRequestHead,
    ) -> Result<Option<ForgePullRequest>> {
        let pulls = self.pulls.lock().unwrap();
        Ok(pulls
            .iter()
            .find(|p| {
                &p.0 == slug && p.1.owner == head.owner && p.1.branch == head.branch && p.2.state == PrState::Open
            })
            .map(|p| p.2.clone()))
    }
    async fn open_pull_request(
        &self,
        slug: &RepoSlug,
        head: &PullRequestHead,
        _: &BranchName,
        _: &str,
        body: &str,
    ) -> Result<ForgePullRequest> {
        assert!(body.contains(BOT_MARKER) && body.contains("**Test**: trailing padding removed."));
        let mut pulls = self.pulls.lock().unwrap();
        let number = PrNumber::new(pulls.len() as i64 + 1);
        let pr =
            ForgePullRequest { number, url: format!("https://example.com/{slug}/pull/{number}"), state: PrState::Open };
        pulls.push((slug.clone(), head.clone(), pr.clone()));
        Ok(pr)
    }
    async fn update_pull_request(
        &self,
        slug: &RepoSlug,
        number: PrNumber,
        _: &str,
        _: &str,
    ) -> Result<ForgePullRequest> {
        self.pull_request(slug, number).await
    }
}

// ------------------------------------------------------------------- setup

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Owner", "-c", "user.email=owner@example.com", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A bare repository with one padded "image" and a README.
fn bare_repo(tmp: &Path, name: &str) -> PathBuf {
    let src = tmp.join(format!("{name}-src"));
    std::fs::create_dir_all(src.join("img")).unwrap();
    let mut image = b"PIXELS".to_vec();
    image.resize(4000, 0);
    std::fs::write(src.join("img/logo.png"), image).unwrap();
    std::fs::write(src.join("README.md"), "# demo\n").unwrap();
    git(&src, &["init", "-q"]);
    git(&src, &["add", "-A"]);
    git(&src, &["commit", "-qm", "init"]);
    let bare = tmp.join(format!("{name}.git"));
    git(tmp, &["clone", "-q", "--bare", src.to_str().unwrap(), bare.to_str().unwrap()]);
    bare
}

fn forge_repo(slug: &str, bare: &Path, can_push: bool) -> ForgeRepository {
    ForgeRepository {
        slug: RepoSlug::parse(slug).unwrap(),
        clone_url: CloneUrl::new(format!("file://{}", bare.display())).unwrap(),
        default_branch: BranchName::new("main").unwrap(),
        is_fork: false,
        read_only: false,
        can_push,
    }
}

async fn services(tmp: &Path, forge: Arc<FakeForge>) -> Arc<Services> {
    let db = Arc::new(SqliteStore::in_memory().await.unwrap());
    let settings = Settings {
        commit_author_name: "Jane Developer".into(),
        commit_author_email: "jane@example.com".into(),
        ..Settings::default()
    };
    SettingsStore::save(db.as_ref(), &settings).await.unwrap();
    let ports = Ports {
        repositories: db.clone(),
        sources: db.clone(),
        runs: db.clone(),
        settings: db.clone(),
        secrets: db.clone(),
        verdicts: db.clone(),
        rejections: db.clone(),
        vcs: Arc::new(GitCli::new(tmp.join("work"))),
        forge,
        clock: Arc::new(FixedClock(DateTime::from_timestamp(1_800_000_000, 0).unwrap())),
        random: Arc::new(NoJitter),
    };
    Services::new(ports, StrategyCatalog::new(vec![Arc::new(TrimPadding)]).await)
}

// ------------------------------------------------------------------- tests

#[tokio::test]
async fn check_opens_a_pull_request_then_learns_from_the_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let bare = bare_repo(tmp.path(), "demo");
    let forge = Arc::new(FakeForge::default());
    forge.add(forge_repo("octo/demo", &bare, true));
    let app = services(tmp.path(), forge.clone()).await;

    let repo = app.add_repository("https://github.com/octo/demo", ModeRequest::Auto, true).await.unwrap().repository;
    assert_eq!(repo.mode, RepoMode::Direct);
    app.check_repository(repo.id, RunTrigger::Manual, false).await.unwrap();

    let runs = app.runs(Some(repo.id), 10).await.unwrap();
    assert_eq!(runs[0].outcome.status, RunStatus::PrOpened, "{:?}", runs[0].outcome.message);
    assert_eq!(runs[0].outcome.savings, Savings::new(ByteSize::new(4000), ByteSize::new(6)));
    let pulls = forge.pulls();
    assert_eq!(pulls.len(), 1);
    assert!(!pulls[0].1.cross_repository);
    let branch = "git-opt/optimize-images";
    assert_eq!(git(&bare, &["log", "-1", "--format=%an <%ae>", branch]), "Jane Developer <jane@example.com>");
    assert!(git(&bare, &["log", "-1", "--format=%B", branch]).contains(BOT_MARKER));

    // A scheduled poll with nothing new leaves no trace.
    app.check_repository(repo.id, RunTrigger::Schedule, false).await.unwrap();
    assert_eq!(app.runs(Some(repo.id), 10).await.unwrap().len(), 1);

    // The maintainer merges: nothing is left to do and the savings are credited.
    git(&bare, &["update-ref", "refs/heads/main", branch]);
    forge.set_pull_state(PrState::Merged);
    app.check_repository(repo.id, RunTrigger::Manual, false).await.unwrap();
    let runs = app.runs(Some(repo.id), 10).await.unwrap();
    assert_eq!(runs[0].outcome.status, RunStatus::UpToDate);
    let repo = app.repository(repo.id).await.unwrap().repository;
    assert_eq!(repo.bytes_saved, ByteSize::new(3994));
    assert_eq!(repo.pull_request.unwrap().state, PrState::Merged);
}

#[tokio::test]
async fn third_party_repositories_go_through_a_fork_after_approval() {
    let tmp = tempfile::tempdir().unwrap();
    let upstream = bare_repo(tmp.path(), "lib");
    let fork_bare = tmp.path().join("fork.git");
    git(tmp.path(), &["init", "-q", "--bare", fork_bare.to_str().unwrap()]);
    let forge = Arc::new(FakeForge::default());
    forge.add(forge_repo("other/lib", &upstream, false));
    forge.forks.lock().unwrap().insert("other/lib".into(), forge_repo("jdev/lib", &fork_bare, true));
    let app = services(tmp.path(), forge.clone()).await;

    let repo = app.add_repository("other/lib", ModeRequest::Auto, false).await.unwrap().repository;
    assert_eq!(repo.mode, RepoMode::Fork);
    app.check_repository(repo.id, RunTrigger::Manual, false).await.unwrap();
    let waiting = app.repository(repo.id).await.unwrap().repository;
    assert_eq!(waiting.status, RepoStatus::AwaitingApproval);
    assert!(waiting.pending_commit.is_some() && forge.pulls().is_empty(), "nothing leaves before approval");

    app.publish_pending(repo.id).await.unwrap();
    let pulls = forge.pulls();
    assert_eq!(pulls.len(), 1);
    assert!(pulls[0].1.cross_repository);
    assert_eq!(pulls[0].1.owner.as_str(), "jdev");
    assert!(!git(&fork_bare, &["rev-parse", "git-opt/optimize-images"]).is_empty());
    let published = app.repository(repo.id).await.unwrap().repository;
    assert_eq!((published.status, published.pending_commit), (RepoStatus::Ok, None));
}

#[tokio::test]
async fn closed_pull_requests_are_never_proposed_again() {
    let tmp = tempfile::tempdir().unwrap();
    let bare = bare_repo(tmp.path(), "demo");
    let forge = Arc::new(FakeForge::default());
    forge.add(forge_repo("octo/demo", &bare, true));
    let app = services(tmp.path(), forge.clone()).await;
    let repo = app.add_repository("octo/demo", ModeRequest::Auto, true).await.unwrap().repository;
    app.check_repository(repo.id, RunTrigger::Manual, false).await.unwrap();

    forge.set_pull_state(PrState::Closed);
    app.check_repository(repo.id, RunTrigger::Manual, true).await.unwrap();
    assert_eq!(app.runs(Some(repo.id), 1).await.unwrap()[0].outcome.status, RunStatus::UpToDate);

    app.forget_rejections(repo.id).await.unwrap();
    app.check_repository(repo.id, RunTrigger::Manual, true).await.unwrap();
    assert_eq!(app.runs(Some(repo.id), 1).await.unwrap()[0].outcome.status, RunStatus::PrOpened);
}

#[tokio::test]
async fn adding_twice_or_forking_your_own_repository_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let bare = bare_repo(tmp.path(), "mine");
    let forge = Arc::new(FakeForge::default());
    forge.add(forge_repo("jdev/mine", &bare, true));
    let app = services(tmp.path(), forge).await;

    let fork_own = app.add_repository("jdev/mine", ModeRequest::Fork, true).await;
    assert!(matches!(fork_own, Err(crate::AppError::Invalid(_))));
    app.add_repository("jdev/mine", ModeRequest::Auto, true).await.unwrap();
    let again = app.add_repository("JDEV/mine", ModeRequest::Auto, true).await;
    assert!(matches!(again, Err(crate::AppError::Conflict(_))));
    assert!(matches!(
        app.add_repository("not a repo", ModeRequest::Auto, true).await,
        Err(crate::AppError::Invalid(_))
    ));
}
