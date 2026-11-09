//! Pluggable file optimizers.
//!
//! An [`Optimizer`] takes the bytes of one file and returns a smaller,
//! *equivalent* version of it. Today every optimizer works on images and
//! proves equivalence by decoding both versions and comparing pixels, but the
//! trait is format-agnostic: adding a new kind of file means implementing it
//! and registering it in [`Registry::new`].

mod external;
mod gif;
mod jpeg;
mod png;
mod verify;

use std::{path::Path, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::settings::Settings;

#[async_trait]
pub trait Optimizer: Send + Sync {
    /// Stable identifier, used in settings, cache keys and reports.
    fn id(&self) -> &'static str;
    /// Human-friendly name for the UI.
    fn label(&self) -> &'static str;
    /// Broad family ("image", later "svg", "json", …).
    fn category(&self) -> &'static str;
    /// Lower-case file extensions this optimizer handles.
    fn extensions(&self) -> &'static [&'static str];
    fn enabled_by_default(&self) -> bool {
        true
    }
    /// Fingerprint of everything that changes the output for a given input.
    /// Part of the blob cache key, so changing a setting re-evaluates files.
    fn profile(&self, settings: &Settings) -> String;
    /// Ok(detail) when the optimizer can run here, Err(reason) otherwise.
    async fn probe(&self) -> std::result::Result<String, String>;
    /// Produces a candidate. May be bigger than the input; the caller decides.
    async fn optimize(&self, input: Arc<Vec<u8>>, settings: &Settings) -> Result<Vec<u8>>;
    /// Must fail unless `candidate` is lossless with respect to `original`.
    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()>;
}

#[derive(Debug, Clone, Serialize)]
pub struct OptimizerStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub extensions: &'static [&'static str],
    pub enabled: bool,
    pub available: bool,
    pub detail: String,
}

/// Result of running one file through its optimizer.
#[derive(Debug)]
pub enum Outcome {
    Optimized(Vec<u8>),
    /// No meaningful gain (or verification could not prove equivalence).
    Optimal(Option<String>),
    Failed(String),
}

pub struct Registry {
    optimizers: Vec<Arc<dyn Optimizer>>,
    /// Availability per optimizer, refreshed by [`Registry::refresh`].
    available: RwLock<Vec<(bool, String)>>,
}

impl Registry {
    pub async fn new() -> Arc<Self> {
        let optimizers: Vec<Arc<dyn Optimizer>> = vec![
            Arc::new(png::Oxipng),
            Arc::new(jpeg::Jpegtran),
            Arc::new(gif::Gifsicle),
        ];
        let registry = Arc::new(Self { available: RwLock::new(Vec::new()), optimizers });
        registry.refresh().await;
        registry
    }

    pub async fn refresh(&self) {
        let mut probes = Vec::with_capacity(self.optimizers.len());
        for o in &self.optimizers {
            probes.push(match o.probe().await {
                Ok(detail) => (true, detail),
                Err(reason) => (false, reason),
            });
        }
        *self.available.write().await = probes;
    }

    pub async fn statuses(&self, settings: &Settings) -> Vec<OptimizerStatus> {
        let available = self.available.read().await;
        self.optimizers
            .iter()
            .zip(available.iter())
            .map(|(o, (ok, detail))| OptimizerStatus {
                id: o.id(),
                label: o.label(),
                category: o.category(),
                extensions: o.extensions(),
                enabled: settings.optimizer_enabled(o.id(), o.enabled_by_default()),
                available: *ok,
                detail: detail.clone(),
            })
            .collect()
    }

    /// Optimizers that are enabled and runnable, for building a per-run plan.
    pub async fn active(&self, settings: &Settings) -> Vec<Arc<dyn Optimizer>> {
        let available = self.available.read().await;
        self.optimizers
            .iter()
            .zip(available.iter())
            .filter(|(o, (ok, _))| *ok && settings.optimizer_enabled(o.id(), o.enabled_by_default()))
            .map(|(o, _)| o.clone())
            .collect()
    }
}

/// Picks the optimizer for a path among `active` ones, by extension.
pub fn for_path<'a>(active: &'a [Arc<dyn Optimizer>], path: &str) -> Option<&'a Arc<dyn Optimizer>> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    active.iter().find(|o| o.extensions().contains(&ext.as_str()))
}

/// Optimizes `input`, keeps the result only if it is smaller by the configured
/// margins and verified lossless.
pub async fn process(opt: &dyn Optimizer, input: Arc<Vec<u8>>, settings: &Settings) -> Outcome {
    let candidate = match opt.optimize(input.clone(), settings).await {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("{e:#}")),
    };
    let before = input.len() as u64;
    let after = candidate.len() as u64;
    if after == 0 || after >= before {
        return Outcome::Optimal(None);
    }
    let saved = before - after;
    let percent = saved as f64 * 100.0 / before as f64;
    if saved < settings.min_savings_bytes || percent < settings.min_savings_percent {
        return Outcome::Optimal(Some(format!("only {saved} bytes ({percent:.2}%)")));
    }
    let candidate = Arc::new(candidate);
    if let Err(e) = opt.verify(input, candidate.clone()).await {
        // Never ship something we cannot prove identical.
        return Outcome::Failed(format!("lossless verification failed: {e:#}"));
    }
    Outcome::Optimized(Arc::try_unwrap(candidate).unwrap_or_else(|a| (*a).clone()))
}

#[cfg(test)]
mod tests {
    use image::{
        ExtendedColorType, ImageEncoder, ImageFormat, Rgba, RgbaImage,
        codecs::{
            jpeg::JpegEncoder,
            png::{CompressionType, FilterType, PngEncoder},
        },
    };

    use super::*;

    fn sample() -> RgbaImage {
        RgbaImage::from_fn(96, 64, |x, y| {
            let alpha = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 128 };
            Rgba([(x * 2) as u8, (y * 3) as u8, ((x ^ y) * 5) as u8, alpha])
        })
    }

    fn bloated_png(img: &RgbaImage) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::NoFilter)
            .write_image(img.as_raw(), img.width(), img.height(), ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[tokio::test]
    async fn png_gets_smaller_and_stays_pixel_identical() {
        let input = Arc::new(bloated_png(&sample()));
        match process(&png::Oxipng, input.clone(), &Settings::default()).await {
            Outcome::Optimized(out) => {
                assert!(out.len() < input.len());
                verify::same_pixels(&input, &out, ImageFormat::Png).unwrap();
            }
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zopfli_png_is_pixel_identical() {
        let input = Arc::new(bloated_png(&sample()));
        let settings = Settings { png_zopfli: true, ..Settings::default() };
        assert_ne!(png::Oxipng.profile(&settings), png::Oxipng.profile(&Settings::default()));
        match process(&png::Oxipng, input.clone(), &settings).await {
            Outcome::Optimized(out) => verify::same_pixels(&input, &out, ImageFormat::Png).unwrap(),
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn already_optimal_png_is_left_alone() {
        let first = match process(&png::Oxipng, Arc::new(bloated_png(&sample())), &Settings::default()).await {
            Outcome::Optimized(out) => out,
            other => panic!("{other:?}"),
        };
        let second = process(&png::Oxipng, Arc::new(first), &Settings::default()).await;
        assert!(matches!(second, Outcome::Optimal(_)), "{second:?}");
    }

    #[test]
    fn verifier_rejects_a_single_changed_pixel() {
        let img = sample();
        let mut altered = img.clone();
        altered.put_pixel(10, 10, Rgba([0, 0, 0, 255]));
        let err = verify::same_pixels(&bloated_png(&img), &bloated_png(&altered), ImageFormat::Png);
        assert!(err.is_err());
    }

    #[test]
    fn apng_is_refused() {
        let mut png = bloated_png(&sample());
        // Splice a fake acTL chunk right after IHDR (8 sig + 25 IHDR bytes).
        let actl = [0, 0, 0, 8, b'a', b'c', b'T', b'L', 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        png.splice(33..33, actl);
        assert!(png::is_apng(&png));
        assert!(!png::is_apng(&bloated_png(&sample())));
    }

    #[tokio::test]
    async fn jpeg_is_lossless_when_jpegtran_is_installed() {
        if jpeg::Jpegtran.probe().await.is_err() {
            eprintln!("jpegtran not installed, skipping");
            return;
        }
        let rgb = image::DynamicImage::ImageRgba8(sample()).to_rgb8();
        let mut input = Vec::new();
        JpegEncoder::new_with_quality(&mut input, 92)
            .write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
            .unwrap();
        let input = Arc::new(input);
        let settings = Settings { min_savings_bytes: 1, min_savings_percent: 0.0, ..Settings::default() };
        match process(&jpeg::Jpegtran, input.clone(), &settings).await {
            Outcome::Optimized(out) => verify::same_pixels(&input, &out, ImageFormat::Jpeg).unwrap(),
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    fn animated_gif(shift: u32) -> Vec<u8> {
        use image::{Delay, Frame, codecs::gif::GifEncoder};
        let frames = (0..4).map(|i| {
            let img = RgbaImage::from_fn(64, 64, |x, y| {
                if (x + y + i * 8 + shift) % 32 < 16 { Rgba([200, 30, 30, 255]) } else { Rgba([30, 30, 200, 255]) }
            });
            Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(100, 1))
        });
        let mut out = Vec::new();
        GifEncoder::new(&mut out).encode_frames(frames).unwrap();
        out
    }

    #[test]
    fn gif_verifier_compares_every_frame() {
        verify::same_gif_frames(&animated_gif(0), &animated_gif(0)).unwrap();
        assert!(verify::same_gif_frames(&animated_gif(0), &animated_gif(1)).is_err());
    }

    #[tokio::test]
    async fn gif_is_lossless_when_gifsicle_is_installed() {
        if gif::Gifsicle.probe().await.is_err() {
            eprintln!("gifsicle not installed, skipping");
            return;
        }
        let input = Arc::new(animated_gif(0));
        let settings = Settings { min_savings_bytes: 1, min_savings_percent: 0.0, ..Settings::default() };
        match process(&gif::Gifsicle, input.clone(), &settings).await {
            Outcome::Optimized(out) => verify::same_gif_frames(&input, &out).unwrap(),
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[test]
    fn picks_optimizer_by_extension() {
        let active: Vec<Arc<dyn Optimizer>> = vec![Arc::new(png::Oxipng), Arc::new(jpeg::Jpegtran)];
        assert_eq!(for_path(&active, "a/b/Logo.PNG").unwrap().id(), "oxipng");
        assert_eq!(for_path(&active, "photo.jpeg").unwrap().id(), "jpegtran");
        assert!(for_path(&active, "anim.gif").is_none());
        assert!(for_path(&active, "README").is_none());
    }
}
