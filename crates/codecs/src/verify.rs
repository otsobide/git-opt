//! Pixel-exact comparison used to prove an optimization is lossless.

// Builds with only some formats leave some helpers unused.
#![cfg_attr(
    not(all(feature = "png", feature = "jpeg", feature = "gif", feature = "webp")),
    allow(dead_code, unused_imports)
)]

#[cfg(any(feature = "gif", feature = "png"))]
use std::io::Cursor;

use anyhow::{Context, Result, bail, ensure};
#[cfg(feature = "gif")]
use image::codecs::gif::GifDecoder;
#[cfg(feature = "png")]
use image::codecs::png::PngDecoder;
#[cfg(any(feature = "gif", feature = "png"))]
use image::{AnimationDecoder, Frames};
use image::{DynamicImage, ImageFormat};

fn decode(bytes: &[u8], format: ImageFormat) -> Result<DynamicImage> {
    image::load_from_memory_with_format(bytes, format).with_context(|| format!("cannot decode {format:?}"))
}

/// Both buffers decode to the same dimensions and the same RGBA values.
pub fn same_pixels(original: &[u8], candidate: &[u8], format: ImageFormat) -> Result<()> {
    let a = decode(original, format)?;
    let b = decode(candidate, format)?;
    ensure!(
        (a.width(), a.height()) == (b.width(), b.height()),
        "dimensions changed: {}x{} -> {}x{}",
        a.width(),
        a.height(),
        b.width(),
        b.height()
    );
    let equal = if a.color() == b.color() {
        a.as_bytes() == b.as_bytes()
    } else {
        // Color type was reduced (e.g. RGBA -> palette): compare in the widest form.
        a.to_rgba16().as_raw() == b.to_rgba16().as_raw()
    };
    if !equal {
        bail!("decoded pixels differ");
    }
    Ok(())
}

/// Every composited animation frame and its delay are identical.
#[cfg(feature = "gif")]
pub fn same_gif_frames(original: &[u8], candidate: &[u8]) -> Result<()> {
    let a = GifDecoder::new(Cursor::new(original)).context("cannot decode original gif")?;
    let b = GifDecoder::new(Cursor::new(candidate)).context("cannot decode optimized gif")?;
    same_frames(a.into_frames(), b.into_frames())
}

#[cfg(feature = "png")]
/// For an animated PNG: same default image, same animation control (frame
/// count and loop count) and identical composited frames with their delays.
pub fn same_apng(original: &[u8], candidate: &[u8]) -> Result<()> {
    same_pixels(original, candidate, ImageFormat::Png)?;
    ensure!(
        png_chunk(original, b"acTL") == png_chunk(candidate, b"acTL"),
        "animation control (acTL) changed"
    );
    let a = PngDecoder::new(Cursor::new(original)).context("cannot decode original apng")?;
    let b = PngDecoder::new(Cursor::new(candidate)).context("cannot decode optimized apng")?;
    ensure!(b.is_apng()?, "animation was lost");
    same_frames(a.apng()?.into_frames(), b.apng()?.into_frames())
}

/// Payload of the first PNG chunk of the given type.
#[cfg(feature = "png")]
pub fn png_chunk<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut pos = 8; // signature
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        let start = pos + 8;
        let end = start.checked_add(len)?;
        if &data[pos + 4..pos + 8] == kind {
            return data.get(start..end);
        }
        pos = end.checked_add(4)?; // skip CRC
    }
    None
}

#[cfg(any(feature = "gif", feature = "png"))]
fn same_frames(mut fa: Frames<'_>, mut fb: Frames<'_>) -> Result<()> {
    let mut index = 0usize;
    loop {
        match (fa.next(), fb.next()) {
            (None, None) => return Ok(()),
            (Some(x), Some(y)) => {
                let (x, y) = (x?, y?);
                ensure!(x.delay() == y.delay(), "frame {index}: delay changed");
                ensure!(x.buffer() == y.buffer(), "frame {index}: pixels differ");
            }
            _ => bail!("frame count changed"),
        }
        index += 1;
    }
}
