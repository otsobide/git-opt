//! SVG: a conservative, structure-preserving cleanup.
//!
//! Removed, because it cannot change rendering, CSS or scripts:
//! - comments, except legal ones (`<!--! … -->`, or mentioning a license or
//!   copyright);
//! - elements and attributes in editor namespaces (Inkscape, Sodipodi, Adobe
//!   Illustrator, Sketch, Figma, Serif, Visio…) and the declarations of those
//!   namespaces;
//! - whitespace-only text between elements, except inside text content
//!   (`text`, `tspan`, `style`, `script`, `title`, `desc`, `metadata`…) or under
//!   `xml:space="preserve"`;
//! - redundant whitespace inside tags.
//!
//! Raster images embedded as `data:image/(png|jpeg|gif|webp);base64,` URIs
//! are optimized by their own optimizer (with its own proof) and re-encoded
//! in base64 without line breaks. Data URIs inside CSS (`url(data:…)`) and
//! embedded SVG documents are left alone.
//!
//! Never touched: numbers, path data, ids, classes, styles, any other
//! attribute value, element order, the XML declaration, the DOCTYPE, CDATA
//! and processing instructions.
//!
//! Proof of equivalence, with three checks:
//! 1. a different XML parser (roxmltree) must see the same tree in both files
//!    once the removals above are applied to the original (embedded images
//!    compared by type only);
//! 2. every embedded image that changed passes its own format's verification;
//! 3. resvg must render both files, embedded images included, to identical
//!    pixels.

use std::{borrow::Cow, collections::HashMap, ops::Range, sync::Arc};

use anyhow::{Context, Result, anyhow, bail, ensure};
use async_trait::async_trait;
use base64::Engine;
use resvg::{tiny_skia, usvg};
use xmlparser::{ElementEnd, EntityDefinition, Token, Tokenizer};

use super::{Optimizer, Outcome, gif, jpeg, png, process, webp};
use crate::settings::Settings;

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const BOM: &str = "\u{feff}";

/// Namespaces written by editors for their own bookkeeping.
const EDITOR_NAMESPACES: &[&str] = &[
    "http://www.inkscape.org/namespaces/inkscape",
    "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd",
    "http://ns.adobe.com/AdobeIllustrator/10.0/",
    "http://ns.adobe.com/AdobeSVGViewerExtensions/3.0/",
    "http://ns.adobe.com/Extensibility/1.0/",
    "http://ns.adobe.com/Flows/1.0/",
    "http://ns.adobe.com/GenericCustomNamespace/1.0/",
    "http://ns.adobe.com/Graphs/1.0/",
    "http://ns.adobe.com/ImageReplacement/1.0/",
    "http://ns.adobe.com/SaveForWeb/1.0/",
    "http://ns.adobe.com/Variables/1.0/",
    "http://ns.adobe.com/XPath/1.0/",
    "http://www.bohemiancoding.com/sketch/ns",
    "http://www.figma.com/figma/ns",
    "http://www.serif.com/",
    "http://schemas.microsoft.com/visio/2003/SVGExtensions/",
    "http://taptrix.com/vectorillustrator/svg_extensions",
    "http://www.vector.evaxdesign.sk",
];

/// Elements whose whitespace is (or may be) content.
const TEXT_ELEMENTS: &[&str] =
    &["text", "tspan", "textPath", "title", "desc", "style", "script", "foreignObject", "metadata", "pre"];

fn is_editor_ns(uri: &str) -> bool {
    EDITOR_NAMESPACES.contains(&uri)
}

fn is_legal_comment(text: &str) -> bool {
    let lower = text.to_lowercase();
    text.starts_with('!')
        || ["copyright", "license", "licence", "©", "(c)", "spdx"].iter().any(|w| lower.contains(w))
}

fn is_xml_whitespace(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

pub struct Svg;

#[async_trait]
impl Optimizer for Svg {
    fn id(&self) -> &'static str {
        "svg"
    }
    fn label(&self) -> &'static str {
        "SVG (safe cleanup)"
    }
    fn category(&self) -> &'static str {
        "image"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["svg"]
    }
    fn profile(&self, s: &Settings) -> String {
        // Embedded images make the output depend on the raster optimizers too.
        let embedded: Vec<String> = EMBEDDED
            .iter()
            .map(|o| if s.optimizer_enabled(o.id(), o.enabled_by_default()) { o.profile(s) } else { format!("{}:off", o.id()) })
            .collect();
        format!("svg-clean:v2+{}", embedded.join("+"))
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        Ok("built-in, verified with resvg".into())
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, s: &Settings) -> Result<Vec<u8>> {
        let mut text = tokio::task::spawn_blocking(move || {
            let text = std::str::from_utf8(&input).context("SVG is not UTF-8")?;
            clean(text)
        })
        .await??;

        // Replace from the end so earlier ranges stay valid.
        for uri in data_uris(&text)?.into_iter().rev() {
            let Some(optimizer) = embedded_optimizer(&uri.mime) else { continue };
            if !s.optimizer_enabled(optimizer.id(), optimizer.enabled_by_default()) {
                continue;
            }
            if let Outcome::Optimized(bytes) = process(optimizer, Arc::new(uri.bytes), s).await {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                text.replace_range(uri.range, &format!("{}{encoded}", escape_attr(&uri.prefix)));
            }
        }
        Ok(text.into_bytes())
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        let changed = tokio::task::spawn_blocking(move || {
            let (a, b) = (std::str::from_utf8(&original)?, std::str::from_utf8(&candidate)?);
            ensure!(canonical(a)? == canonical(b)?, "the XML tree changed");
            let (ra, rb) = (render(&original)?, render(&candidate)?);
            ensure!(
                (ra.width(), ra.height()) == (rb.width(), rb.height()) && ra.data() == rb.data(),
                "rendering changed"
            );
            let (ea, eb) = (embedded_images(a)?, embedded_images(b)?);
            ensure!(ea.len() == eb.len(), "number of embedded images changed");
            let mut changed = Vec::new();
            for ((mime_a, bytes_a), (mime_b, bytes_b)) in ea.into_iter().zip(eb) {
                ensure!(mime_a == mime_b, "embedded image type changed");
                if bytes_a != bytes_b {
                    changed.push((mime_a, bytes_a, bytes_b));
                }
            }
            Ok(changed)
        })
        .await??;

        for (index, (mime, before, after)) in changed.into_iter().enumerate() {
            let optimizer = embedded_optimizer(&mime).ok_or_else(|| anyhow!("unexpected embedded {mime}"))?;
            optimizer
                .verify(Arc::new(before), Arc::new(after))
                .await
                .with_context(|| format!("embedded image {index} ({mime})"))?;
        }
        Ok(())
    }
}

// --------------------------------------------------------- embedded images

/// Raster optimizers reused for images embedded as data URIs.
const EMBEDDED: [&dyn Optimizer; 4] = [&png::Oxipng, &jpeg::Mozjpeg, &webp::Webp, &gif::Gifsicle];

fn embedded_optimizer(mime: &str) -> Option<&'static dyn Optimizer> {
    let id = match mime {
        "image/png" => "oxipng",
        "image/jpeg" | "image/jpg" => "mozjpeg",
        "image/webp" => "webp",
        "image/gif" => "gifsicle",
        _ => return None,
    };
    EMBEDDED.iter().copied().find(|o| o.id() == id)
}

/// A whole attribute value of the form `data:image/<type>[;…];base64,<data>`.
struct DataUri {
    /// Everything up to and including the comma, unescaped.
    prefix: String,
    mime: String,
    bytes: Vec<u8>,
}

fn parse_data_uri(value: &str) -> Option<DataUri> {
    let rest = value.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let lower = meta.to_ascii_lowercase();
    let mime = lower.strip_suffix(";base64")?.split(';').next()?.to_string();
    embedded_optimizer(&mime)?;
    let compact: String = payload.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(compact).ok()?;
    Some(DataUri { prefix: format!("data:{meta},"), mime, bytes })
}

/// Resolves the predefined XML entities and character references. `None` if
/// the value uses other (DTD) entities: such values are left alone.
fn unescape(raw: &str) -> Option<Cow<'_, str>> {
    if !raw.contains('&') {
        return Some(Cow::Borrowed(raw));
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(';')?;
        let entity = &after[..end];
        out.push(match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => match entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                Some(hex) => char::from_u32(u32::from_str_radix(hex, 16).ok()?)?,
                None => char::from_u32(entity.strip_prefix('#')?.parse().ok()?)?,
            },
        });
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(Cow::Owned(out))
}

fn escape_attr(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Embedded raster images in `text`, with the byte range of each raw
/// attribute value.
fn data_uris(text: &str) -> Result<Vec<DataUriAt>> {
    let offset = if text.starts_with(BOM) { BOM.len() } else { 0 };
    let mut found = Vec::new();
    for token in Tokenizer::from(&text[offset..]) {
        if let Token::Attribute { value, .. } = token.map_err(|e| anyhow!("invalid XML: {e}"))?
            && let Some(uri) = unescape(value.as_str()).and_then(|v| parse_data_uri(&v))
        {
            found.push(DataUriAt {
                range: offset + value.start()..offset + value.end(),
                prefix: uri.prefix,
                mime: uri.mime,
                bytes: uri.bytes,
            });
        }
    }
    Ok(found)
}

struct DataUriAt {
    range: Range<usize>,
    prefix: String,
    mime: String,
    bytes: Vec<u8>,
}

/// (type, bytes) of every embedded raster image, in document order.
fn embedded_images(text: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let doc = parse(text)?;
    Ok(doc
        .descendants()
        .flat_map(|n| n.attributes())
        .filter_map(|a| parse_data_uri(a.value()))
        .map(|u| (u.mime, u.bytes))
        .collect())
}

// ------------------------------------------------------------------- clean

/// One start tag being collected until its `>` or `/>`.
struct StartTag<'a> {
    prefix: &'a str,
    local: &'a str,
    attrs: Vec<(&'a str, &'a str, &'a str, &'a str)>, // prefix, local, value, source text
}

struct Scope {
    namespaces: HashMap<String, String>,
    preserve: bool,
}

pub(super) fn clean(input: &str) -> Result<String> {
    let (bom, text) = match input.strip_prefix(BOM) {
        Some(rest) => (BOM, rest),
        None => ("", input),
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(bom);

    let mut entities: HashMap<&str, &str> = HashMap::new();
    let mut scopes: Vec<Scope> = vec![Scope {
        namespaces: HashMap::from([("xml".to_string(), XML_NS.to_string())]),
        preserve: false,
    }];
    let mut pending: Option<StartTag> = None;
    let mut skip_depth = 0usize; // > 0 while inside a removed element
    let mut dtd_start: Option<usize> = None;
    let mut depth = 0usize;

    for token in Tokenizer::from(text) {
        let token = token.map_err(|e| anyhow!("invalid XML: {e}"))?;
        if let Some(tag) = &mut pending {
            match token {
                Token::Attribute { prefix, local, value, span } => {
                    tag.attrs.push((prefix.as_str(), local.as_str(), value.as_str(), span.as_str()));
                    continue;
                }
                Token::ElementEnd { end: end @ (ElementEnd::Open | ElementEnd::Empty), .. } => {
                    let tag = pending.take().expect("pending tag");
                    let open = matches!(end, ElementEnd::Open);
                    if skip_depth > 0 {
                        skip_depth += usize::from(open);
                        continue;
                    }
                    let scope = new_scope(&tag, &scopes, &entities);
                    let ns = resolve(tag.prefix, &scope, &scopes).unwrap_or_default();
                    if is_editor_ns(&ns) {
                        skip_depth = usize::from(open);
                        continue;
                    }
                    out.push('<');
                    out.push_str(&qname(tag.prefix, tag.local));
                    for (prefix, local, value, source) in &tag.attrs {
                        let declares_editor_ns = (*prefix == "xmlns" || (prefix.is_empty() && *local == "xmlns"))
                            && is_editor_ns(&expand(value, &entities));
                        let in_editor_ns = !prefix.is_empty()
                            && *prefix != "xmlns"
                            && resolve(prefix, &scope, &scopes).is_some_and(|u| is_editor_ns(&u));
                        if !declares_editor_ns && !in_editor_ns {
                            out.push(' ');
                            out.push_str(source);
                        }
                    }
                    if open {
                        out.push('>');
                        depth += 1;
                        scopes.push(scope);
                    } else {
                        out.push_str("/>");
                    }
                    continue;
                }
                _ => bail!("unexpected token inside a start tag"),
            }
        }

        match token {
            Token::ElementStart { prefix, local, .. } => {
                pending = Some(StartTag { prefix: prefix.as_str(), local: local.as_str(), attrs: Vec::new() });
            }
            Token::ElementEnd { end: ElementEnd::Close(prefix, local), .. } => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                    continue;
                }
                out.push_str("</");
                out.push_str(&qname(prefix.as_str(), local.as_str()));
                out.push('>');
                depth = depth.checked_sub(1).ok_or_else(|| anyhow!("unbalanced end tag"))?;
                scopes.pop();
            }
            Token::ElementEnd { .. } | Token::Attribute { .. } => bail!("malformed start tag"),
            _ if skip_depth > 0 => {}
            Token::Text { text } => {
                let raw = text.as_str();
                let preserve = scopes.last().is_some_and(|s| s.preserve);
                if !is_xml_whitespace(raw) || (depth > 0 && preserve) {
                    out.push_str(raw);
                }
            }
            Token::Comment { text, span } => {
                if is_legal_comment(text.as_str()) {
                    out.push_str(span.as_str());
                }
            }
            Token::Cdata { span, .. } | Token::ProcessingInstruction { span, .. } | Token::Declaration { span, .. } => {
                out.push_str(span.as_str());
            }
            Token::EmptyDtd { span, .. } => out.push_str(span.as_str()),
            Token::DtdStart { span, .. } => dtd_start = Some(span.start()),
            Token::EntityDeclaration { name, definition: EntityDefinition::EntityValue(value), .. } => {
                entities.insert(name.as_str(), value.as_str());
            }
            Token::EntityDeclaration { .. } => {}
            Token::DtdEnd { span } => {
                let start = dtd_start.take().ok_or_else(|| anyhow!("DTD end without start"))?;
                out.push_str(&text[start..span.end()]); // the whole DOCTYPE, verbatim
            }
        }
    }
    ensure!(pending.is_none() && depth == 0 && skip_depth == 0, "unexpected end of document");
    if text.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn qname(prefix: &str, local: &str) -> String {
    if prefix.is_empty() { local.to_string() } else { format!("{prefix}:{local}") }
}

/// Replaces `&name;` references to internal DTD entities (Illustrator uses
/// them for namespace URIs).
fn expand(value: &str, entities: &HashMap<&str, &str>) -> String {
    let mut out = value.to_string();
    for (name, replacement) in entities {
        out = out.replace(&format!("&{name};"), replacement);
    }
    out
}

fn new_scope(tag: &StartTag, scopes: &[Scope], entities: &HashMap<&str, &str>) -> Scope {
    let parent_preserve = scopes.last().is_some_and(|s| s.preserve);
    let mut namespaces = HashMap::new();
    let mut space = None;
    for (prefix, local, value, _) in &tag.attrs {
        match (*prefix, *local) {
            ("xmlns", p) => {
                namespaces.insert(p.to_string(), expand(value, entities));
            }
            ("", "xmlns") => {
                namespaces.insert(String::new(), expand(value, entities));
            }
            ("xml", "space") => space = Some(*value),
            _ => {}
        }
    }
    let preserve = match space {
        Some("preserve") => true,
        Some("default") => false,
        _ => parent_preserve,
    } || TEXT_ELEMENTS.contains(&tag.local);
    Scope { namespaces, preserve }
}

/// Namespace URI bound to `prefix` ("" = default namespace).
fn resolve(prefix: &str, current: &Scope, scopes: &[Scope]) -> Option<String> {
    std::iter::once(current)
        .chain(scopes.iter().rev())
        .find_map(|s| s.namespaces.get(prefix))
        .cloned()
}

// ------------------------------------------------------------ verification

fn parse(text: &str) -> Result<roxmltree::Document<'_>> {
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(text.strip_prefix(BOM).unwrap_or(text), options)
        .map_err(|e| anyhow!("invalid XML: {e}"))
}

/// A canonical dump of the document after the documented removals.
pub(super) fn canonical(text: &str) -> Result<String> {
    let doc = parse(text)?;
    let mut out = String::new();
    write_children(doc.root(), false, &mut out);
    Ok(out)
}

fn write_children(node: roxmltree::Node, preserve: bool, out: &mut String) {
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut String| {
        if !text.is_empty() && (preserve || !is_xml_whitespace(text)) {
            out.push_str(&format!("T{text:?}"));
        }
        text.clear();
    };
    for child in node.children() {
        match child.node_type() {
            roxmltree::NodeType::Text => text.push_str(child.text().unwrap_or_default()),
            roxmltree::NodeType::Comment => {
                let body = child.text().unwrap_or_default();
                if is_legal_comment(body) {
                    flush(&mut text, out);
                    out.push_str(&format!("C{body:?}"));
                }
            }
            roxmltree::NodeType::PI => {
                flush(&mut text, out);
                if let Some(pi) = child.pi() {
                    out.push_str(&format!("P{:?}{:?}", pi.target, pi.value));
                }
            }
            roxmltree::NodeType::Element => {
                let name = child.tag_name();
                if name.namespace().is_some_and(is_editor_ns) {
                    continue;
                }
                flush(&mut text, out);
                let mut attrs: Vec<String> = child
                    .attributes()
                    .filter(|a| !a.namespace().is_some_and(is_editor_ns))
                    .map(|a| {
                        // Embedded images are compared separately, by their own verifier.
                        let value = match parse_data_uri(a.value()) {
                            Some(uri) => format!("embedded {}", uri.mime),
                            None => a.value().to_string(),
                        };
                        format!("{:?}{:?}={value:?}", a.namespace().unwrap_or_default(), a.name())
                    })
                    .collect();
                attrs.sort();
                out.push_str(&format!("E{:?}{:?}[{}](", name.namespace().unwrap_or_default(), name.name(), attrs.join(",")));
                let preserve_child = match child.attribute((XML_NS, "space")) {
                    Some("preserve") => true,
                    Some("default") => false,
                    _ => preserve,
                } || TEXT_ELEMENTS.contains(&name.name());
                write_children(child, preserve_child, out);
                out.push(')');
            }
            roxmltree::NodeType::Root => {}
        }
    }
    flush(&mut text, out);
}

/// Renders at a size where every detail counts (longest side 768 px).
fn render(svg: &[u8]) -> Result<tiny_skia::Pixmap> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).map_err(|e| anyhow!("resvg cannot read it: {e}"))?;
    let size = tree.size();
    let scale = 768.0 / size.width().max(size.height());
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap =
        tiny_skia::Pixmap::new(w.clamp(1, 4096), h.clamp(1, 4096)).ok_or_else(|| anyhow!("cannot render {w}x{h}"))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizers::{Outcome, process};

    const INKSCAPE: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!-- Created with Inkscape (http://www.inkscape.org/) -->
<!-- Copyright 2026 Example Corp. Licensed under MIT. -->
<svg
   width="64"
   height="64"
   viewBox="0 0 64 64"
   version="1.1"
   id="svg1"
   inkscape:version="1.3 (0e150ed6c4, 2023-07-21)"
   sodipodi:docname="icon.svg"
   xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
   xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
   xmlns="http://www.w3.org/2000/svg"
   xmlns:svg="http://www.w3.org/2000/svg">
  <sodipodi:namedview
     id="namedview1"
     pagecolor="#ffffff"
     inkscape:zoom="8"
     inkscape:cx="32" />
  <defs
     id="defs1" />
  <g
     inkscape:label="Layer 1"
     inkscape:groupmode="layer"
     id="layer1">
    <rect
       style="fill:#ff0000"
       id="rect1"
       width="40"
       height="30"
       x="12"
       y="17" />
    <circle cx="32" cy="32" r="10" fill="blue" class="dot"/>
    <text x="4" y="60"><tspan>Hello</tspan>   <tspan>world</tspan></text>
  </g>
</svg>
"##;

    const ILLUSTRATOR: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<!-- Generator: Adobe Illustrator 27.0.0, SVG Export Plug-In . SVG Version: 6.00 Build 0)  -->
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd" [
	<!ENTITY ns_extend "http://ns.adobe.com/Extensibility/1.0/">
	<!ENTITY ns_ai "http://ns.adobe.com/AdobeIllustrator/10.0/">
	<!ENTITY ns_svg "http://www.w3.org/2000/svg">
]>
<svg version="1.1" xmlns:x="&ns_extend;" xmlns:i="&ns_ai;" xmlns="&ns_svg;" x="0px" y="0px" width="100px" height="50px" viewBox="0 0 100 50" xml:space="preserve">
<switch>
	<foreignObject requiredExtensions="&ns_ai;" x="0" y="0" width="1" height="1">
		<i:pgf id="adobe_illustrator_pgf">
		<![CDATA[
			eJzs/WtzHUmSIIp9l5n+w9GH0kgvCWB2V3VXzezeGSPAi+K3FcmWNK8xuJgr/4N7zh+eezo3Prb5d+aOJbu/ttyWuGRvdr7jdF6f9c
			ZW1+KudnI0rXlCCJ7cXT8U7hLNGiLaqh8Mx/q3DCpQRFq7hRsP7JZMLpFL3BvEuajGtvqIdGNabcYOW/Y/ivfs2ffmPf4ZnZnp6
		]]>
		</i:pgf>
	</foreignObject>
	<g i:extraneous="self">
		<rect x="10" y="10" fill="#00FF00" width="80" height="30"/>
	</g>
</switch>
</svg>
"##;

    async fn optimize(svg: &str) -> String {
        let settings = Settings { min_savings_bytes: 1, min_savings_percent: 0.0, ..Settings::default() };
        match process(&Svg, Arc::new(svg.as_bytes().to_vec()), &settings).await {
            Outcome::Optimized(out) => String::from_utf8(out).unwrap(),
            other => panic!("expected optimization, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn inkscape_editor_data_is_removed_and_content_kept() {
        let out = optimize(INKSCAPE).await;
        assert!(out.len() < INKSCAPE.len() * 3 / 4, "{out}");
        assert!(!out.contains("inkscape") && !out.contains("sodipodi"), "{out}");
        assert!(!out.contains("Created with Inkscape"));
        assert!(out.contains("<!-- Copyright 2026 Example Corp. Licensed under MIT. -->"));
        assert!(out.contains(r#"<tspan>Hello</tspan>   <tspan>world</tspan>"#), "text spacing must survive");
        assert!(out.contains(r#"id="rect1""#) && out.contains(r#"class="dot""#));
        assert!(out.starts_with(r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>"#));
        assert!(out.ends_with("</svg>\n"));
    }

    #[tokio::test]
    async fn illustrator_private_data_is_removed() {
        let out = optimize(ILLUSTRATOR).await;
        assert!(!out.contains("i:pgf") && !out.contains("i:extraneous") && !out.contains("xmlns:i="), "{out}");
        assert!(!out.contains("xmlns:x="), "{out}");
        assert!(out.contains("<!DOCTYPE svg PUBLIC"), "DOCTYPE with entities must stay");
        assert!(out.contains(r#"requiredExtensions="&ns_ai;""#));
        assert!(out.contains(r##"<rect x="10" y="10" fill="#00FF00" width="80" height="30"/>"##));
    }

    #[tokio::test]
    async fn already_clean_svg_is_left_alone() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><path d="M0 0h8v8z"/></svg>"#;
        let outcome = process(&Svg, Arc::new(svg.as_bytes().to_vec()), &Settings::default()).await;
        assert!(matches!(outcome, Outcome::Optimal(_)), "{outcome:?}");
    }

    #[tokio::test]
    async fn verifier_rejects_real_changes() {
        let original = Arc::new(INKSCAPE.as_bytes().to_vec());
        for altered in [
            INKSCAPE.replace(r#"width="40""#, r#"width="41""#),
            INKSCAPE.replace(r#"class="dot""#, ""),
            INKSCAPE.replace("<tspan>Hello</tspan>   <tspan>", "<tspan>Hello</tspan> <tspan>"),
            INKSCAPE.replace("<!-- Copyright 2026 Example Corp. Licensed under MIT. -->", ""),
        ] {
            let result = Svg.verify(original.clone(), Arc::new(altered.into_bytes())).await;
            assert!(result.is_err());
        }
        assert!(Svg.verify(original.clone(), original).await.is_ok());
    }

    fn bloated_png(shift: u8) -> Vec<u8> {
        use image::{ExtendedColorType, ImageEncoder, codecs::png::{CompressionType, FilterType, PngEncoder}};
        let rgba: Vec<u8> = (0..64u32 * 64)
            .flat_map(|i| [(i % 64) as u8 * 4, ((i / 64) as u8 * 4).wrapping_add(shift), 120, 255])
            .collect();
        let mut out = Vec::new();
        PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::NoFilter)
            .write_image(&rgba, 64, 64, ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    /// Two embedded PNGs: one plain, one with the base64 wrapped by `&#10;`
    /// line breaks the way Illustrator writes it.
    fn svg_with_images(first: &[u8], second: &[u8]) -> String {
        let wrapped = b64(second).as_bytes().chunks(76).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("&#10;");
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 128 64"><image width="64" height="64" href="data:image/png;base64,{}"/><image x="64" width="64" height="64" xlink:href="data:image/png;base64,{wrapped}"/></svg>"#,
            b64(first)
        )
    }

    #[tokio::test]
    async fn embedded_images_are_optimized_and_verified() {
        let svg = svg_with_images(&bloated_png(0), &bloated_png(9));
        let out = optimize(&svg).await;
        assert!(out.len() < svg.len() / 2, "{} -> {}", svg.len(), out.len());
        assert!(!out.contains("&#10;"));
        let (before, after) = (embedded_images(&svg).unwrap(), embedded_images(&out).unwrap());
        assert_eq!(after.len(), 2);
        for ((_, a), (mime, b)) in before.iter().zip(&after) {
            assert_eq!(mime, "image/png");
            assert!(b.len() < a.len());
            crate::optimizers::verify::same_pixels(a, b, image::ImageFormat::Png).unwrap();
        }
    }

    #[tokio::test]
    async fn verifier_rejects_a_changed_embedded_image() {
        let mut img = image::load_from_memory(&bloated_png(9)).unwrap().to_rgba8();
        img.put_pixel(3, 3, image::Rgba([0, 0, 0, 255]));
        let mut altered_png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut altered_png), image::ImageFormat::Png).unwrap();

        let original = svg_with_images(&bloated_png(0), &bloated_png(9));
        let altered = svg_with_images(&bloated_png(0), &altered_png);
        let result = Svg.verify(Arc::new(original.into_bytes()), Arc::new(altered.into_bytes())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn lossy_or_disabled_embedded_images_are_left_alone() {
        let rgba: Vec<u8> = (0..32u32 * 32).flat_map(|i| [i as u8, (i / 3) as u8, 7, 255]).collect();
        let lossy = unsafe {
            let mut out = std::ptr::null_mut();
            let len = libwebp_sys::WebPEncodeRGBA(rgba.as_ptr(), 32, 32, 32 * 4, 80.0, &mut out);
            let bytes = std::slice::from_raw_parts(out, len).to_vec();
            libwebp_sys::WebPFree(out as *mut std::ffi::c_void);
            bytes
        };
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><image width="32" height="32" href="data:image/webp;base64,{}"/></svg>"#,
            b64(&lossy)
        );
        let outcome = process(&Svg, Arc::new(svg.into_bytes()), &Settings::default()).await;
        assert!(matches!(outcome, Outcome::Optimal(_)), "{outcome:?}");

        let svg = svg_with_images(&bloated_png(0), &bloated_png(9));
        let mut settings = Settings::default();
        settings.optimizers.insert("oxipng".into(), false);
        let outcome = process(&Svg, Arc::new(svg.into_bytes()), &settings).await;
        assert!(matches!(outcome, Outcome::Optimal(_)), "PNG optimizer is off: {outcome:?}");
    }

    #[test]
    fn data_uris_are_parsed_strictly() {
        assert!(parse_data_uri("data:image/png;base64,iVBORw0KGgo=").is_some());
        assert!(parse_data_uri("data:image/svg+xml;base64,PHN2Zz4=").is_none());
        assert!(parse_data_uri("data:image/png,raw-not-base64").is_none());
        assert!(parse_data_uri("url(data:image/png;base64,iVBORw0KGgo=)").is_none());
        assert_eq!(unescape("a&#10;b&amp;c&#x41;").unwrap(), "a\nb&cA");
        assert!(unescape("&custom;").is_none());
    }

    #[test]
    fn legal_comments_are_recognized() {
        assert!(is_legal_comment("! keep me"));
        assert!(is_legal_comment(" Font Awesome by @fontawesome - License - https://fontawesome.com/license "));
        assert!(is_legal_comment(" SPDX-License-Identifier: MIT "));
        assert!(!is_legal_comment(" Generator: Sketch 52 "));
    }

    #[test]
    fn invalid_xml_is_an_error() {
        assert!(clean("<svg><g></svg>").is_err());
        assert!(clean("not xml at all <").is_err());
    }
}
