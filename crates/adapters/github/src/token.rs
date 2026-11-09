//! Where the developer's token comes from: a chain of strategies, tried in
//! order. The standard chain is the UI setting, then environment variables,
//! then the GitHub CLI session.

use std::sync::Arc;

use async_trait::async_trait;
use git_opt_domain::{
    ports::{SecretStore, TokenOrigin},
    values::AccessToken,
};
use tokio::process::Command;

#[async_trait]
pub trait TokenSource: Send + Sync {
    fn origin(&self) -> TokenOrigin;
    async fn find(&self) -> Option<AccessToken>;
}

/// The token saved from the web UI.
pub struct SavedToken(pub Arc<dyn SecretStore>);

#[async_trait]
impl TokenSource for SavedToken {
    fn origin(&self) -> TokenOrigin {
        TokenOrigin::Settings
    }
    async fn find(&self) -> Option<AccessToken> {
        self.0.token().await.ok().flatten()
    }
}

/// The first non-empty variable among `names`.
pub struct EnvToken(pub Vec<String>);

impl EnvToken {
    pub fn standard() -> Self {
        Self(["GIT_OPT_GITHUB_TOKEN", "GITHUB_TOKEN", "GH_TOKEN"].map(String::from).to_vec())
    }
}

#[async_trait]
impl TokenSource for EnvToken {
    fn origin(&self) -> TokenOrigin {
        TokenOrigin::Environment
    }
    async fn find(&self) -> Option<AccessToken> {
        self.0.iter().find_map(|name| AccessToken::new(std::env::var(name).ok()?).ok())
    }
}

/// `gh auth token --hostname <host>`: the GitHub CLI session of the user
/// running git-opt.
pub struct GhCliToken {
    pub host: String,
}

#[async_trait]
impl TokenSource for GhCliToken {
    fn origin(&self) -> TokenOrigin {
        TokenOrigin::Cli
    }
    async fn find(&self) -> Option<AccessToken> {
        let out = Command::new("gh").args(["auth", "token", "--hostname", &self.host]).output().await.ok()?;
        out.status.success().then(|| AccessToken::new(String::from_utf8_lossy(&out.stdout)).ok())?
    }
}

/// Tries each source in order.
pub async fn resolve(sources: &[Box<dyn TokenSource>]) -> Option<(AccessToken, TokenOrigin)> {
    for source in sources {
        if let Some(token) = source.find().await {
            return Some((token, source.origin()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Option<&'static str>, TokenOrigin);

    #[async_trait]
    impl TokenSource for Fixed {
        fn origin(&self) -> TokenOrigin {
            self.1
        }
        async fn find(&self) -> Option<AccessToken> {
            self.0.map(|t| AccessToken::new(t).unwrap())
        }
    }

    #[tokio::test]
    async fn the_first_source_with_a_token_wins() {
        let chain: Vec<Box<dyn TokenSource>> = vec![
            Box::new(Fixed(None, TokenOrigin::Settings)),
            Box::new(Fixed(Some("from-env"), TokenOrigin::Environment)),
            Box::new(Fixed(Some("from-gh"), TokenOrigin::Cli)),
        ];
        let (token, origin) = resolve(&chain).await.unwrap();
        assert_eq!((token.expose(), origin), ("from-env", TokenOrigin::Environment));
        assert!(resolve(&[]).await.is_none());
    }

    #[tokio::test]
    async fn env_source_skips_unset_variables() {
        let source = EnvToken(vec!["GIT_OPT_TEST_SURELY_UNSET_VARIABLE".into()]);
        assert!(source.find().await.is_none());
    }
}
