//! Lossless WebP via libwebp, built in.
//!
//! Only lossless (VP8L) files are touched: a lossy WebP cannot be recompressed
//! without loss, and animations are skipped. The pixels are re-encoded with
//! the strongest lossless preset (`cwebp -lossless -z 9 -exact`; `exact` keeps
//! the RGB values under fully transparent pixels), and the new bitstream
//! replaces the old one inside the original container, so every other chunk
//! (VP8X header, ICC profile, EXIF, XMP, unknown chunks) stays byte for byte.

use std::{ffi::c_void, os::raw::c_int, sync::Arc};

use anyhow::{Result, anyhow, bail, ensure};
use async_trait::async_trait;
use image::ImageFormat;
use libwebp_sys as ffi;

use git_opt_domain::{
    model::CodecOptions,
    ports::FormatStrategy,
    values::{FileFormat, Profile},
};

use crate::verify;

pub struct Webp;

#[async_trait]
impl FormatStrategy for Webp {
    fn id(&self) -> &'static str {
        "webp"
    }
    fn label(&self) -> &'static str {
        "WebP lossless (libwebp)"
    }
    fn formats(&self) -> &'static [FileFormat] {
        &[FileFormat::WebP]
    }
    fn explanation(&self) -> &'static str {
        "**WebP**: lossless files only, re-encoded with libwebp at maximum effort (`-lossless -z 9 -exact`); ICC, EXIF and XMP chunks are kept."
    }
    fn profile(&self, _: &CodecOptions) -> Profile {
        Profile::new("libwebp:lossless:z9:exact")
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        let v = unsafe { ffi::WebPGetEncoderVersion() };
        Ok(format!("built-in libwebp {}.{}.{}", (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff))
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, _: &CodecOptions) -> Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || recompress(&input, 9)).await?
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || {
            ensure!(
                non_image_chunks(&chunks(&original)?) == non_image_chunks(&chunks(&candidate)?),
                "WebP metadata chunks changed"
            );
            verify::same_pixels(&original, &candidate, ImageFormat::WebP)
        })
        .await?
    }
}

// ---------------------------------------------------------------- container

/// One RIFF chunk: its FourCC and payload (without padding).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Chunk<'a> {
    pub fourcc: [u8; 4],
    pub payload: &'a [u8],
}

pub(super) fn chunks(data: &[u8]) -> Result<Vec<Chunk<'_>>> {
    ensure!(data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP", "not a WebP file");
    let riff_len = u32::from_le_bytes(data[4..8].try_into()?) as usize;
    // Refuse trailing or missing bytes: we would not reproduce them faithfully.
    ensure!(riff_len + 8 == data.len(), "unusual WebP layout (RIFF size mismatch)");
    let mut out = Vec::new();
    let mut pos = 12;
    while pos < data.len() {
        ensure!(pos + 8 <= data.len(), "truncated WebP chunk header");
        let fourcc: [u8; 4] = data[pos..pos + 4].try_into()?;
        let len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into()?) as usize;
        let start = pos + 8;
        let end = start.checked_add(len).filter(|&e| e <= data.len()).ok_or_else(|| anyhow!("truncated WebP chunk"))?;
        out.push(Chunk { fourcc, payload: &data[start..end] });
        pos = end + (len & 1);
    }
    Ok(out)
}

fn non_image_chunks<'a>(chunks: &'a [Chunk<'a>]) -> Vec<&'a Chunk<'a>> {
    chunks.iter().filter(|c| &c.fourcc != b"VP8L").collect()
}

fn assemble(chunks: &[Chunk<'_>]) -> Vec<u8> {
    let body: usize = chunks.iter().map(|c| 8 + c.payload.len() + (c.payload.len() & 1)).sum();
    let mut out = Vec::with_capacity(12 + body);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + body) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    for c in chunks {
        out.extend_from_slice(&c.fourcc);
        out.extend_from_slice(&(c.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(c.payload);
        if c.payload.len() & 1 == 1 {
            out.push(0);
        }
    }
    out
}

/// Re-encodes the lossless bitstream of `input` at `level` (0–9) and puts it
/// back into the original container.
pub(super) fn recompress(input: &[u8], level: c_int) -> Result<Vec<u8>> {
    let original = chunks(input)?;
    let kinds: Vec<&[u8; 4]> = original.iter().map(|c| &c.fourcc).collect();
    if kinds.iter().any(|k| *k == b"ANIM" || *k == b"ANMF") {
        bail!("animated WebP is not supported");
    }
    if kinds.iter().any(|k| *k == b"VP8 " || *k == b"ALPH") {
        bail!("lossy WebP cannot be recompressed without loss");
    }
    ensure!(kinds.iter().filter(|k| **k == b"VP8L").count() == 1, "expected one lossless (VP8L) bitstream");

    let (rgba, width, height) = decode_rgba(input)?;
    let encoded = encode_lossless(&rgba, width, height, level)?;
    let fresh = chunks(&encoded)?;
    let bitstream =
        fresh.iter().find(|c| &c.fourcc == b"VP8L").ok_or_else(|| anyhow!("libwebp produced no lossless bitstream"))?;

    let rebuilt: Vec<Chunk<'_>> =
        original
            .iter()
            .map(|c| {
                if &c.fourcc == b"VP8L" {
                    Chunk { fourcc: *b"VP8L", payload: bitstream.payload }
                } else {
                    Chunk { ..*c }
                }
            })
            .collect();
    Ok(assemble(&rebuilt))
}

fn decode_rgba(input: &[u8]) -> Result<(Vec<u8>, c_int, c_int)> {
    let (mut width, mut height) = (0, 0);
    unsafe {
        let pixels = ffi::WebPDecodeRGBA(input.as_ptr(), input.len(), &mut width, &mut height);
        ensure!(!pixels.is_null(), "libwebp could not decode the image");
        let len = width as usize * height as usize * 4;
        let rgba = std::slice::from_raw_parts(pixels, len).to_vec();
        ffi::WebPFree(pixels as *mut c_void);
        Ok((rgba, width, height))
    }
}

pub(super) fn encode_lossless(rgba: &[u8], width: c_int, height: c_int, level: c_int) -> Result<Vec<u8>> {
    ensure!(rgba.len() == width as usize * height as usize * 4, "pixel buffer size mismatch");
    unsafe {
        let mut config = std::mem::MaybeUninit::<ffi::WebPConfig>::uninit();
        ensure!(
            ffi::WebPConfigInitInternal(
                config.as_mut_ptr(),
                ffi::WebPPreset::WEBP_PRESET_DEFAULT,
                75.0,
                ffi::WEBP_ENCODER_ABI_VERSION as c_int,
            ) != 0,
            "libwebp version mismatch"
        );
        let mut config = config.assume_init();
        ensure!(ffi::WebPConfigLosslessPreset(&mut config, level) != 0, "invalid lossless preset");
        config.lossless = 1;
        config.exact = 1; // keep RGB under transparent pixels
        ensure!(ffi::WebPValidateConfig(&config) != 0, "invalid libwebp configuration");

        let mut picture = std::mem::MaybeUninit::<ffi::WebPPicture>::uninit();
        ensure!(
            ffi::WebPPictureInitInternal(picture.as_mut_ptr(), ffi::WEBP_ENCODER_ABI_VERSION as c_int) != 0,
            "libwebp version mismatch"
        );
        let mut picture = picture.assume_init();
        picture.use_argb = 1;
        picture.width = width;
        picture.height = height;
        let mut writer = std::mem::MaybeUninit::<ffi::WebPMemoryWriter>::uninit();
        ffi::WebPMemoryWriterInit(writer.as_mut_ptr());
        let mut writer = writer.assume_init();

        let result = (|| {
            ensure!(ffi::WebPPictureImportRGBA(&mut picture, rgba.as_ptr(), width * 4) != 0, "out of memory");
            picture.writer = Some(ffi::WebPMemoryWrite);
            picture.custom_ptr = &mut writer as *mut ffi::WebPMemoryWriter as *mut c_void;
            ensure!(ffi::WebPEncode(&config, &mut picture) != 0, "libwebp failed: {:?}", picture.error_code);
            ensure!(!writer.mem.is_null() && writer.size > 0, "libwebp produced no output");
            Ok(std::slice::from_raw_parts(writer.mem, writer.size).to_vec())
        })();
        ffi::WebPPictureFree(&mut picture);
        ffi::WebPMemoryWriterClear(&mut writer);
        result
    }
}
