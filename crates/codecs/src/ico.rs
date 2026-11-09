//! ICO and CUR: recompress the PNG images embedded in the icon directory.
//!
//! Only the compression of embedded PNGs changes. Their pixel format (IHDR)
//! is kept, because some Windows icon loaders only accept 32-bit RGBA PNGs,
//! and BMP entries are copied untouched (turning them into PNG would break
//! older readers). The directory entries keep every field except the size and
//! offset of the data they point to.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail, ensure};
use async_trait::async_trait;

use git_opt_domain::{
    model::CodecOptions,
    ports::FormatStrategy,
    values::{FileFormat, Profile},
};

use crate::png;

pub struct Ico;

#[async_trait]
impl FormatStrategy for Ico {
    fn id(&self) -> &'static str {
        "ico"
    }
    fn label(&self) -> &'static str {
        "ICO/CUR (embedded PNGs)"
    }
    fn formats(&self) -> &'static [FileFormat] {
        &[FileFormat::Ico]
    }
    fn explanation(&self) -> &'static str {
        "**ICO/CUR**: embedded PNG images recompressed with oxipng, keeping their pixel format; BMP images untouched."
    }
    fn profile(&self, s: &CodecOptions) -> Profile {
        Profile::new(format!("ico:{}", png::keep_format_profile(s)))
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        Ok("built-in, oxipng on embedded PNGs".into())
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, s: &CodecOptions) -> Result<Vec<u8>> {
        let opts = png::keep_format_options(s);
        tokio::task::spawn_blocking(move || {
            let icon = parse(&input)?;
            ensure!(icon.entries.iter().any(|e| e.is_png()), "no embedded PNG images");
            let mut payloads = Vec::with_capacity(icon.entries.len());
            for entry in &icon.entries {
                let payload = if entry.is_png() {
                    png::recompress_keep_format(entry.data, &opts)?
                } else {
                    entry.data.to_vec()
                };
                payloads.push(payload);
            }
            Ok(build(&icon, &payloads))
        })
        .await?
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || {
            let (a, b) = (parse(&original)?, parse(&candidate)?);
            ensure!(a.header == b.header, "icon header changed");
            ensure!(a.entries.len() == b.entries.len(), "number of images changed");
            for (i, (x, y)) in a.entries.iter().zip(&b.entries).enumerate() {
                ensure!(x.fields == y.fields, "image {i}: directory entry changed");
                if x.is_png() {
                    png::verify_keep_format(x.data, y.data).map_err(|e| e.context(format!("image {i}")))?;
                } else {
                    ensure!(x.data == y.data, "image {i}: BMP data changed");
                }
            }
            Ok(())
        })
        .await?
    }
}

pub(super) struct Entry<'a> {
    /// Width, height, colors, reserved, planes/hotspot and bit count/hotspot.
    fields: [u8; 8],
    data: &'a [u8],
}

impl Entry<'_> {
    fn is_png(&self) -> bool {
        self.data.starts_with(png::SIGNATURE)
    }
}

pub(super) struct Icon<'a> {
    header: [u8; 6],
    entries: Vec<Entry<'a>>,
}

/// Parses an ICO/CUR whose images are stored back to back, in directory
/// order, right after the directory: the layout every encoder produces. Any
/// other layout (gaps, overlaps, trailing bytes) is refused rather than
/// silently normalized.
pub(super) fn parse(data: &[u8]) -> Result<Icon<'_>> {
    ensure!(data.len() >= 6, "not an icon file");
    let header: [u8; 6] = data[..6].try_into()?;
    let kind = u16::from_le_bytes([header[2], header[3]]);
    ensure!(header[..2] == [0, 0] && (kind == 1 || kind == 2), "not an ICO/CUR file");
    let count = u16::from_le_bytes([header[4], header[5]]) as usize;
    ensure!(count > 0, "empty icon");
    let mut expected = 6 + 16 * count;
    ensure!(data.len() >= expected, "truncated icon directory");

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let raw = &data[6 + 16 * i..6 + 16 * (i + 1)];
        let size = u32::from_le_bytes(raw[8..12].try_into()?) as usize;
        let offset = u32::from_le_bytes(raw[12..16].try_into()?) as usize;
        if offset != expected {
            bail!("unusual icon layout");
        }
        let end = offset.checked_add(size).filter(|&e| e <= data.len()).ok_or_else(|| anyhow!("truncated icon image"))?;
        entries.push(Entry { fields: raw[..8].try_into()?, data: &data[offset..end] });
        expected = end;
    }
    ensure!(expected == data.len(), "unusual icon layout (trailing data)");
    Ok(Icon { header, entries })
}

fn build(icon: &Icon<'_>, payloads: &[Vec<u8>]) -> Vec<u8> {
    let mut out = icon.header.to_vec();
    let mut offset = 6 + 16 * payloads.len();
    for (entry, payload) in icon.entries.iter().zip(payloads) {
        out.extend_from_slice(&entry.fields);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += payload.len();
    }
    for payload in payloads {
        out.extend_from_slice(payload);
    }
    out
}
