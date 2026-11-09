//! Pixel-exact comparison used to prove an optimization is lossless.

use std::io::Cursor;

use anyhow::{Context, Result, bail, ensure};
use image::{AnimationDecoder, DynamicImage, ImageFormat, codecs::gif::GifDecoder};

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
pub fn same_gif_frames(original: &[u8], candidate: &[u8]) -> Result<()> {
    let a = GifDecoder::new(Cursor::new(original)).context("cannot decode original gif")?;
    let b = GifDecoder::new(Cursor::new(candidate)).context("cannot decode optimized gif")?;
    let mut fa = a.into_frames();
    let mut fb = b.into_frames();
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
