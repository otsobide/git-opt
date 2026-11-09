//! PNG and APNG via the oxipng library (pure Rust, lossless by design).

use std::{sync::Arc, time::Duration};

use anyhow::Result;
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
        "PNG and APNG (oxipng)"
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
        tokio::task::spawn_blocking(move || {
            if is_apng(&original) {
                verify::same_apng(&original, &candidate)
            } else {
                verify::same_pixels(&original, &candidate, ImageFormat::Png)
            }
        })
        .await?
    }
}

/// True for an animated PNG (an `acTL` chunk).
pub(super) fn is_apng(data: &[u8]) -> bool {
    verify::png_chunk(data, b"acTL").is_some()
}

pub(super) const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// oxipng settings for PNGs embedded in containers (icons): recompress
/// without changing the pixel format (IHDR), because some readers only
/// accept the format they wrote.
pub(super) fn keep_format_options(s: &Settings) -> Options {
    let mut opts = Options::from_preset(s.png_level);
    opts.color_type_reduction = false;
    opts.bit_depth_reduction = false;
    opts.palette_reduction = false;
    opts.grayscale_reduction = false;
    opts.interlace = None;
    opts.optimize_alpha = false;
    opts.scale_16 = false;
    opts.strip = StripChunks::None;
    if s.png_zopfli {
        opts.deflater = Deflater::Zopfli(ZopfliOptions::default());
    }
    opts.timeout = Some(Duration::from_secs(120));
    opts
}

pub(super) fn keep_format_profile(s: &Settings) -> String {
    format!("oxipng10:l{}:zopfli{}:keep-ihdr", s.png_level, u8::from(s.png_zopfli))
}

/// Recompresses an embedded PNG; returns the input unchanged if not smaller.
pub(super) fn recompress_keep_format(data: &[u8], opts: &Options) -> Result<Vec<u8>> {
    let smaller = oxipng::optimize_from_memory(data, opts)?;
    Ok(if smaller.len() < data.len() { smaller } else { data.to_vec() })
}

/// The embedded PNG kept its pixel format and its pixels.
pub(super) fn verify_keep_format(original: &[u8], candidate: &[u8]) -> Result<()> {
    anyhow::ensure!(candidate.starts_with(SIGNATURE), "no longer a PNG");
    anyhow::ensure!(
        verify::png_chunk(original, b"IHDR") == verify::png_chunk(candidate, b"IHDR"),
        "pixel format changed"
    );
    verify::same_pixels(original, candidate, ImageFormat::Png)
}
