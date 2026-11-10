//! SVGZ: gzip-compressed SVG.
//!
//! The SVG inside gets exactly the same cleanup and proof as a `.svg` file
//! (if the cleanup cannot be proven, the content is kept as is), and is then
//! recompressed with Zopfli. The gzip header (file name, comment, time,
//! flags, extra field) is copied byte for byte; only the deflate stream and
//! the CRC/size trailer change. Multi-member files, trailing bytes and bad
//! checksums are refused.

use std::{io::Read, sync::Arc};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;

use git_opt_domain::{
    model::CodecOptions,
    ports::FormatStrategy,
    values::{FileFormat, Profile},
};

use crate::svg::Svg;

/// Refuse to inflate anything bigger than this (decompression bombs).
const MAX_CONTENT: u64 = 64 << 20;
/// Above this size Zopfli gets too slow; use zlib's best level instead.
const ZOPFLI_LIMIT: usize = 8 << 20;

/// Wraps the SVG strategy (injected) with gzip handling.
pub struct Svgz {
    svg: Arc<Svg>,
}

impl Svgz {
    pub fn new(svg: Arc<Svg>) -> Self {
        Self { svg }
    }
}

#[async_trait]
impl FormatStrategy for Svgz {
    fn id(&self) -> &'static str {
        "svgz"
    }
    fn label(&self) -> &'static str {
        "SVGZ (gzip-compressed SVG)"
    }
    fn formats(&self) -> &'static [FileFormat] {
        &[FileFormat::Svgz]
    }
    fn explanation(&self) -> &'static str {
        "**SVGZ**: the SVG inside cleaned as above and recompressed with Zopfli; the gzip header (file name, time) is unchanged."
    }
    fn profile(&self, s: &CodecOptions) -> Profile {
        Profile::new(format!("svgz:zopfli:{}", self.svg.profile(s)))
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        Ok("built-in, SVG cleanup + Zopfli".into())
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, s: &CodecOptions) -> Result<Vec<u8>> {
        let (gzip, input) = tokio::task::spawn_blocking(move || Gzip::parse(&input).map(|g| (g, input))).await??;
        let original = Arc::new(gzip.content);

        // Same rules as a plain .svg; keep the content if the cleanup can't be proven.
        let content = match self.svg.optimize(original.clone(), s).await {
            Ok(cleaned) if cleaned != *original => {
                let cleaned = Arc::new(cleaned);
                match self.svg.verify(original.clone(), cleaned.clone()).await {
                    Ok(()) => cleaned,
                    Err(_) => original,
                }
            }
            _ => original,
        };

        tokio::task::spawn_blocking(move || {
            let mut out = input[..gzip.header_len].to_vec();
            out.extend_from_slice(&deflate(&content)?);
            let mut crc = flate2::Crc::new();
            crc.update(&content);
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&(content.len() as u32).to_le_bytes());
            Ok(out)
        })
        .await?
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        let (a, b) = tokio::task::spawn_blocking(move || -> Result<_> {
            let (a, b) = (Gzip::parse(&original)?, Gzip::parse(&candidate)?);
            ensure!(original[..a.header_len] == candidate[..b.header_len], "gzip header changed");
            Ok((a.content, b.content))
        })
        .await??;
        if a != b {
            self.svg.verify(Arc::new(a), Arc::new(b)).await.context("compressed SVG")?;
        }
        Ok(())
    }
}

fn deflate(content: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    if content.len() <= ZOPFLI_LIMIT {
        zopfli::compress(zopfli::Options::default(), zopfli::Format::Deflate, content, &mut out)?;
    } else {
        use std::io::Write;
        let mut encoder = flate2::write::DeflateEncoder::new(&mut out, flate2::Compression::best());
        encoder.write_all(content)?;
        encoder.finish()?;
    }
    Ok(out)
}

pub(super) struct Gzip {
    /// Bytes before the deflate stream (copied verbatim when rewriting).
    pub header_len: usize,
    pub content: Vec<u8>,
}

impl Gzip {
    /// A single-member gzip file with a valid checksum and nothing after it.
    pub(super) fn parse(data: &[u8]) -> Result<Self> {
        ensure!(data.len() >= 18 && data[..3] == [0x1f, 0x8b, 8], "not a gzip file");
        let flags = data[3];
        ensure!(flags & 0xE0 == 0, "unknown gzip flags");
        let mut pos = 10;
        if flags & 0x04 != 0 {
            // FEXTRA
            ensure!(pos + 2 <= data.len(), "truncated gzip header");
            pos += 2 + u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        }
        for flag in [0x08, 0x10] {
            // FNAME, FCOMMENT: zero-terminated strings
            if flags & flag != 0 {
                let nul = data.get(pos..).and_then(|r| r.iter().position(|&b| b == 0));
                pos += nul.context("truncated gzip header")? + 1;
            }
        }
        if flags & 0x02 != 0 {
            pos += 2; // FHCRC
        }
        ensure!(pos < data.len(), "truncated gzip header");

        let mut decoder = flate2::read::DeflateDecoder::new(&data[pos..]);
        let mut content = Vec::new();
        (&mut decoder).take(MAX_CONTENT + 1).read_to_end(&mut content).context("corrupt gzip data")?;
        ensure!(content.len() as u64 <= MAX_CONTENT, "compressed SVG is too large");
        let trailer = pos + decoder.total_in() as usize;
        ensure!(trailer + 8 == data.len(), "multi-member gzip or trailing data");

        let crc = u32::from_le_bytes(data[trailer..trailer + 4].try_into()?);
        let size = u32::from_le_bytes(data[trailer + 4..trailer + 8].try_into()?);
        let mut actual = flate2::Crc::new();
        actual.update(&content);
        ensure!(actual.sum() == crc && content.len() as u32 == size, "gzip checksum mismatch");
        Ok(Self { header_len: pos, content })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use git_opt_domain::{
        ports::{Outcome, attempt},
        values::SavingsThreshold,
    };

    fn svgz() -> Svgz {
        Svgz::new(Arc::new(Svg::new(crate::raster_strategies())))
    }

    const INKSCAPE: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!-- Created with Inkscape (http://www.inkscape.org/) -->
<svg
   width="64"
   height="64"
   viewBox="0 0 64 64"
   inkscape:version="1.3"
   xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
   xmlns="http://www.w3.org/2000/svg">
  <g
     inkscape:label="Layer 1"
     inkscape:groupmode="layer">
    <rect
       style="fill:#d97706"
       width="40"
       height="30"
       x="12"
       y="17" />
  </g>
</svg>
"##;

    fn gzip(content: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = flate2::GzBuilder::new()
            .filename("drawing.svg")
            .mtime(1_700_000_000)
            .write(&mut out, flate2::Compression::fast());
        encoder.write_all(content).unwrap();
        encoder.finish().unwrap();
        out
    }

    fn gunzip(data: &[u8]) -> Vec<u8> {
        Gzip::parse(data).unwrap().content
    }

    #[tokio::test]
    async fn svgz_is_cleaned_recompressed_and_keeps_its_header() {
        let input = gzip(INKSCAPE.as_bytes());
        let header_len = Gzip::parse(&input).unwrap().header_len;
        let settings = CodecOptions { threshold: SavingsThreshold::any_reduction(), ..CodecOptions::default() };
        match attempt(&svgz(), Arc::new(input.clone()), &settings).await {
            Outcome::Optimized(out) => {
                assert!(out.len() < input.len());
                assert_eq!(out[..header_len], input[..header_len], "file name and time must survive");
                let svg = String::from_utf8(gunzip(&out)).unwrap();
                assert!(!svg.contains("inkscape") && svg.contains(r#"style="fill:#d97706""#), "{svg}");
            }
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn content_that_cannot_be_cleaned_is_only_recompressed() {
        // Not XML at all: the bytes must come back identical, just packed tighter.
        let content: Vec<u8> = (0..20_000u32).map(|i| b"abcdefgh"[(i * i % 7) as usize]).collect();
        let input = gzip(&content);
        let settings = CodecOptions { threshold: SavingsThreshold::any_reduction(), ..CodecOptions::default() };
        match attempt(&svgz(), Arc::new(input.clone()), &settings).await {
            Outcome::Optimized(out) => assert_eq!(gunzip(&out), content),
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[test]
    fn unusual_gzip_files_are_refused() {
        let good = gzip(INKSCAPE.as_bytes());
        let mut two_members = good.clone();
        two_members.extend_from_slice(&good);
        assert!(Gzip::parse(&two_members).is_err());
        let mut bad_crc = good.clone();
        let at = bad_crc.len() - 8;
        bad_crc[at] ^= 1;
        assert!(Gzip::parse(&bad_crc).is_err());
        assert!(Gzip::parse(&good[..good.len() - 1]).is_err());
        assert!(Gzip::parse(b"<svg/>").is_err());
    }

    #[tokio::test]
    async fn verifier_rejects_changed_content_or_header() {
        let original = Arc::new(gzip(INKSCAPE.as_bytes()));
        let changed = Arc::new(gzip(INKSCAPE.replace(r#"width="40""#, r#"width="41""#).as_bytes()));
        assert!(svgz().verify(original.clone(), changed).await.is_err());
        let mut renamed = Vec::new();
        let mut encoder = flate2::GzBuilder::new()
            .filename("other.svg")
            .mtime(1_700_000_000)
            .write(&mut renamed, flate2::Compression::fast());
        encoder.write_all(INKSCAPE.as_bytes()).unwrap();
        encoder.finish().unwrap();
        assert!(svgz().verify(original, Arc::new(renamed)).await.is_err());
    }
}
