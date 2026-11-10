//! ICNS (macOS icons): recompress the PNG images stored in the icon family.
//!
//! An ICNS file is a list of typed elements. Elements holding a PNG are
//! recompressed keeping their pixel format; every other element (RLE images,
//! masks, JPEG 2000, `icnV`, `info`, nested dark-mode families…) is copied
//! byte for byte. The optional `TOC ` element, which lists the size of each
//! following element, is rebuilt with the new sizes.

use std::sync::Arc;

use anyhow::{Result, anyhow, ensure};
use async_trait::async_trait;

use git_opt_domain::{
    model::CodecOptions,
    ports::FormatStrategy,
    values::{FileFormat, Profile},
};

use crate::png;

const TOC: [u8; 4] = *b"TOC ";

pub struct Icns;

#[async_trait]
impl FormatStrategy for Icns {
    fn id(&self) -> &'static str {
        "icns"
    }
    fn label(&self) -> &'static str {
        "ICNS (embedded PNGs)"
    }
    fn formats(&self) -> &'static [FileFormat] {
        &[FileFormat::Icns]
    }
    fn explanation(&self) -> &'static str {
        "**ICNS**: embedded PNG images recompressed with oxipng, keeping their pixel format; every other element untouched."
    }
    fn profile(&self, s: &CodecOptions) -> Profile {
        Profile::new(format!("icns:{}", png::keep_format_profile(s)))
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        Ok("built-in, oxipng on embedded PNGs".into())
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, s: &CodecOptions) -> Result<Vec<u8>> {
        let opts = png::keep_format_options(s);
        tokio::task::spawn_blocking(move || {
            let elements = parse(&input)?;
            ensure!(elements.iter().any(Element::is_png), "no embedded PNG images");
            ensure!(toc_is_consistent(&elements), "unusual ICNS table of contents");
            let mut rebuilt = Vec::with_capacity(elements.len());
            for e in &elements {
                let data = if e.is_png() { png::recompress_keep_format(e.data, &opts)? } else { e.data.to_vec() };
                rebuilt.push((e.kind, data));
            }
            Ok(build(rebuilt))
        })
        .await?
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || {
            let (a, b) = (parse(&original)?, parse(&candidate)?);
            ensure!(a.iter().map(|e| e.kind).eq(b.iter().map(|e| e.kind)), "the list of icon elements changed");
            ensure!(toc_is_consistent(&b), "table of contents does not match the elements");
            for (x, y) in a.iter().zip(&b) {
                let kind = String::from_utf8_lossy(&x.kind);
                if x.kind == TOC {
                    continue; // checked above against the actual elements
                } else if x.is_png() {
                    png::verify_keep_format(x.data, y.data).map_err(|e| e.context(format!("element {kind}")))?;
                } else {
                    ensure!(x.data == y.data, "element {kind} changed");
                }
            }
            Ok(())
        })
        .await?
    }
}

pub(super) struct Element<'a> {
    pub kind: [u8; 4],
    pub data: &'a [u8],
}

impl Element<'_> {
    fn is_png(&self) -> bool {
        self.data.starts_with(png::SIGNATURE)
    }
}

/// Parses the element list. The declared total size must match the file and
/// the elements must tile it exactly; anything else is refused.
pub(super) fn parse(data: &[u8]) -> Result<Vec<Element<'_>>> {
    ensure!(data.len() >= 8 && &data[..4] == b"icns", "not an ICNS file");
    let total = u32::from_be_bytes(data[4..8].try_into()?) as usize;
    ensure!(total == data.len(), "unusual ICNS layout (size mismatch)");
    let mut elements = Vec::new();
    let mut pos = 8;
    while pos < data.len() {
        ensure!(pos + 8 <= data.len(), "truncated ICNS element header");
        let kind: [u8; 4] = data[pos..pos + 4].try_into()?;
        let len = u32::from_be_bytes(data[pos + 4..pos + 8].try_into()?) as usize;
        let end = pos
            .checked_add(len)
            .filter(|&e| len >= 8 && e <= data.len())
            .ok_or_else(|| anyhow!("truncated ICNS element"))?;
        elements.push(Element { kind, data: &data[pos + 8..end] });
        pos = end;
    }
    Ok(elements)
}

/// `TOC ` entries, as (type, size including the 8-byte header).
fn toc_entries(data: &[u8]) -> Option<Vec<([u8; 4], u32)>> {
    if !data.len().is_multiple_of(8) {
        return None;
    }
    data.chunks(8).map(|c| Some((c[..4].try_into().ok()?, u32::from_be_bytes(c[4..].try_into().ok()?)))).collect()
}

/// A `TOC `, if present, lists exactly the elements that follow it.
fn toc_is_consistent(elements: &[Element<'_>]) -> bool {
    let Some(at) = elements.iter().position(|e| e.kind == TOC) else { return true };
    let actual: Vec<([u8; 4], u32)> = elements[at + 1..].iter().map(|e| (e.kind, e.data.len() as u32 + 8)).collect();
    toc_entries(elements[at].data).is_some_and(|toc| toc == actual)
}

fn build(mut elements: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    if let Some(at) = elements.iter().position(|(kind, _)| *kind == TOC) {
        let toc: Vec<u8> = elements[at + 1..]
            .iter()
            .flat_map(|(kind, data)| kind.iter().copied().chain(((data.len() + 8) as u32).to_be_bytes()))
            .collect();
        elements[at].1 = toc;
    }
    let total: usize = 8 + elements.iter().map(|(_, d)| d.len() + 8).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    for (kind, data) in elements {
        out.extend_from_slice(&kind);
        out.extend_from_slice(&((data.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(&data);
    }
    out
}
