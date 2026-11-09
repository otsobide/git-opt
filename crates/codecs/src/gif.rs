//! GIF via `gifsicle -O3` (lossless unless `--lossy` is passed, which we never do).

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use git_opt_domain::{
    model::CodecOptions,
    ports::FormatStrategy,
    values::{FileFormat, Profile},
};

use crate::{external, verify};

pub struct Gifsicle;

#[async_trait]
impl FormatStrategy for Gifsicle {
    fn id(&self) -> &'static str {
        "gifsicle"
    }
    fn label(&self) -> &'static str {
        "GIF (gifsicle)"
    }
    fn formats(&self) -> &'static [FileFormat] {
        &[FileFormat::Gif]
    }
    fn explanation(&self) -> &'static str {
        "**GIF**: `gifsicle -O3`, without `--lossy`."
    }
    fn profile(&self, _: &CodecOptions) -> Profile {
        Profile::new("gifsicle:O3")
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        external::probe_version("gifsicle", &["--version"]).await
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, _: &CodecOptions) -> Result<Vec<u8>> {
        external::run_on_bytes("gifsicle", &["--optimize=3", "--no-warnings", "-o", "{out}", "{in}"], &input, "gif")
            .await
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || verify::same_gif_frames(&original, &candidate)).await?
    }
}
