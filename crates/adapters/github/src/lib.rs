//! The [`CodeForge`] port implemented with the GitHub REST API. Every call is
//! made with the developer's own token, so forks and pull requests appear
//! under their account.

mod token;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use git_opt_domain::{
    model::{PrState, SourceKind},
    ports::{CodeForge, ForgeAccount, ForgePullRequest, ForgeRepository, PullRequestHead, SecretStore, TokenOrigin},
    values::{AccessToken, BranchName, CloneUrl, Identity, OwnerName, PrNumber, RepoName, RepoSlug},
};
use reqwest::{Method, RequestBuilder, Response, StatusCode, header};
use serde::{Deserialize, de::DeserializeOwned};
pub use token::{EnvToken, GhCliToken, SavedToken, TokenSource};
use tokio::sync::Mutex;

const CACHE_TTL: Duration = Duration::from_secs(300);

/// When the token was resolved, and what was found.
type CachedToken = Option<(Instant, Option<(AccessToken, TokenOrigin)>)>;

pub struct GitHub {
    http: reqwest::Client,
    api: String,
    sources: Vec<Box<dyn TokenSource>>,
    token: Mutex<CachedToken>,
    account: Mutex<Option<(Instant, ForgeAccount)>>,
}

impl GitHub {
    /// A client for `api` (e.g. `https://api.github.com`) looking for the
    /// token in `sources`, in order.
    pub fn new(api: impl Into<String>, sources: Vec<Box<dyn TokenSource>>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("git-opt/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()?;
        let api = api.into().trim_end_matches('/').to_string();
        Ok(Self { http, api, sources, token: Mutex::new(None), account: Mutex::new(None) })
    }

    /// The standard token chain: UI setting, environment, `gh` CLI.
    pub fn with_standard_tokens(api: impl Into<String>, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        let api = api.into();
        let host = git_host(&api);
        Self::new(
            api,
            vec![Box::new(SavedToken(secrets)), Box::new(EnvToken::standard()), Box::new(GhCliToken { host })],
        )
    }

    async fn require_token(&self) -> Result<AccessToken> {
        self.credentials()
            .await
            .map(|(t, _)| t)
            .ok_or_else(|| anyhow!("no GitHub token: set one in Settings, export GITHUB_TOKEN or run `gh auth login`"))
    }

    async fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let token = self.require_token().await?;
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{path}", self.api) };
        Ok(self
            .http
            .request(method, url)
            .bearer_auth(token.expose())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28"))
    }

    async fn send<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        Ok(check(req.send().await?).await?.json().await?)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let req = self.request(Method::GET, path).await?;
        self.send(req).await
    }

    /// Follows `Link: rel="next"` pagination.
    async fn get_all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let mut out = Vec::new();
        let mut next = Some(path.to_string());
        while let Some(url) = next.take() {
            let resp = check(self.request(Method::GET, &url).await?.send().await?).await?;
            next = next_link(resp.headers());
            out.extend(resp.json::<Vec<T>>().await?);
        }
        Ok(out)
    }

    async fn api_repo(&self, slug: &RepoSlug) -> Result<ApiRepo> {
        self.get(&format!("/repos/{slug}")).await
    }
}

/// Host of the web/git side for an API URL.
pub fn git_host(api: &str) -> String {
    if api.trim_end_matches('/') == "https://api.github.com" {
        return "github.com".into();
    }
    api.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("github.com")
        .to_string()
}

#[async_trait]
impl CodeForge for GitHub {
    async fn credentials(&self) -> Option<(AccessToken, TokenOrigin)> {
        let mut cached = self.token.lock().await;
        // A missing token is looked for again on every call; a found one is cached.
        if let Some((at, value @ Some(_))) = cached.as_ref()
            && at.elapsed() < CACHE_TTL
        {
            return value.clone();
        }
        let value = token::resolve(&self.sources).await;
        *cached = Some((Instant::now(), value.clone()));
        value
    }

    async fn forget_credentials(&self) {
        *self.token.lock().await = None;
        *self.account.lock().await = None;
    }

    async fn account(&self) -> Result<ForgeAccount> {
        let mut cached = self.account.lock().await;
        if let Some((at, account)) = cached.as_ref()
            && at.elapsed() < CACHE_TTL
        {
            return Ok(account.clone());
        }
        let user: ApiUser = self.get("/user").await?;
        let account = user.into_account()?;
        *cached = Some((Instant::now(), account.clone()));
        Ok(account)
    }

    async fn repository(&self, slug: &RepoSlug) -> Result<ForgeRepository> {
        self.api_repo(slug).await?.into_domain()
    }

    async fn list_repositories(&self, kind: SourceKind, owner: Option<&OwnerName>) -> Result<Vec<ForgeRepository>> {
        let me = self.account().await?;
        let path = match (kind, owner) {
            (SourceKind::Me, _) => "/user/repos?per_page=100&affiliation=owner,collaborator,organization_member".into(),
            // The public endpoint hides private repositories, even our own.
            (SourceKind::User, Some(o)) if o.same_as(me.login.as_str()) => {
                "/user/repos?per_page=100&affiliation=owner".into()
            }
            (SourceKind::User, Some(o)) => format!("/users/{o}/repos?per_page=100&type=owner"),
            (SourceKind::Org, Some(o)) => format!("/orgs/{o}/repos?per_page=100&type=all"),
            (_, None) => bail!("a {} source needs a name", kind.as_str()),
        };
        let repos: Vec<ApiRepo> = self.get_all(&path).await?;
        // Skip anything with a name we could not address anyway.
        Ok(repos.into_iter().filter_map(|r| r.into_domain().ok()).collect())
    }

    async fn fork(&self, slug: &RepoSlug) -> Result<ForgeRepository> {
        let req = self
            .request(Method::POST, &format!("/repos/{slug}/forks"))
            .await?
            .json(&serde_json::json!({ "default_branch_only": true }));
        let fork: ApiRepo = self.send(req).await?;
        let fork = fork.into_domain()?;
        // Forking is asynchronous: wait until the repository answers.
        for _ in 0..30 {
            if self.api_repo(&fork.slug).await.is_ok() {
                return Ok(fork);
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        bail!("fork {} was not ready after 60s", fork.slug)
    }

    async fn pull_request(&self, slug: &RepoSlug, number: PrNumber) -> Result<ForgePullRequest> {
        let pull: ApiPull = self.get(&format!("/repos/{slug}/pulls/{number}")).await?;
        Ok(pull.into_domain())
    }

    async fn find_open_pull_request(
        &self,
        slug: &RepoSlug,
        head: &PullRequestHead,
    ) -> Result<Option<ForgePullRequest>> {
        let req = self
            .request(Method::GET, &format!("/repos/{slug}/pulls"))
            .await?
            .query(&[("state", "open"), ("head", &format!("{}:{}", head.owner, head.branch))]);
        let pulls: Vec<ApiPull> = self.send(req).await?;
        Ok(pulls.into_iter().next().map(ApiPull::into_domain))
    }

    async fn open_pull_request(
        &self,
        slug: &RepoSlug,
        head: &PullRequestHead,
        base: &BranchName,
        title: &str,
        body: &str,
    ) -> Result<ForgePullRequest> {
        let head =
            if head.cross_repository { format!("{}:{}", head.owner, head.branch) } else { head.branch.to_string() };
        let req = self.request(Method::POST, &format!("/repos/{slug}/pulls")).await?.json(&serde_json::json!({
            "title": title, "body": body, "head": head, "base": base.as_str(), "maintainer_can_modify": true,
        }));
        let pull: ApiPull = self.send(req).await?;
        Ok(pull.into_domain())
    }

    async fn update_pull_request(
        &self,
        slug: &RepoSlug,
        number: PrNumber,
        title: &str,
        body: &str,
    ) -> Result<ForgePullRequest> {
        let req = self
            .request(Method::PATCH, &format!("/repos/{slug}/pulls/{number}"))
            .await?
            .json(&serde_json::json!({ "title": title, "body": body }));
        let pull: ApiPull = self.send(req).await?;
        Ok(pull.into_domain())
    }
}

// --------------------------------------------------------------- API types

#[derive(Debug, Deserialize)]
struct ApiOwner {
    login: String,
}

#[derive(Debug, Default, Deserialize)]
struct ApiPermissions {
    #[serde(default)]
    push: bool,
}

#[derive(Debug, Deserialize)]
struct ApiRepo {
    name: String,
    owner: ApiOwner,
    clone_url: String,
    default_branch: Option<String>,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    disabled: bool,
    permissions: Option<ApiPermissions>,
}

impl ApiRepo {
    fn into_domain(self) -> Result<ForgeRepository> {
        Ok(ForgeRepository {
            slug: RepoSlug::new(OwnerName::new(self.owner.login)?, RepoName::new(self.name)?),
            clone_url: CloneUrl::new(self.clone_url)?,
            default_branch: BranchName::new(self.default_branch.unwrap_or_else(|| "main".into()))?,
            is_fork: self.fork,
            read_only: self.archived || self.disabled,
            can_push: self.permissions.is_some_and(|p| p.push),
        })
    }
}

#[derive(Debug, Deserialize)]
struct ApiUser {
    id: u64,
    login: String,
    name: Option<String>,
}

impl ApiUser {
    fn into_account(self) -> Result<ForgeAccount> {
        let display_name = self.name.filter(|n| !n.trim().is_empty());
        // GitHub's private no-reply address, attributed to this account.
        let email = format!("{}+{}@users.noreply.github.com", self.id, self.login);
        let commit_identity = Identity::new(display_name.clone().unwrap_or_else(|| self.login.clone()), email)?;
        Ok(ForgeAccount { login: OwnerName::new(self.login)?, display_name, commit_identity })
    }
}

#[derive(Debug, Deserialize)]
struct ApiPull {
    number: i64,
    html_url: String,
    state: String,
    merged_at: Option<String>,
}

impl ApiPull {
    fn into_domain(self) -> ForgePullRequest {
        let state = match (self.state.as_str(), self.merged_at.is_some()) {
            ("open", _) => PrState::Open,
            (_, true) => PrState::Merged,
            _ => PrState::Closed,
        };
        ForgePullRequest { number: PrNumber::new(self.number), url: self.html_url, state }
    }
}

#[derive(Deserialize)]
struct ApiError {
    message: Option<String>,
    errors: Option<Vec<serde_json::Value>>,
}

async fn check(resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<ApiError>(&body)
        .ok()
        .map(|e| {
            let mut msg = e.message.unwrap_or_default();
            if let Some(errs) = e.errors.filter(|e| !e.is_empty()) {
                msg.push_str(&format!(" {}", serde_json::Value::from(errs)));
            }
            msg
        })
        .unwrap_or(body);
    let hint = match status {
        StatusCode::UNAUTHORIZED => " (token invalid or expired)",
        StatusCode::FORBIDDEN => " (insufficient permissions or rate limited)",
        StatusCode::NOT_FOUND => " (not found or no access)",
        _ => "",
    };
    Err(anyhow!("GitHub API {status}{hint}: {}", detail.trim()))
}

fn next_link(headers: &header::HeaderMap) -> Option<String> {
    let link = headers.get(header::LINK)?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"").then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_next_link() {
        let mut h = header::HeaderMap::new();
        h.insert(
            header::LINK,
            r#"<https://api.github.com/user/repos?page=2>; rel="next", <https://api.github.com/user/repos?page=5>; rel="last""#
                .parse()
                .unwrap(),
        );
        assert_eq!(next_link(&h).as_deref(), Some("https://api.github.com/user/repos?page=2"));
    }

    #[test]
    fn maps_api_payloads_to_the_domain() {
        let repo: ApiRepo = serde_json::from_value(serde_json::json!({
            "name": "site", "owner": { "login": "acme" }, "clone_url": "https://github.com/acme/site.git",
            "default_branch": "trunk", "archived": true, "permissions": { "push": true }
        }))
        .unwrap();
        let repo = repo.into_domain().unwrap();
        assert_eq!(repo.slug.to_string(), "acme/site");
        assert_eq!(repo.default_branch.as_str(), "trunk");
        assert!(repo.read_only && repo.can_push && !repo.is_fork);

        let user: ApiUser =
            serde_json::from_value(serde_json::json!({ "id": 42, "login": "jdev", "name": "" })).unwrap();
        let account = user.into_account().unwrap();
        assert_eq!(account.commit_identity.to_string(), "jdev <42+jdev@users.noreply.github.com>");

        let merged: ApiPull = serde_json::from_value(serde_json::json!({
            "number": 7, "html_url": "u", "state": "closed", "merged_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(merged.into_domain().state, PrState::Merged);
    }

    #[test]
    fn git_host_follows_the_api_url() {
        assert_eq!(git_host("https://api.github.com"), "github.com");
        assert_eq!(git_host("https://ghe.example.com/api/v3"), "ghe.example.com");
    }
}
