//! Pluggable file optimizers.
//!
//! An [`Optimizer`] takes the bytes of one file and returns a smaller,
//! *equivalent* version of it. Today every optimizer works on images and
//! proves equivalence by decoding both versions and comparing pixels, but the
//! trait is format-agnostic: adding a new kind of file means implementing it
//! and registering it in [`Registry::new`].

mod external;
mod gif;
mod ico;
mod jpeg;
mod png;
mod verify;
mod webp;

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
            Arc::new(jpeg::Mozjpeg),
            Arc::new(gif::Gifsicle),
            Arc::new(webp::Webp),
            Arc::new(ico::Ico),
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

    fn bloated_apng(shift: u8) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = ::png::Encoder::new(&mut out, 48, 32);
        enc.set_color(::png::ColorType::Rgba);
        enc.set_depth(::png::BitDepth::Eight);
        enc.set_compression(::png::Compression::NoCompression);
        enc.set_animated(3, 0).unwrap();
        let mut writer = enc.write_header().unwrap();
        for frame in 0..3u8 {
            writer.set_frame_delay(frame as u16 + 1, 10).unwrap();
            let data: Vec<u8> = (0..48 * 32)
                .flat_map(|i| {
                    let (x, y) = ((i % 48) as u8, (i / 48) as u8);
                    [x.wrapping_mul(5) ^ frame.wrapping_mul(40), y.wrapping_mul(7).wrapping_add(shift), 90, 255]
                })
                .collect();
            writer.write_image_data(&data).unwrap();
        }
        writer.finish().unwrap();
        out
    }

    #[tokio::test]
    async fn apng_keeps_every_frame() {
        let input = Arc::new(bloated_apng(0));
        assert!(png::is_apng(&input));
        match process(&png::Oxipng, input.clone(), &Settings::default()).await {
            Outcome::Optimized(out) => {
                assert!(png::is_apng(&out));
                verify::same_apng(&input, &out).unwrap();
            }
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[test]
    fn apng_verifier_sees_later_frames() {
        assert!(verify::same_apng(&bloated_apng(0), &bloated_apng(0)).is_ok());
        assert!(verify::same_apng(&bloated_apng(0), &bloated_apng(1)).is_err());
        assert!(!png::is_apng(&bloated_png(&sample())));
    }

    fn sample_jpeg() -> Vec<u8> {
        let rgb = image::DynamicImage::ImageRgba8(sample()).to_rgb8();
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 92)
            .write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    /// Inserts EXIF-like APP1 and COM segments right after SOI.
    fn with_metadata(jpeg: &[u8], exif: &[u8]) -> Vec<u8> {
        let mut out = jpeg[..2].to_vec();
        for (marker, payload) in [(0xE1u8, exif), (0xFE, b"made by a test".as_slice())] {
            out.extend_from_slice(&[0xFF, marker]);
            out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(payload);
        }
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[tokio::test]
    async fn jpeg_keeps_pixels_and_metadata() {
        let input = Arc::new(with_metadata(&sample_jpeg(), b"Exif\0\0orientation=6"));
        let settings = Settings { min_savings_bytes: 1, min_savings_percent: 0.0, ..Settings::default() };
        match process(&jpeg::Mozjpeg, input.clone(), &settings).await {
            Outcome::Optimized(out) => {
                verify::same_pixels(&input, &out, ImageFormat::Jpeg).unwrap();
                let segments = jpeg::metadata_segments(&out).unwrap();
                assert!(segments.contains(&(0xE1, b"Exif\0\0orientation=6".as_slice())));
                assert!(segments.contains(&(0xFE, b"made by a test".as_slice())));
            }
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn jpeg_verifier_rejects_changed_metadata() {
        let original = Arc::new(with_metadata(&sample_jpeg(), b"Exif\0\0orientation=6"));
        let altered = Arc::new(with_metadata(&sample_jpeg(), b"Exif\0\0orientation=1"));
        assert!(jpeg::Mozjpeg.verify(original, altered).await.is_err());
    }

    #[test]
    fn damaged_jpeg_is_an_error_not_a_crash() {
        let jpeg = sample_jpeg();
        assert!(jpeg::transcode(&jpeg[..jpeg.len() / 2], true).is_err());
        assert!(jpeg::transcode(b"definitely not a jpeg", true).is_err());
        assert!(jpeg::transcode(&jpeg, true).is_ok());
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

    fn noisy_rgba(width: u32, height: u32, alpha: bool) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                let a = if alpha && (x + y) % 7 == 0 { 0 } else { 255 };
                [((x * 3) ^ y) as u8, (y * 5 + x / 3) as u8, ((x * y) % 251) as u8, a]
            })
            .collect()
    }

    /// Wraps a simple lossless WebP in an extended (VP8X) container with
    /// ICC and EXIF chunks.
    fn with_webp_metadata(simple: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut vp8x = vec![0x20 | 0x10 | 0x08, 0, 0, 0];
        vp8x.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        vp8x.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        let original = webp::chunks(simple).unwrap();
        let vp8l = original.iter().find(|c| &c.fourcc == b"VP8L").unwrap();
        let parts: [(&[u8; 4], &[u8]); 4] =
            [(b"VP8X", &vp8x), (b"ICCP", b"fake icc profile"), (b"VP8L", vp8l.payload), (b"EXIF", b"Exif\0\0odd")];
        let mut body = b"WEBP".to_vec();
        for (fourcc, payload) in parts {
            body.extend_from_slice(fourcc);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[tokio::test]
    async fn lossless_webp_keeps_pixels_and_chunks() {
        let (w, h) = (80, 60);
        let fast = webp::encode_lossless(&noisy_rgba(w, h, true), w as i32, h as i32, 0).unwrap();
        let input = Arc::new(with_webp_metadata(&fast, w, h));
        let settings = Settings { min_savings_bytes: 1, min_savings_percent: 0.0, ..Settings::default() };
        match process(&webp::Webp, input.clone(), &settings).await {
            Outcome::Optimized(out) => {
                assert!(out.len() < input.len());
                let kinds: Vec<[u8; 4]> = webp::chunks(&out).unwrap().iter().map(|c| c.fourcc).collect();
                assert_eq!(kinds, [*b"VP8X", *b"ICCP", *b"VP8L", *b"EXIF"]);
                verify::same_pixels(&input, &out, ImageFormat::WebP).unwrap();
            }
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn lossy_webp_is_never_touched() {
        let (w, h) = (32, 32);
        let rgba = noisy_rgba(w, h, false);
        let lossy = unsafe {
            let mut out = std::ptr::null_mut();
            let len = libwebp_sys::WebPEncodeRGBA(rgba.as_ptr(), w as i32, h as i32, w as i32 * 4, 80.0, &mut out);
            let bytes = std::slice::from_raw_parts(out, len).to_vec();
            libwebp_sys::WebPFree(out as *mut std::ffi::c_void);
            bytes
        };
        let outcome = process(&webp::Webp, Arc::new(lossy), &Settings::default()).await;
        assert!(matches!(&outcome, Outcome::Failed(e) if e.contains("lossy")), "{outcome:?}");
    }

    #[test]
    fn webp_verifier_rejects_changed_pixels() {
        let (w, h) = (24, 24);
        let a = webp::encode_lossless(&noisy_rgba(w, h, false), w as i32, h as i32, 3).unwrap();
        let mut pixels = noisy_rgba(w, h, false);
        pixels[0] ^= 1;
        let b = webp::encode_lossless(&pixels, w as i32, h as i32, 3).unwrap();
        assert!(verify::same_pixels(&a, &b, ImageFormat::WebP).is_err());
    }

    /// An icon with a bloated 32-bit PNG entry and a (fake) BMP entry.
    fn sample_ico() -> Vec<u8> {
        let png = bloated_png(&RgbaImage::from_fn(48, 48, |x, y| Rgba([(x * 5) as u8, (y * 5) as u8, 128, 255])));
        let bmp: Vec<u8> = (0..200u8).collect();
        let mut out = vec![0, 0, 1, 0, 2, 0];
        let mut offset = 6 + 32;
        for (size, data) in [(48u8, &png), (16, &bmp)] {
            out.extend_from_slice(&[size, size, 0, 0, 1, 0, 32, 0]);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += data.len();
        }
        out.extend_from_slice(&png);
        out.extend_from_slice(&bmp);
        out
    }

    #[tokio::test]
    async fn ico_png_entries_shrink_and_bmp_entries_stay() {
        let input = Arc::new(sample_ico());
        match process(&ico::Ico, input.clone(), &Settings::default()).await {
            Outcome::Optimized(out) => {
                assert!(out.len() < input.len());
                assert!(ico::parse(&out).is_ok());
                assert!(out.ends_with(&(0..200u8).collect::<Vec<_>>()), "BMP entry must be untouched");
                assert_eq!(icon_count(&out), 2);
            }
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    fn icon_count(data: &[u8]) -> u16 {
        u16::from_le_bytes([data[4], data[5]])
    }

    #[test]
    fn ico_with_unusual_layout_is_refused() {
        let mut ico = sample_ico();
        ico.push(0); // trailing byte
        assert!(ico::parse(&ico).is_err());
        assert!(ico::parse(b"\0\0\x01\0").is_err());
    }

    #[test]
    fn picks_optimizer_by_extension() {
        let active: Vec<Arc<dyn Optimizer>> = vec![Arc::new(png::Oxipng), Arc::new(jpeg::Mozjpeg)];
        assert_eq!(for_path(&active, "a/b/Logo.PNG").unwrap().id(), "oxipng");
        assert_eq!(for_path(&active, "photo.jpeg").unwrap().id(), "mozjpeg");
        assert!(for_path(&active, "anim.gif").is_none());
        assert!(for_path(&active, "README").is_none());
    }
}
