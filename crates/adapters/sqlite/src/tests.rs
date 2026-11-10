use chrono::{DateTime, Utc};
use git_opt_domain::{model::*, ports::*, values::*};

use crate::SqliteStore;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}

fn sha(c: char) -> String {
    c.to_string().repeat(40)
}

fn new_repo(slug: &str) -> NewRepository {
    NewRepository {
        slug: RepoSlug::parse(slug).unwrap(),
        source_id: None,
        mode: RepoMode::Direct,
        clone_url: CloneUrl::new(format!("https://github.com/{slug}.git")).unwrap(),
        default_branch: BranchName::new("main").unwrap(),
        auto_pr: true,
    }
}

#[tokio::test]
async fn repositories_round_trip_with_targeted_updates() {
    let db = SqliteStore::in_memory().await.unwrap();
    let (id, inserted) = RepositoryStore::insert(&db, &new_repo("Acme/Site"), now()).await.unwrap();
    assert!(inserted);
    let (again, inserted) = RepositoryStore::insert(&db, &new_repo("acme/site"), now()).await.unwrap();
    assert_eq!((again, inserted), (id, false), "case-insensitive like GitHub");

    let changes = RepositoryChanges {
        enabled: Some(false),
        poll_interval_minutes: Some(Some(60)),
        ignore_rules: Some(IgnoreRules::new("docs/").unwrap()),
        ..Default::default()
    };
    RepositoryStore::update(&db, id, &changes).await.unwrap();
    db.record_check(
        id,
        &CheckRecord {
            status: RepoStatus::Ok,
            error: None,
            seen: Some(CommitSha::new(sha('a')).unwrap()),
            checked_at: now(),
            next_check_at: now(),
        },
    )
    .await
    .unwrap();
    let pr = PullRequest { number: PrNumber::new(7), url: "https://x/7".into(), state: PrState::Open };
    db.set_pull_request(id, &pr).await.unwrap();
    db.add_bytes_saved(id, ByteSize::new(100)).await.unwrap();

    let repo = RepositoryStore::get(&db, id).await.unwrap().unwrap();
    assert_eq!(repo.slug.to_string(), "Acme/Site");
    assert!(!repo.enabled);
    assert_eq!(repo.poll_interval_minutes, Some(60));
    assert_eq!(repo.ignore_rules.as_str(), "docs/");
    assert_eq!(repo.last_seen.as_ref().map(CommitSha::as_str), Some(sha('a').as_str()));
    assert_eq!(repo.open_pull_request(), Some(&pr));
    assert_eq!(repo.bytes_saved, ByteSize::new(100));
    assert!(db.due(now()).await.unwrap().is_empty(), "disabled repositories are never due");

    let fork = Fork {
        slug: RepoSlug::parse("jdev/site").unwrap(),
        clone_url: CloneUrl::new("https://github.com/jdev/site.git").unwrap(),
    };
    db.set_fork(id, Some(&fork)).await.unwrap();
    assert_eq!(RepositoryStore::get(&db, id).await.unwrap().unwrap().fork, Some(fork));
    db.set_fork(id, None).await.unwrap();
    assert_eq!(RepositoryStore::get(&db, id).await.unwrap().unwrap().fork, None);

    let totals = db.totals().await.unwrap();
    assert_eq!((totals.repos, totals.repos_enabled, totals.prs_open), (1, 0, 1));
}

#[tokio::test]
async fn runs_files_and_rejections() {
    let db = SqliteStore::in_memory().await.unwrap();
    let (repo, _) = RepositoryStore::insert(&db, &new_repo("acme/site"), now()).await.unwrap();
    let change = FileChange {
        path: RepoPath::new("img/a.png").unwrap(),
        optimizer: OptimizerId::new("oxipng").unwrap(),
        original_blob: BlobId::new(sha('b')).unwrap(),
        savings: Savings::new(ByteSize::new(1000), ByteSize::new(400)),
    };
    let outcome = RunOutcome {
        commit: Some(CommitSha::new(sha('c')).unwrap()),
        files_optimized: 1,
        savings: change.savings,
        pull_request: Some((PrNumber::new(3), "https://x/3".into())),
        ..RunOutcome::new(RunStatus::PrOpened)
    };
    let run = db.record(repo, RunTrigger::Manual, now(), now(), &outcome, std::slice::from_ref(&change)).await.unwrap();

    let listed = RunStore::list(&db, Some(repo), 10).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].repo.as_ref().unwrap().to_string(), "acme/site");
    assert_eq!(listed[0].outcome, outcome);
    assert_eq!(db.files(run).await.unwrap()[0].change, change);
    assert_eq!(db.latest_for_pull_request(repo, PrNumber::new(3)).await.unwrap().unwrap().id, run);
    assert_eq!(db.by_commit(repo, &CommitSha::new(sha('c')).unwrap()).await.unwrap().unwrap().id, run);

    db.reject_run(repo, run).await.unwrap();
    assert!(db.rejected(repo).await.unwrap().contains(&change.original_blob));
    db.forget(repo).await.unwrap();
    assert!(db.rejected(repo).await.unwrap().is_empty());
}

#[tokio::test]
async fn verdicts_settings_and_token() {
    let db = SqliteStore::in_memory().await.unwrap();
    let blob = BlobId::new(sha('d')).unwrap();
    let profile = Profile::new("oxipng10:l3");
    assert!(VerdictCache::get(&db, &blob, &profile).await.unwrap().is_none());
    let verdict = BlobVerdict::Failed { reason: "pixels differ".into() };
    VerdictCache::put(&db, &blob, &profile, &verdict, Savings::default()).await.unwrap();
    assert_eq!(VerdictCache::get(&db, &blob, &profile).await.unwrap(), Some(verdict));

    assert_eq!(SettingsStore::load(&db).await.unwrap(), Settings::default());
    let settings = Settings { png_level: 5, branch: BranchName::new("bot/images").unwrap(), ..Settings::default() };
    SettingsStore::save(&db, &settings).await.unwrap();
    assert_eq!(SettingsStore::load(&db).await.unwrap(), settings);

    assert!(db.token().await.unwrap().is_none());
    db.set_token(Some(&AccessToken::new("ghp_x").unwrap())).await.unwrap();
    assert_eq!(db.token().await.unwrap().unwrap().expose(), "ghp_x");
    db.set_token(None).await.unwrap();
    assert!(db.token().await.unwrap().is_none());
}

#[tokio::test]
async fn deleting_a_source_can_take_its_repositories() {
    let db = SqliteStore::in_memory().await.unwrap();
    let source = NewSource {
        kind: SourceKind::Org,
        owner: Some(OwnerName::new("acme").unwrap()),
        include_forks: false,
        include_archived: false,
        auto_sync: true,
    };
    let id = SourceStore::insert(&db, &source, now()).await.unwrap();
    let repo = NewRepository { source_id: Some(id), ..new_repo("acme/site") };
    RepositoryStore::insert(&db, &repo, now()).await.unwrap();
    assert_eq!(db.due_for_sync(now()).await.unwrap(), vec![id]);
    db.mark_synced(id, now(), Some(1), None).await.unwrap();
    assert_eq!(SourceStore::get(&db, id).await.unwrap().unwrap().repo_count, 1);

    let removed = SourceStore::delete(&db, id, true).await.unwrap();
    assert_eq!(removed.len(), 1);
    assert!(RepositoryStore::list(&db).await.unwrap().is_empty());
}
