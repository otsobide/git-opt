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
    /// Head commits of pull requests that are no longer open.
    frozen_heads: Mutex<HashMap<PrNumber, CommitSha>>,
    /// Branch name -> commits it has that the upstream default branch lacks (0 when missing).
    ahead: Mutex<HashMap<String, u64>>,
    deleted: Mutex<Vec<RepoSlug>>,
    /// Refuse deletions, like a token without the delete_repo scope.
    deny_delete: Mutex<bool>,
}

impl FakeForge {
    fn add(&self, repo: ForgeRepository) {
        self.repos.lock().unwrap().insert(repo.slug.to_string().to_lowercase(), repo);
    }
    fn pulls(&self) -> Vec<(RepoSlug, PullRequestHead, ForgePullRequest)> {
        self.pulls.lock().unwrap().clone()
    }
    fn set_pull_state(&self, state: PrState) {
        let pulls: Vec<_> = self.pulls();
        for (slug, head, pr) in pulls {
            if state != PrState::Open
                && let Some(tip) = self.tip(&head_repo(&slug, &head), &head.branch)
            {
                self.frozen_heads.lock().unwrap().insert(pr.number, tip);
            }
        }
        for pr in self.pulls.lock().unwrap().iter_mut() {
            pr.2.state = state;
        }
    }
    /// The bare repository behind a forge repository.
    fn deleted(&self) -> Vec<RepoSlug> {
        self.deleted.lock().unwrap().clone()
    }
    fn bare(&self, slug: &RepoSlug) -> Option<PathBuf> {
        let repos = self.repos.lock().unwrap();
        let repo = repos.get(&slug.to_string().to_lowercase())?;
        Some(PathBuf::from(repo.clone_url.as_str().trim_start_matches("file://")))
    }
    fn tip(&self, slug: &RepoSlug, branch: &BranchName) -> Option<CommitSha> {
        let out = std::process::Command::new("git")
            .current_dir(self.bare(slug)?)
            .args(["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")])
            .output()
            .ok()?;
        CommitSha::new(String::from_utf8_lossy(&out.stdout).trim()).ok()
    }
}

/// The repository a pull request's branch lives in.
fn head_repo(slug: &RepoSlug, head: &PullRequestHead) -> RepoSlug {
    if head.cross_repository { RepoSlug::new(head.owner.clone(), slug.name().clone()) } else { slug.clone() }
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
    async fn branches(&self, slug: &RepoSlug) -> Result<Vec<ForgeBranch>> {
        let bare = self.bare(slug).ok_or_else(|| anyhow!("404"))?;
        let refs = git(&bare, &["for-each-ref", "--format=%(refname:short) %(objectname)", "refs/heads"]);
        refs.lines()
            .map(|line| {
                let (name, sha) = line.split_once(' ').unwrap();
                Ok(ForgeBranch { name: BranchName::new(name)?, head: CommitSha::new(sha)? })
            })
            .collect()
    }
    async fn pull_requests_from(&self, slug: &RepoSlug, head: &PullRequestHead) -> Result<Vec<BranchPullRequest>> {
        let matching: Vec<_> = self
            .pulls()
            .into_iter()
            .filter(|p| &p.0 == slug && p.1.owner == head.owner && p.1.branch == head.branch)
            .collect();
        let mut out = Vec::new();
        for (slug, head, pr) in matching {
            let frozen = self.frozen_heads.lock().unwrap().get(&pr.number).cloned();
            let tip = frozen.or_else(|| self.tip(&head_repo(&slug, &head), &head.branch));
            out.push(BranchPullRequest {
                number: pr.number,
                state: pr.state,
                head: tip.ok_or_else(|| anyhow!("gone"))?,
            });
        }
        Ok(out)
    }
    async fn commits_ahead(&self, _: &RepoSlug, _: &BranchName, head: &PullRequestHead) -> Result<u64> {
        Ok(self.ahead.lock().unwrap().get(head.branch.as_str()).copied().unwrap_or(0))
    }
    async fn delete_repository(&self, slug: &RepoSlug) -> Result<()> {
        if *self.deny_delete.lock().unwrap() {
            return Err(anyhow!("403: needs the delete_repo scope"));
        }
        self.repos.lock().unwrap().remove(&slug.to_string().to_lowercase()).ok_or_else(|| anyhow!("404"))?;
        self.deleted.lock().unwrap().push(slug.clone());
        Ok(())
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
        parent: None,
        read_only: false,
        can_push,
    }
}

/// The developer's fork of `upstream`, as the forge describes it.
fn fork_of(upstream: &str, slug: &str, bare: &Path) -> ForgeRepository {
    ForgeRepository { is_fork: true, parent: Some(RepoSlug::parse(upstream).unwrap()), ..forge_repo(slug, bare, true) }
}

/// A third-party repository in fork mode whose bot pull request was just merged.
async fn merged_contribution(tmp: &Path) -> (Arc<FakeForge>, Arc<Services>, RepoId, PathBuf) {
    let upstream = bare_repo(tmp, "lib");
    let fork_bare = tmp.join("fork.git");
    git(tmp, &["clone", "-q", "--bare", upstream.to_str().unwrap(), fork_bare.to_str().unwrap()]);
    let forge = Arc::new(FakeForge::default());
    forge.add(forge_repo("other/lib", &upstream, false));
    forge.forks.lock().unwrap().insert("other/lib".into(), fork_of("other/lib", "jdev/lib", &fork_bare));
    let app = services(tmp, forge.clone()).await;
    let repo = app.add_repository("other/lib", ModeRequest::Auto, true).await.unwrap().repository;
    assert_eq!(repo.mode, RepoMode::Fork);
    app.check_repository(repo.id, RunTrigger::Manual, false).await.unwrap();
    assert_eq!(forge.pulls().len(), 1);
    assert!(forge.deleted().is_empty(), "never while the pull request is open");
    forge.set_pull_state(PrState::Merged);
    (forge, app, repo.id, fork_bare)
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

#[tokio::test]
async fn merged_forks_are_deleted_once_nothing_else_lives_there() {
    let tmp = tempfile::tempdir().unwrap();
    let (forge, app, id, fork_bare) = merged_contribution(tmp.path()).await;

    // The developer has unmerged work on another branch of the fork.
    git(&fork_bare, &["branch", "wip", "main"]);
    forge.ahead.lock().unwrap().insert("wip".into(), 2);
    app.check_repository(id, RunTrigger::Manual, false).await.unwrap();
    assert!(forge.deleted().is_empty(), "unmerged commits keep the fork");

    // That work becomes an open pull request: still kept.
    let head = PullRequestHead {
        owner: OwnerName::new("jdev").unwrap(),
        branch: BranchName::new("wip").unwrap(),
        cross_repository: true,
    };
    let pr = ForgePullRequest { number: PrNumber::new(2), url: "u".into(), state: PrState::Open };
    forge.pulls.lock().unwrap().push((RepoSlug::parse("other/lib").unwrap(), head, pr));
    app.check_repository(id, RunTrigger::Manual, false).await.unwrap();
    assert!(forge.deleted().is_empty(), "an open pull request keeps the fork");

    // Merged as well: the fork goes, once.
    forge.set_pull_state(PrState::Merged);
    app.check_repository(id, RunTrigger::Schedule, false).await.unwrap();
    assert_eq!(forge.deleted(), vec![RepoSlug::parse("jdev/lib").unwrap()]);
    let runs = app.runs(Some(id), 10).await.unwrap();
    assert_eq!(runs[0].outcome.status, RunStatus::ForkDeleted);
    assert!(runs[0].outcome.message.as_deref().unwrap().contains("jdev/lib"));
    assert!(app.repository(id).await.unwrap().repository.fork.is_none());
    app.check_repository(id, RunTrigger::Schedule, false).await.unwrap();
    assert_eq!((forge.deleted().len(), app.runs(Some(id), 10).await.unwrap().len()), (1, runs.len()));
}

#[tokio::test]
async fn forks_stay_when_the_setting_is_off_or_in_direct_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let (forge, app, id, _) = merged_contribution(tmp.path()).await;

    let settings = app.settings().await.unwrap();
    app.update_settings(Settings { delete_merged_forks: false, ..settings.clone() }).await.unwrap();
    app.check_repository(id, RunTrigger::Manual, false).await.unwrap();
    assert!(forge.deleted().is_empty(), "setting off");

    app.update_settings(settings).await.unwrap();
    let direct = RepositoryChanges { mode: Some(RepoMode::Direct), ..Default::default() };
    app.update_repository(id, direct).await.unwrap();
    app.check_repository(id, RunTrigger::Manual, false).await.unwrap();
    assert!(forge.deleted().is_empty(), "direct mode never deletes anything");
}

#[tokio::test]
async fn a_failed_deletion_is_reported_once() {
    let tmp = tempfile::tempdir().unwrap();
    let (forge, app, id, _) = merged_contribution(tmp.path()).await;
    *forge.deny_delete.lock().unwrap() = true;

    app.check_repository(id, RunTrigger::Manual, false).await.unwrap();
    let runs = app.runs(Some(id), 10).await.unwrap();
    assert_eq!(runs[0].outcome.status, RunStatus::Error);
    assert!(runs[0].outcome.message.as_deref().unwrap().contains("delete_repo"));
    let repo = app.repository(id).await.unwrap().repository;
    assert_eq!(repo.status, RepoStatus::Ok, "the check itself succeeded");
    app.check_repository(id, RunTrigger::Schedule, false).await.unwrap();
    assert_eq!(app.runs(Some(id), 10).await.unwrap().len(), runs.len(), "no duplicate rows");

    *forge.deny_delete.lock().unwrap() = false;
    app.check_repository(id, RunTrigger::Schedule, false).await.unwrap();
    assert_eq!(app.runs(Some(id), 1).await.unwrap()[0].outcome.status, RunStatus::ForkDeleted);
}
