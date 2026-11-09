//! The activity log.

use git_opt_domain::{
    model::Run,
    ports::StoredFileChange,
    values::{RepoId, RunId},
};

use crate::{AppError, AppResult, Services};

impl Services {
    pub async fn runs(&self, repo: Option<RepoId>, limit: u32) -> AppResult<Vec<Run>> {
        Ok(self.ports.runs.list(repo, limit).await?)
    }

    pub async fn run(&self, id: RunId) -> AppResult<(Run, Vec<StoredFileChange>)> {
        let run = self.ports.runs.get(id).await?.ok_or(AppError::NotFound)?;
        let files = self.ports.runs.files(id).await?;
        Ok((run, files))
    }
}
