//! Cross-strategy tests: every optimizer shrinks a sample, its result is
//! proven equivalent, and its verifier rejects a minimal change.

use std::sync::Arc;

use git_opt_domain::{
    model::CodecOptions,
    ports::{FormatStrategy, Outcome, attempt},
    values::SavingsThreshold,
};

// `::png` below is the png crate used to build fixtures; `png` is our strategy.
use crate::{gif, icns, ico, jpeg, png, verify, webp};
use image::{
    ExtendedColorType, ImageEncoder, ImageFormat, Rgba, RgbaImage,
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
    },
};

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
    match attempt(&png::Oxipng, input.clone(), &CodecOptions::default()).await {
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
    let settings = CodecOptions { png_zopfli: true, ..CodecOptions::default() };
    assert_ne!(png::Oxipng.profile(&settings), png::Oxipng.profile(&CodecOptions::default()));
    match attempt(&png::Oxipng, input.clone(), &settings).await {
        Outcome::Optimized(out) => verify::same_pixels(&input, &out, ImageFormat::Png).unwrap(),
        other => panic!("expected optimization, got {other:?}"),
    }
}

#[tokio::test]
async fn already_optimal_png_is_left_alone() {
    let first = match attempt(&png::Oxipng, Arc::new(bloated_png(&sample())), &CodecOptions::default()).await {
        Outcome::Optimized(out) => out,
        other => panic!("{other:?}"),
    };
    let second = attempt(&png::Oxipng, Arc::new(first), &CodecOptions::default()).await;
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
    match attempt(&png::Oxipng, input.clone(), &CodecOptions::default()).await {
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
    let settings = CodecOptions { threshold: SavingsThreshold::any_reduction(), ..CodecOptions::default() };
    match attempt(&jpeg::Mozjpeg, input.clone(), &settings).await {
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
    let settings = CodecOptions { threshold: SavingsThreshold::any_reduction(), ..CodecOptions::default() };
    match attempt(&gif::Gifsicle, input.clone(), &settings).await {
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
    let settings = CodecOptions { threshold: SavingsThreshold::any_reduction(), ..CodecOptions::default() };
    match attempt(&webp::Webp, input.clone(), &settings).await {
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
    let outcome = attempt(&webp::Webp, Arc::new(lossy), &CodecOptions::default()).await;
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
    match attempt(&ico::Ico, input.clone(), &CodecOptions::default()).await {
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

/// An ICNS with a table of contents, a version element, a bloated PNG
/// image and a (fake) RLE image.
fn sample_icns() -> Vec<u8> {
    let png = bloated_png(&RgbaImage::from_fn(128, 128, |x, y| Rgba([(x * 2) as u8, (y * 2) as u8, 64, 255])));
    let rle: Vec<u8> = (0..120u8).collect();
    let elements: Vec<(&[u8; 4], Vec<u8>)> =
        vec![(b"icnV", vec![0x42, 0x00, 0x00, 0x00]), (b"ic07", png), (b"is32", rle)];
    let toc: Vec<u8> =
        elements.iter().flat_map(|(k, d)| k.iter().copied().chain(((d.len() + 8) as u32).to_be_bytes())).collect();
    let mut body = Vec::new();
    for (kind, data) in std::iter::once((b"TOC ", toc)).chain(elements) {
        body.extend_from_slice(kind);
        body.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
        body.extend_from_slice(&data);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

#[tokio::test]
async fn icns_png_elements_shrink_and_toc_follows() {
    let input = Arc::new(sample_icns());
    match attempt(&icns::Icns, input.clone(), &CodecOptions::default()).await {
        Outcome::Optimized(out) => {
            assert!(out.len() < input.len());
            let elements = icns::parse(&out).unwrap();
            let kinds: Vec<&[u8; 4]> = elements.iter().map(|e| &e.kind).collect();
            assert_eq!(kinds, [b"TOC ", b"icnV", b"ic07", b"is32"]);
            assert_eq!(elements[3].data, (0..120u8).collect::<Vec<_>>().as_slice());
            let toc_png_len = u32::from_be_bytes(elements[0].data[12..16].try_into().unwrap()) as usize;
            assert_eq!(toc_png_len, elements[2].data.len() + 8, "TOC must carry the new PNG size");
        }
        other => panic!("expected optimization, got {other:?}"),
    }
}

#[tokio::test]
async fn icns_verifier_rejects_changed_elements() {
    let original = sample_icns();
    let mut altered = original.clone();
    let last = altered.len() - 1;
    altered[last] ^= 1; // inside the RLE element
    assert!(icns::Icns.verify(Arc::new(original.clone()), Arc::new(altered)).await.is_err());
    let mut truncated = original;
    truncated.pop();
    assert!(icns::parse(&truncated).is_err());
}
