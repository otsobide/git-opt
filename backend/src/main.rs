mod config;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("git_opt=info,tower_http=warn")),
        )
        .init();

    let config = config::Config::from_env()?;
    tracing::info!(?config.bind, data_dir = %config.data_dir.display(), "starting git-opt");
    Ok(())
}
