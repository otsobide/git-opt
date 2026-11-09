//! Format strategies: one lossless optimizer per family of file formats, each
//! implementing the domain's [`FormatStrategy`] port and each behind a Cargo
//! feature so its library can be left out or replaced.
//!
//! | Feature | Strategy | Library |
//! |---|---|---|
//! | `png` | [`png::Oxipng`] (PNG, APNG) | oxipng |
//! | `jpeg` | [`jpeg::Mozjpeg`] | mozjpeg (FFI) |
//! | `gif` | [`gif::Gifsicle`] | `gifsicle` (external tool) |
//! | `webp` | [`webp::Webp`] | libwebp (FFI) |
//! | `ico` | [`ico::Ico`] (ICO, CUR) | oxipng |
//! | `icns` | [`icns::Icns`] | oxipng |
//! | `svg` | [`svg::Svg`] | xmlparser, roxmltree, resvg |
//! | `svgz` | [`svgz::Svgz`] | flate2, zopfli |

use std::sync::Arc;

use git_opt_domain::ports::FormatStrategy;

#[cfg(feature = "gif")]
mod external;
mod verify;

#[cfg(feature = "gif")]
pub mod gif;
#[cfg(feature = "icns")]
pub mod icns;
#[cfg(feature = "ico")]
pub mod ico;
#[cfg(feature = "jpeg")]
pub mod jpeg;
#[cfg(feature = "png")]
pub mod png;
#[cfg(feature = "svg")]
pub mod svg;
#[cfg(feature = "svgz")]
pub mod svgz;
#[cfg(feature = "webp")]
pub mod webp;

#[cfg(test)]
mod tests;

/// Strategies for raster formats that can be embedded in documents (SVG), in
/// a stable order (it is part of the SVG cache profile).
#[allow(clippy::vec_init_then_push, unused_mut)] // pushes depend on features
pub fn raster_strategies() -> Vec<Arc<dyn FormatStrategy>> {
    let mut all: Vec<Arc<dyn FormatStrategy>> = Vec::new();
    #[cfg(feature = "png")]
    all.push(Arc::new(png::Oxipng));
    #[cfg(feature = "jpeg")]
    all.push(Arc::new(jpeg::Mozjpeg));
    #[cfg(feature = "webp")]
    all.push(Arc::new(webp::Webp));
    #[cfg(feature = "gif")]
    all.push(Arc::new(gif::Gifsicle));
    all
}

/// Every compiled-in strategy, wired together, in display order.
#[allow(clippy::vec_init_then_push, unused_mut)] // pushes depend on features
pub fn default_strategies() -> Vec<Arc<dyn FormatStrategy>> {
    let mut all: Vec<Arc<dyn FormatStrategy>> = Vec::new();
    #[cfg(feature = "png")]
    all.push(Arc::new(png::Oxipng));
    #[cfg(feature = "jpeg")]
    all.push(Arc::new(jpeg::Mozjpeg));
    #[cfg(feature = "gif")]
    all.push(Arc::new(gif::Gifsicle));
    #[cfg(feature = "webp")]
    all.push(Arc::new(webp::Webp));
    #[cfg(feature = "ico")]
    all.push(Arc::new(ico::Ico));
    #[cfg(feature = "icns")]
    all.push(Arc::new(icns::Icns));
    #[cfg(feature = "svg")]
    {
        let svg = Arc::new(svg::Svg::new(raster_strategies()));
        all.push(svg.clone());
        #[cfg(feature = "svgz")]
        all.push(Arc::new(svgz::Svgz::new(svg)));
    }
    all
}
