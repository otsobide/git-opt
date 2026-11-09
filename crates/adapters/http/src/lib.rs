//! Driving adapter: the JSON API used by the web UI (and scripts), plus the
//! built UI itself. Handlers only translate HTTP to use cases and back.

mod dto;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, patch, post, put},
};
use dto::{RepoDto, RunDto, RunFileDto, SettingsDto, SourceDto};
use git_opt_application::{AppError, Services, use_cases::repositories::ModeRequest};
use git_opt_domain::{
    model::{NewSource, RepoMode, RepositoryChanges, Settings, SourceChanges, SourceKind},
    values::{IgnoreRules, OwnerName, RepoId, RunId, SourceId},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};

pub struct HttpConfig {
    /// When set, every API call needs `Authorization: Bearer <token>`.
    pub admin_token: Option<String>,
    /// The built web UI (`frontend/dist`), served with an SPA fallback.
    pub static_dir: Option<PathBuf>,
    /// Shown on the dashboard.
    pub version: &'static str,
}

#[derive(Clone)]
struct AppState {
    services: Arc<Services>,
    admin_token: Option<Arc<str>>,
    version: &'static str,
}

pub fn router(services: Arc<Services>, config: HttpConfig) -> Router {
    let state = AppState { services, admin_token: config.admin_token.map(Arc::from), version: config.version };
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
        .route("/api/sources/{id}", patch(patch_source).delete(delete_source))
        .route("/api/sources/{id}/sync", post(sync_source))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));

    let mut router = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/auth", get(auth_info))
        .merge(protected)
        .route("/api/{*rest}", any(|| async { ApiError(AppError::NotFound) }))
        .with_state(state);
    match config.static_dir {
        Some(dir) => {
            tracing::info!(dir = %dir.display(), "serving web UI");
            router = router.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html"))));
        }
        None => tracing::warn!("web UI not found (build frontend/ or set GIT_OPT_STATIC_DIR); serving API only"),
    }
    router.layer(TraceLayer::new_for_http())
}

// ------------------------------------------------------------------- errors

struct ApiError(AppError);

impl<E: Into<AppError>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Invalid(_) => StatusCode::BAD_REQUEST,
            AppError::Internal(e) => {
                tracing::error!(error = %format!("{e:#}"), "api error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let message = match &self.0 {
            AppError::Internal(e) => format!("{e:#}"),
            other => other.to_string(),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError(AppError::Invalid(message.into()))
}

// --------------------------------------------------------------------- auth

async fn require_token(State(state): State<AppState>, headers: HeaderMap, req: Request, next: Next) -> Response {
    if let Some(expected) = &state.admin_token {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !constant_time_eq(given.as_bytes(), expected.as_bytes()) {
            return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "admin token required" }))).into_response();
        }
    }
    next.run(req).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn auth_info(State(state): State<AppState>) -> Json<Value> {
    Json(json!({ "required": state.admin_token.is_some() }))
}

// ------------------------------------------------------- status & settings

async fn status(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(dto::status_json(&s.services.status().await?, s.version)))
}

async fn get_settings(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let settings = SettingsDto::from(s.services.settings().await?);
    Ok(Json(json!({ "settings": settings, "github_token_set": s.services.has_saved_token().await? })))
}

async fn put_settings(State(s): State<AppState>, Json(body): Json<SettingsDto>) -> ApiResult<Json<SettingsDto>> {
    let settings = Settings::try_from(body)?;
    Ok(Json(s.services.update_settings(settings).await?.into()))
}

#[derive(Deserialize)]
struct TokenBody {
    token: Option<String>,
}

async fn put_token(State(s): State<AppState>, Json(body): Json<TokenBody>) -> ApiResult<StatusCode> {
    s.services.set_token(body.token).await?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------- repositories

async fn list_repos(State(s): State<AppState>) -> ApiResult<Json<Vec<RepoDto>>> {
    Ok(Json(s.services.list_repositories().await?.into_iter().map(RepoDto::from).collect()))
}

async fn get_repo(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<RepoDto>> {
    Ok(Json(s.services.repository(RepoId::new(id)).await?.into()))
}

#[derive(Deserialize)]
struct NewRepoBody {
    repo: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default = "yes")]
    auto_pr: bool,
}

fn yes() -> bool {
    true
}

async fn create_repo(State(s): State<AppState>, Json(body): Json<NewRepoBody>) -> ApiResult<(StatusCode, Json<RepoDto>)> {
    let mode = match body.mode.as_deref() {
        None | Some("auto") => ModeRequest::Auto,
        Some("direct") => ModeRequest::Direct,
        Some("fork") => ModeRequest::Fork,
        Some(other) => return Err(invalid(format!("unknown mode {other:?}"))),
    };
    let view = s.services.add_repository(&body.repo, mode, body.auto_pr).await?;
    Ok((StatusCode::CREATED, Json(view.into())))
}

#[derive(Deserialize)]
struct RepoPatchBody {
    enabled: Option<bool>,
    auto_pr: Option<bool>,
    mode: Option<String>,
    /// Absent: unchanged; `null`: back to the global interval.
    #[serde(default, deserialize_with = "present")]
    poll_interval_minutes: Option<Option<i64>>,
    ignore_globs: Option<String>,
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<i64>>, D::Error> {
    Option::<i64>::deserialize(d).map(Some)
}

async fn patch_repo(State(s): State<AppState>, Path(id): Path<i64>, Json(b): Json<RepoPatchBody>) -> ApiResult<Json<RepoDto>> {
    let mode = b.mode.map(|m| RepoMode::parse(&m).ok_or_else(|| invalid(format!("unknown mode {m:?}")))).transpose()?;
    let changes = RepositoryChanges {
        enabled: b.enabled,
        auto_pr: b.auto_pr,
        mode,
        poll_interval_minutes: b.poll_interval_minutes.map(|m| m.map(|m| m.clamp(1, 7 * 24 * 60) as u32)),
        ignore_rules: b.ignore_globs.map(IgnoreRules::new).transpose()?,
    };
    Ok(Json(s.services.update_repository(RepoId::new(id), changes).await?.into()))
}

async fn delete_repo(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    s.services.remove_repository(RepoId::new(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct CheckBody {
    #[serde(default)]
    force: bool,
}

async fn check_repo(State(s): State<AppState>, Path(id): Path<i64>, body: Bytes) -> ApiResult<StatusCode> {
    let body: CheckBody =
        if body.is_empty() { CheckBody::default() } else { serde_json::from_slice(&body).map_err(|e| invalid(e.to_string()))? };
    s.services.request_check(RepoId::new(id), body.force).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn check_all(State(s): State<AppState>) -> ApiResult<StatusCode> {
    s.services.request_check_all().await?;
    Ok(StatusCode::ACCEPTED)
}

async fn publish_repo(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    s.services.request_publish(RepoId::new(id)).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn reset_ignored(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    s.services.forget_rejections(RepoId::new(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --------------------------------------------------------------------- runs

#[derive(Deserialize)]
struct RunsQuery {
    limit: Option<u32>,
    repo_id: Option<i64>,
}

async fn repo_runs(State(s): State<AppState>, Path(id): Path<i64>, Query(q): Query<RunsQuery>) -> ApiResult<Json<Vec<RunDto>>> {
    let runs = s.services.runs(Some(RepoId::new(id)), q.limit.unwrap_or(50)).await?;
    Ok(Json(runs.into_iter().map(RunDto::from).collect()))
}

async fn list_runs(State(s): State<AppState>, Query(q): Query<RunsQuery>) -> ApiResult<Json<Vec<RunDto>>> {
    let runs = s.services.runs(q.repo_id.map(RepoId::new), q.limit.unwrap_or(100)).await?;
    Ok(Json(runs.into_iter().map(RunDto::from).collect()))
}

async fn get_run(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let (run, files) = s.services.run(RunId::new(id)).await?;
    let files: Vec<RunFileDto> = files.into_iter().map(|f| RunFileDto::new(id, f)).collect();
    Ok(Json(json!({ "run": RunDto::from(run), "files": files })))
}

// ------------------------------------------------------------------ sources

async fn list_sources(State(s): State<AppState>) -> ApiResult<Json<Vec<SourceDto>>> {
    Ok(Json(s.services.list_sources().await?.into_iter().map(SourceDto::from).collect()))
}

#[derive(Deserialize)]
struct NewSourceBody {
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    include_forks: bool,
    #[serde(default)]
    include_archived: bool,
    #[serde(default = "yes")]
    auto_sync: bool,
}

async fn create_source(State(s): State<AppState>, Json(b): Json<NewSourceBody>) -> ApiResult<(StatusCode, Json<SourceDto>)> {
    let kind = SourceKind::parse(&b.kind).ok_or_else(|| invalid(format!("unknown source kind {:?}", b.kind)))?;
    let owner = match kind {
        SourceKind::Me => None,
        _ => Some(OwnerName::new(b.name).map_err(|_| invalid("invalid user or organization name"))?),
    };
    let source = NewSource { kind, owner, include_forks: b.include_forks, include_archived: b.include_archived, auto_sync: b.auto_sync };
    Ok((StatusCode::CREATED, Json(s.services.add_source(source).await?.into())))
}

#[derive(Deserialize)]
struct SourcePatchBody {
    include_forks: Option<bool>,
    include_archived: Option<bool>,
    auto_sync: Option<bool>,
}

async fn patch_source(State(s): State<AppState>, Path(id): Path<i64>, Json(b): Json<SourcePatchBody>) -> ApiResult<Json<SourceDto>> {
    let changes = SourceChanges { include_forks: b.include_forks, include_archived: b.include_archived, auto_sync: b.auto_sync };
    Ok(Json(s.services.update_source(SourceId::new(id), changes).await?.into()))
}

#[derive(Deserialize)]
struct DeleteSourceQuery {
    #[serde(default)]
    delete_repos: bool,
}

async fn delete_source(State(s): State<AppState>, Path(id): Path<i64>, Query(q): Query<DeleteSourceQuery>) -> ApiResult<StatusCode> {
    s.services.remove_source(SourceId::new(id), q.delete_repos).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sync_source(State(s): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    s.services.request_sync(SourceId::new(id)).await?;
    Ok(StatusCode::ACCEPTED)
}
