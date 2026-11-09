//! PNG via the oxipng library (pure Rust, lossless by design).

use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
use async_trait::async_trait;
use image::ImageFormat;
use oxipng::{Deflater, Options, StripChunks, ZopfliOptions};

use super::{Optimizer, verify};
use crate::settings::Settings;

pub struct Oxipng;

#[async_trait]
impl Optimizer for Oxipng {
    fn id(&self) -> &'static str {
        "oxipng"
    }
    fn label(&self) -> &'static str {
        "PNG (oxipng)"
    }
    fn category(&self) -> &'static str {
        "image"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["png"]
    }
    fn profile(&self, s: &Settings) -> String {
        format!("oxipng10:l{}:strip{}:zopfli{}", s.png_level, u8::from(s.strip_metadata), u8::from(s.png_zopfli))
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        Ok("built-in oxipng 10".into())
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, s: &Settings) -> Result<Vec<u8>> {
        if is_apng(&input) {
            // The verifier only sees the first frame; do not risk animations.
            bail!("animated PNG (APNG) is not supported");
        }
        let mut opts = Options::from_preset(s.png_level);
        opts.strip = if s.strip_metadata { StripChunks::Safe } else { StripChunks::None };
        // Keep RGB of fully transparent pixels: pixel-exact, not just visually equal.
        opts.optimize_alpha = false;
        opts.scale_16 = false;
        if s.png_zopfli {
            opts.deflater = Deflater::Zopfli(ZopfliOptions::default());
        }
        opts.timeout = Some(Duration::from_secs(120));
        Ok(tokio::task::spawn_blocking(move || oxipng::optimize_from_memory(&input, &opts)).await??)
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || verify::same_pixels(&original, &candidate, ImageFormat::Png)).await?
    }
}

/// True if an `acTL` chunk appears before the first `IDAT`.
pub(super) fn is_apng(data: &[u8]) -> bool {
    let mut pos = 8; // PNG signature
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let kind = &data[pos + 4..pos + 8];
        match kind {
            b"acTL" => return true,
            b"IDAT" => return false,
            _ => {}
        }
        pos = pos.saturating_add(12).saturating_add(len);
    }
    false
}
