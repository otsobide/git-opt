//! GIF via `gifsicle -O3` (lossless unless `--lossy` is passed, which we never do).

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use super::{Optimizer, external, verify};
use crate::settings::Settings;

pub struct Gifsicle;

#[async_trait]
impl Optimizer for Gifsicle {
    fn id(&self) -> &'static str {
        "gifsicle"
    }
    fn label(&self) -> &'static str {
        "GIF (gifsicle)"
    }
    fn category(&self) -> &'static str {
        "image"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["gif"]
    }
    fn profile(&self, _: &Settings) -> String {
        "gifsicle:O3".into()
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        external::probe_version("gifsicle", &["--version"]).await
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, _: &Settings) -> Result<Vec<u8>> {
        external::run_on_bytes("gifsicle", &["--optimize=3", "--no-warnings", "-o", "{out}", "{in}"], &input, "gif")
            .await
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || verify::same_gif_frames(&original, &candidate)).await?
    }
}
