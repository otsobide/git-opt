//! Minimal GitHub REST client: repos, forks and pull requests.
//!
//! Every call is made with the developer's own token, so PRs, forks and
//! comments appear under their account.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Method, RequestBuilder, Response, StatusCode, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{process::Command, sync::Mutex};

use crate::{
    db::{Db, TOKEN_KEY},
    models::SourceKind,
};

const TOKEN_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenSource {
    /// Saved from the web UI.
    Settings,
    /// `GIT_OPT_GITHUB_TOKEN`, `GITHUB_TOKEN` or `GH_TOKEN`.
    Env,
    /// `gh auth token` (GitHub CLI session of the machine's user).
    Gh,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiOwner {
    pub login: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ApiPermissions {
    #[serde(default)]
    pub push: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiRepo {
    pub name: String,
    pub full_name: String,
    pub owner: ApiOwner,
    pub clone_url: String,
    pub default_branch: Option<String>,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub disabled: bool,
    pub permissions: Option<ApiPermissions>,
}

impl ApiRepo {
    pub fn can_push(&self) -> bool {
        self.permissions.as_ref().is_some_and(|p| p.push)
    }
    pub fn branch(&self) -> String {
        self.default_branch.clone().unwrap_or_else(|| "main".into())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiUser {
    pub id: u64,
    pub login: String,
    pub name: Option<String>,
}

impl ApiUser {
    /// GitHub's private no-reply address, attributed to this account.
    pub fn noreply_email(&self) -> String {
        format!("{}+{}@users.noreply.github.com", self.id, self.login)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiPull {
    pub number: i64,
    pub html_url: String,
    pub state: String,
    pub merged_at: Option<String>,
}

impl ApiPull {
    /// `open`, `merged` or `closed`.
    pub fn status(&self) -> &'static str {
        match (self.state.as_str(), self.merged_at.is_some()) {
            ("open", _) => "open",
            (_, true) => "merged",
            _ => "closed",
        }
    }
}

#[derive(Deserialize)]
struct ApiError {
    message: Option<String>,
    errors: Option<Vec<serde_json::Value>>,
}

struct CachedToken {
    at: Instant,
    value: Option<(String, TokenSource)>,
}

pub struct GitHub {
    http: reqwest::Client,
    api: String,
    db: Db,
    token: Mutex<Option<CachedToken>>,
    user: Mutex<Option<(Instant, ApiUser)>>,
}

impl GitHub {
    pub fn new(api: String, db: Db) -> Result<Arc<Self>> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("git-opt/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Arc::new(Self { http, api, db, token: Mutex::new(None), user: Mutex::new(None) }))
    }

    /// Host of the web/git side, derived from the API URL.
    pub fn git_host(&self) -> String {
        if self.api == "https://api.github.com" {
            "github.com".into()
        } else {
            self.api
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("github.com")
                .to_string()
        }
    }

    // ------------------------------------------------------------------ token

    /// Resolves the developer's token: UI setting, then env vars, then `gh`.
    pub async fn token(&self) -> Option<(String, TokenSource)> {
        let mut cached = self.token.lock().await;
        if let Some(c) = cached.as_ref()
            && c.at.elapsed() < TOKEN_TTL
        {
            return c.value.clone();
        }
        let value = self.resolve_token().await;
        *cached = Some(CachedToken { at: Instant::now(), value: value.clone() });
        value
    }

    pub async fn invalidate(&self) {
        *self.token.lock().await = None;
        *self.user.lock().await = None;
    }

    async fn resolve_token(&self) -> Option<(String, TokenSource)> {
        if let Ok(Some(t)) = self.db.kv_get(TOKEN_KEY).await
            && !t.trim().is_empty()
        {
            return Some((t.trim().to_string(), TokenSource::Settings));
        }
        for key in ["GIT_OPT_GITHUB_TOKEN", "GITHUB_TOKEN", "GH_TOKEN"] {
            if let Ok(t) = std::env::var(key)
                && !t.trim().is_empty()
            {
                return Some((t.trim().to_string(), TokenSource::Env));
            }
        }
        let out = Command::new("gh")
            .args(["auth", "token", "--hostname", &self.git_host()])
            .output()
            .await
            .ok()?;
        let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !t.is_empty()).then_some((t, TokenSource::Gh))
    }

    async fn require_token(&self) -> Result<String> {
        self.token.lock().await.take_if(|c| c.value.is_none());
        self.token().await.map(|(t, _)| t).ok_or_else(|| {
            anyhow!("no GitHub token: set one in Settings, export GITHUB_TOKEN or run `gh auth login`")
        })
    }

    // ----------------------------------------------------------------- plumbing

    async fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
        let token = self.require_token().await?;
        let url = if path.starts_with("http") { path.to_string() } else { format!("{}{}", self.api, path) };
        Ok(self
            .http
            .request(method, url)
            .bearer_auth(token)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28"))
    }

    async fn send<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let resp = check(req.send().await?).await?;
        Ok(resp.json().await?)
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

    // -------------------------------------------------------------- endpoints

    /// The account behind the token (cached for a few minutes).
    pub async fn user(&self) -> Result<ApiUser> {
        let mut cached = self.user.lock().await;
        if let Some((at, u)) = cached.as_ref()
            && at.elapsed() < TOKEN_TTL
        {
            return Ok(u.clone());
        }
        let u: ApiUser = self.get("/user").await?;
        *cached = Some((Instant::now(), u.clone()));
        Ok(u)
    }

    pub async fn repo(&self, owner: &str, name: &str) -> Result<ApiRepo> {
        self.get(&format!("/repos/{owner}/{name}")).await
    }

    /// Repositories of a user, an org, or everything the token can access.
    pub async fn list_repos(&self, kind: SourceKind, name: &str) -> Result<Vec<ApiRepo>> {
        let me = self.user().await?;
        let path = match kind {
            SourceKind::Me => "/user/repos?per_page=100&affiliation=owner,collaborator,organization_member".to_string(),
            // The public endpoint hides private repos, even our own.
            SourceKind::User if name.eq_ignore_ascii_case(&me.login) => {
                "/user/repos?per_page=100&affiliation=owner".to_string()
            }
            SourceKind::User => format!("/users/{name}/repos?per_page=100&type=owner"),
            SourceKind::Org => format!("/orgs/{name}/repos?per_page=100&type=all"),
        };
        self.get_all(&path).await
    }

    /// Creates (or returns the existing) fork under the token's account.
    pub async fn fork(&self, owner: &str, name: &str) -> Result<ApiRepo> {
        let req = self
            .request(Method::POST, &format!("/repos/{owner}/{name}/forks"))
            .await?
            .json(&serde_json::json!({ "default_branch_only": true }));
        let fork: ApiRepo = self.send(req).await?;
        // Forking is asynchronous: wait until the repository answers.
        for _ in 0..30 {
            if self.repo(&fork.owner.login, &fork.name).await.is_ok() {
                return Ok(fork);
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        bail!("fork {} was not ready after 60s", fork.full_name)
    }

    pub async fn pull(&self, owner: &str, name: &str, number: i64) -> Result<ApiPull> {
        self.get(&format!("/repos/{owner}/{name}/pulls/{number}")).await
    }

    /// Open PR whose head is `head_owner:branch`, if any.
    pub async fn find_open_pull(&self, owner: &str, name: &str, head_owner: &str, branch: &str) -> Result<Option<ApiPull>> {
        let req = self
            .request(Method::GET, &format!("/repos/{owner}/{name}/pulls"))
            .await?
            .query(&[("state", "open"), ("head", &format!("{head_owner}:{branch}"))]);
        let pulls: Vec<ApiPull> = self.send(req).await?;
        Ok(pulls.into_iter().next())
    }

    pub async fn create_pull(
        &self,
        owner: &str,
        name: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<ApiPull> {
        let req = self
            .request(Method::POST, &format!("/repos/{owner}/{name}/pulls"))
            .await?
            .json(&serde_json::json!({
                "title": title, "body": body, "head": head, "base": base, "maintainer_can_modify": true,
            }));
        self.send(req).await
    }

    pub async fn update_pull(&self, owner: &str, name: &str, number: i64, title: &str, body: &str) -> Result<ApiPull> {
        let req = self
            .request(Method::PATCH, &format!("/repos/{owner}/{name}/pulls/{number}"))
            .await?
            .json(&serde_json::json!({ "title": title, "body": body }));
        self.send(req).await
    }
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
        rel.contains("rel=\"next\"")
            .then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

/// Accepts `owner/name`, `https://github.com/owner/name(.git)` or
/// `git@github.com:owner/name.git`.
pub fn parse_repo_ref(input: &str) -> Result<(String, String)> {
    let s = input.trim().trim_end_matches('/');
    let s = s.strip_suffix(".git").unwrap_or(s);
    let path = if let Some(rest) = s.split_once("://").map(|(_, r)| r) {
        rest.split_once('/').map(|(_, p)| p).unwrap_or("")
    } else if let Some((_, p)) = s.split_once(':').filter(|(h, _)| h.contains('@')) {
        p
    } else {
        s
    };
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    match (parts.next(), parts.next()) {
        (Some(o), Some(n)) if valid_name(o) && valid_name(n) => Ok((o.to_string(), n.to_string())),
        _ => Err(anyhow!("expected owner/name or a GitHub URL, got {input:?}")).context("invalid repository"),
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repo_references() {
        let want = ("octo".to_string(), "hello-world".to_string());
        for input in [
            "octo/hello-world",
            "https://github.com/octo/hello-world",
            "https://github.com/octo/hello-world.git",
            "https://github.com/octo/hello-world/",
            "git@github.com:octo/hello-world.git",
            " octo/hello-world ",
        ] {
            assert_eq!(parse_repo_ref(input).unwrap(), want, "{input}");
        }
        assert!(parse_repo_ref("octo").is_err());
        assert!(parse_repo_ref("octo/hello world").is_err());
    }

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
}
