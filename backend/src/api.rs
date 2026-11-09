//! JSON API consumed by the Vue frontend. Messages are user-facing (English).

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post, put},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    app::{App, AppState},
    db::TOKEN_KEY,
    git,
    jobs::{Job, JobKey},
    models::{Repo, RepoPatch, RunTrigger, SourceKind},
    pipeline::{self, ModeRequest},
    settings::Settings,
};

// ------------------------------------------------------------------- errors

pub struct ApiError(StatusCode, String);

impl ApiError {
    fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        Self(status, msg.into())
    }
    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }
    fn bad_request(e: anyhow::Error) -> Self {
        Self::new(StatusCode::BAD_REQUEST, format!("{e:#}"))
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        let e = e.into();
        tracing::error!(error = %format!("{e:#}"), "api error");
        Self(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ------------------------------------------------------------------- router

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/status", get(status))
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/settings/github-token", put(put_token))
        .route("/api/repos", get(list_repos).post(create_repo))
        .route("/api/repos/check-all", post(check_all))
        .route("/api/repos/{id}", get(get_repo).patch(patch_repo).delete(delete_repo))
        .route("/api/repos/{id}/check", post(check_repo))
        .route("/api/repos/{id}/publish", post(publish_repo))
        .route("/api/repos/{id}/reset-ignored", post(reset_ignored))
        .route("/api/repos/{id}/runs", get(repo_runs))
        .route("/api/runs", get(list_runs))
        .route("/api/runs/{id}", get(get_run))
        .route("/api/sources", get(list_sources).post(create_source))
        .route("/api/sources/{id}", axum::routing::patch(patch_source).delete(delete_source))
        .route("/api/sources/{id}/sync", post(sync_source))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/auth", get(auth_info))
        .merge(protected)
        .route("/api/{*rest}", any(|| async { ApiError::not_found() }))
        .with_state(state)
}

async fn require_token(State(app): State<AppState>, headers: HeaderMap, req: Request, next: Next) -> Response {
    if let Some(expected) = &app.config.admin_token {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !constant_time_eq(given.as_bytes(), expected.as_bytes()) {
            return ApiError::new(StatusCode::UNAUTHORIZED, "admin token required").into_response();
        }
    }
    next.run(req).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn auth_info(State(app): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "required": app.config.admin_token.is_some() }))
}

// ------------------------------------------------------------------- status

async fn status(State(app): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let settings = app.db.settings().await?;

    let github = match app.github.token().await {
        None => json!({ "ok": false, "error": "No GitHub token: set one in Settings, export GITHUB_TOKEN or run `gh auth login`." }),
        Some((_, source)) => match app.github.user().await {
            Ok(u) => json!({ "ok": true, "login": u.login, "name": u.name, "token_source": source }),
            Err(e) => json!({ "ok": false, "token_source": source, "error": format!("{e:#}") }),
        },
    };

    let git = {
        let version = git::version().await;
        let identity = pipeline::resolve_identity(&app, &settings, None).await;
        let mut v = json!({});
        match version {
            Ok(ver) => v["version"] = json!(ver),
            Err(e) => v["error"] = json!(format!("{e:#}")),
        }
        match identity {
            Ok((id, source)) => {
                v["identity"] = json!(id);
                v["identity_source"] = json!(source);
            }
            Err(e) if v.get("error").is_none() => v["error"] = json!(format!("{e:#}")),
            Err(_) => {}
        }
        v
    };

    let (running, queued) = app.jobs.snapshot();
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "github": github,
        "git": git,
        "optimizers": app.optimizers.statuses(&settings).await,
        "jobs": { "running": running, "queued": queued },
        "totals": app.db.totals().await?,
    })))
}

// ----------------------------------------------------------------- settings

async fn get_settings(State(app): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let settings = app.db.settings().await?;
    let token_set = app.db.kv_get(TOKEN_KEY).await?.is_some();
    Ok(Json(json!({ "settings": settings, "github_token_set": token_set })))
}

async fn put_settings(State(app): State<AppState>, Json(new): Json<Settings>) -> ApiResult<Json<Settings>> {
    let old = app.db.settings().await?;
    let new = new.sanitized();
    app.db.save_settings(&new).await?;
    if (old.poll_interval_minutes, old.poll_jitter_minutes) != (new.poll_interval_minutes, new.poll_jitter_minutes) {
        // Pull far-away checks closer when the interval shrinks.
        for repo in app.db.repos().await? {
            let candidate = pipeline::next_check_at(&new, repo.poll_interval_minutes);
            if repo.next_check_at.is_none_or(|at| at > candidate) {
                app.db.set_next_check(repo.id, candidate).await?;
            }
        }
    }
    app.optimizers.refresh().await;
    Ok(Json(new))
}

#[derive(Deserialize)]
struct TokenBody {
    token: Option<String>,
}

async fn put_token(State(app): State<AppState>, Json(body): Json<TokenBody>) -> ApiResult<StatusCode> {
    let previous = app.db.kv_get(TOKEN_KEY).await?;
    let token = body.token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    app.db.kv_set(TOKEN_KEY, token.as_deref()).await?;
    app.github.invalidate().await;
    if token.is_some()
        && let Err(e) = app.github.user().await
    {
        app.db.kv_set(TOKEN_KEY, previous.as_deref()).await?;
        app.github.invalidate().await;
        return Err(ApiError::new(StatusCode::BAD_REQUEST, format!("token rejected by GitHub: {e:#}")));
    }
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------------- repos

#[derive(Serialize)]
struct RepoView {
    #[serde(flatten)]
    repo: Repo,
    running: bool,
    queued: bool,
    phase: Option<String>,
}

fn view(app: &App, repo: Repo) -> RepoView {
    let (running, queued, phase) = app.jobs.repo_state(repo.id);
    RepoView { repo, running, queued, phase }
}

async fn load_repo(app: &App, id: i64) -> ApiResult<Repo> {
    app.db.repo(id).await?.ok_or_else(ApiError::not_found)
}

async fn list_repos(State(app): State<AppState>) -> ApiResult<Json<Vec<RepoView>>> {
    let repos = app.db.repos().await?;
    Ok(Json(repos.into_iter().map(|r| view(&app, r)).collect()))
}

async fn get_repo(State(app): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<RepoView>> {
    Ok(Json(view(&app, load_repo(&app, id).await?)))
}

#[derive(Deserialize)]
struct NewRepoBody {
    repo: String,
    #[serde(default)]
    mode: ModeRequest,
    #[serde(default = "yes")]
    auto_pr: bool,
}

fn yes() -> bool {
    true
}

async fn create_repo(
    State(app): State<AppState>,
    Json(body): Json<NewRepoBody>,
) -> ApiResult<(StatusCode, Json<RepoView>)> {
    let (id, created) = pipeline::add_repo(&app, &body.repo, body.mode, body.auto_pr, None)
        .await
        .map_err(ApiError::bad_request)?;
    if !created {
        return Err(ApiError::new(StatusCode::CONFLICT, "that repository is already being watched"));
    }
    app.jobs.enqueue(Job::Check { repo_id: id, trigger: RunTrigger::Manual, force: false });
    Ok((StatusCode::CREATED, Json(view(&app, load_repo(&app, id).await?))))
}

async fn patch_repo(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Json(patch): Json<RepoPatch>,
) -> ApiResult<Json<RepoView>> {
    let before = load_repo(&app, id).await?;
    app.db.patch_repo(id, &patch).await?;
    let repo = load_repo(&app, id).await?;
    if patch.poll_interval_minutes.is_some() {
        let settings = app.db.settings().await?;
        app.db.set_next_check(id, pipeline::next_check_at(&settings, repo.poll_interval_minutes)).await?;
    }
    if repo.enabled && !before.enabled {
        app.db.set_next_check(id, Utc::now()).await?;
    }
    Ok(Json(view(&app, load_repo(&app, id).await?)))
}

async fn delete_repo(State(app): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    if app.jobs.repo_state(id).0 {
        return Err(ApiError::new(StatusCode::CONFLICT, "the repository is being processed; try again in a moment"));
    }
    let repo = app.db.delete_repo(id).await?.ok_or_else(ApiError::not_found)?;
    remove_clone(&app, &repo).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_clone(app: &App, repo: &Repo) {
    let dir = app.config.repos_dir().join(&repo.owner).join(&repo.name);
    if let Err(e) = tokio::fs::remove_dir_all(&dir).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(dir = %dir.display(), error = %e, "could not remove clone");
    }
}

#[derive(Deserialize, Default)]
struct CheckBody {
    #[serde(default)]
    force: bool,
}

async fn check_repo(State(app): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult<StatusCode> {
    load_repo(&app, id).await?;
    let body: CheckBody = if body.is_empty() { CheckBody::default() } else { serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(e.into()))? };
    app.jobs.enqueue(Job::Check { repo_id: id, trigger: RunTrigger::Manual, force: body.force });
    Ok(StatusCode::ACCEPTED)
}

async fn check_all(State(app): State<AppState>) -> ApiResult<StatusCode> {
    for repo in app.db.repos().await?.into_iter().filter(|r| r.enabled) {
        app.jobs.enqueue(Job::Check { repo_id: repo.id, trigger: RunTrigger::Manual, force: false });
    }
    Ok(StatusCode::ACCEPTED)
}

async fn publish_repo(State(app): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let repo = load_repo(&app, id).await?;
    if repo.pending_commit.is_none() {
        return Err(ApiError::new(StatusCode::CONFLICT, "there is no optimization awaiting approval"));
    }
    app.jobs.enqueue(Job::Publish { repo_id: id });
    Ok(StatusCode::ACCEPTED)
}

async fn reset_ignored(State(app): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    load_repo(&app, id).await?;
    app.db.clear_ignored_blobs(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --------------------------------------------------------------------- runs

#[derive(Deserialize)]
struct RunsQuery {
    limit: Option<i64>,
    repo_id: Option<i64>,
}

async fn repo_runs(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Json<Vec<crate::models::Run>>> {
    Ok(Json(app.db.runs(Some(id), q.limit.unwrap_or(50)).await?))
}

async fn list_runs(State(app): State<AppState>, Query(q): Query<RunsQuery>) -> ApiResult<Json<Vec<crate::models::Run>>> {
    Ok(Json(app.db.runs(q.repo_id, q.limit.unwrap_or(100)).await?))
}

async fn get_run(State(app): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<serde_json::Value>> {
    let run = app.db.run(id).await?.ok_or_else(ApiError::not_found)?;
    let files = app.db.run_files(id).await?;
    Ok(Json(json!({ "run": run, "files": files })))
}

// ------------------------------------------------------------------ sources

async fn list_sources(State(app): State<AppState>) -> ApiResult<Json<Vec<crate::models::Source>>> {
    Ok(Json(app.db.sources().await?))
}

#[derive(Deserialize)]
struct NewSourceBody {
    kind: SourceKind,
    #[serde(default)]
    name: String,
    #[serde(default)]
    include_forks: bool,
    #[serde(default)]
    include_archived: bool,
    #[serde(default = "yes")]
    auto_sync: bool,
}

async fn create_source(
    State(app): State<AppState>,
    Json(body): Json<NewSourceBody>,
) -> ApiResult<(StatusCode, Json<crate::models::Source>)> {
    let name = match body.kind {
        SourceKind::Me => String::new(),
        _ => {
            let n = body.name.trim().trim_start_matches('@').to_string();
            if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid user or organization name"));
            }
            n
        }
    };
    if app.db.sources().await?.iter().any(|s| s.kind == body.kind && s.name.eq_ignore_ascii_case(&name)) {
        return Err(ApiError::new(StatusCode::CONFLICT, "that source already exists"));
    }
    let id = app
        .db
        .insert_source(body.kind, &name, body.include_forks, body.include_archived, body.auto_sync)
        .await?;
    app.jobs.enqueue(Job::SyncSource { source_id: id });
    let source = app.db.source(id).await?.ok_or_else(ApiError::not_found)?;
    Ok((StatusCode::CREATED, Json(source)))
}

#[derive(Deserialize)]
struct SourcePatch {
    include_forks: Option<bool>,
    include_archived: Option<bool>,
    auto_sync: Option<bool>,
}

async fn patch_source(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Json(p): Json<SourcePatch>,
) -> ApiResult<Json<crate::models::Source>> {
    app.db.source(id).await?.ok_or_else(ApiError::not_found)?;
    app.db.update_source(id, p.include_forks, p.include_archived, p.auto_sync).await?;
    Ok(Json(app.db.source(id).await?.ok_or_else(ApiError::not_found)?))
}

#[derive(Deserialize)]
struct DeleteSourceQuery {
    #[serde(default)]
    delete_repos: bool,
}

async fn delete_source(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<DeleteSourceQuery>,
) -> ApiResult<StatusCode> {
    if app.jobs.is_queued_or_running(JobKey::Source(id)) && q.delete_repos {
        return Err(ApiError::new(StatusCode::CONFLICT, "the source is syncing; try again in a moment"));
    }
    app.db.source(id).await?.ok_or_else(ApiError::not_found)?;
    for repo in app.db.delete_source(id, q.delete_repos).await? {
        remove_clone(&app, &repo).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn sync_source(State(app): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    app.db.source(id).await?.ok_or_else(ApiError::not_found)?;
    app.jobs.enqueue(Job::SyncSource { source_id: id });
    Ok(StatusCode::ACCEPTED)
}
