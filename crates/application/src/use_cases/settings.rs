//! Global settings and the forge token.

use git_opt_domain::{model::Settings, values::AccessToken};

use crate::{AppError, AppResult, Services};

impl Services {
    pub async fn settings(&self) -> AppResult<Settings> {
        Ok(self.ports.settings.load().await?)
    }

    /// Whether a token was saved from the UI (env or CLI tokens don't count).
    pub async fn has_saved_token(&self) -> AppResult<bool> {
        Ok(self.ports.secrets.token().await?.is_some())
    }

    /// Saves the settings. A shorter interval pulls far-away checks closer;
    /// strategies are probed again (a tool may have been installed).
    pub async fn update_settings(&self, new: Settings) -> AppResult<Settings> {
        let p = &self.ports;
        let old = p.settings.load().await?;
        let new = new.sanitized();
        p.settings.save(&new).await?;
        if old.poll_policy() != new.poll_policy() {
            let now = p.clock.now();
            for repo in p.repositories.list().await? {
                let candidate = repo.poll_policy(&new).next_check(now, |max| p.random.up_to(max));
                if repo.next_check_at.is_none_or(|at| at > candidate) {
                    p.repositories.schedule(repo.id, candidate).await?;
                }
            }
        }
        self.catalog.refresh().await;
        Ok(new)
    }

    /// Saves (or with `None` removes) the token, after checking it against the
    /// forge. A rejected token leaves the previous one in place.
    pub async fn set_token(&self, token: Option<String>) -> AppResult<()> {
        let p = &self.ports;
        let token = token.filter(|t| !t.trim().is_empty()).map(AccessToken::new).transpose()?;
        let previous = p.secrets.token().await?;
        p.secrets.set_token(token.as_ref()).await?;
        p.forge.forget_credentials().await;
        if token.is_some()
            && let Err(e) = p.forge.account().await
        {
            p.secrets.set_token(previous.as_ref()).await?;
            p.forge.forget_credentials().await;
            return Err(AppError::Invalid(format!("token rejected by GitHub: {e:#}")));
        }
        Ok(())
    }
}
