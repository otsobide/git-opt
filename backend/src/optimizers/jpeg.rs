//! JPEG via `jpegtran` (libjpeg-turbo / mozjpeg).
//!
//! jpegtran rewrites the entropy coding (optimized Huffman tables and,
//! when smaller, a progressive scan script) without touching the DCT
//! coefficients, so the image is bit-for-bit identical once decoded.
//! Metadata is always kept (`-copy all`): dropping EXIF orientation or the
//! ICC profile would change how the image is displayed.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use image::ImageFormat;

use super::{Optimizer, external, verify};
use crate::settings::Settings;

pub struct Jpegtran;

#[async_trait]
impl Optimizer for Jpegtran {
    fn id(&self) -> &'static str {
        "jpegtran"
    }
    fn label(&self) -> &'static str {
        "JPEG (jpegtran)"
    }
    fn category(&self) -> &'static str {
        "image"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["jpg", "jpeg", "jpe", "jfif"]
    }
    fn profile(&self, _: &Settings) -> String {
        "jpegtran:copyall:opt+prog".into()
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        external::probe_version("jpegtran", &["-version"]).await
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, _: &Settings) -> Result<Vec<u8>> {
        let baseline = external::run_on_bytes(
            "jpegtran",
            &["-copy", "all", "-optimize", "-outfile", "{out}", "{in}"],
            &input,
            "jpg",
        )
        .await?;
        let progressive = external::run_on_bytes(
            "jpegtran",
            &["-copy", "all", "-optimize", "-progressive", "-outfile", "{out}", "{in}"],
            &input,
            "jpg",
        )
        .await?;
        Ok(if progressive.len() < baseline.len() { progressive } else { baseline })
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || verify::same_pixels(&original, &candidate, ImageFormat::Jpeg)).await?
    }
}
