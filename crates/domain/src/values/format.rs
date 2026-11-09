use std::fmt;

use super::RepoPath;

/// The kinds of file git-opt knows how to optimize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
    Ico,
    Icns,
    Svg,
    Svgz,
}

impl FileFormat {
    pub const ALL: [FileFormat; 8] = [
        Self::Png,
        Self::Jpeg,
        Self::Gif,
        Self::WebP,
        Self::Ico,
        Self::Icns,
        Self::Svg,
        Self::Svgz,
    ];

    /// Lower-case extensions, without the dot.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg", "jpe", "jfif"],
            Self::Gif => &["gif"],
            Self::WebP => &["webp"],
            Self::Ico => &["ico", "cur"],
            Self::Icns => &["icns"],
            Self::Svg => &["svg"],
            Self::Svgz => &["svgz"],
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        let ext = ext.to_ascii_lowercase();
        Self::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
    }

    pub fn from_path(path: &RepoPath) -> Option<Self> {
        Self::from_extension(&path.extension()?)
    }

    /// Raster formats that may be embedded in documents as `data:` URIs.
    pub fn from_media_type(media_type: &str) -> Option<Self> {
        match media_type.to_ascii_lowercase().as_str() {
            "image/png" => Some(Self::Png),
            "image/jpeg" | "image/jpg" => Some(Self::Jpeg),
            "image/gif" => Some(Self::Gif),
            "image/webp" => Some(Self::WebP),
            _ => None,
        }
    }
}

impl fmt::Display for FileFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Gif => "GIF",
            Self::WebP => "WebP",
            Self::Ico => "ICO",
            Self::Icns => "ICNS",
            Self::Svg => "SVG",
            Self::Svgz => "SVGZ",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_come_from_extensions() {
        let path = |p: &str| RepoPath::new(p).unwrap();
        assert_eq!(FileFormat::from_path(&path("a/Logo.PNG")), Some(FileFormat::Png));
        assert_eq!(FileFormat::from_path(&path("photo.jpeg")), Some(FileFormat::Jpeg));
        assert_eq!(FileFormat::from_path(&path("cursor.cur")), Some(FileFormat::Ico));
        assert_eq!(FileFormat::from_path(&path("README")), None);
        assert_eq!(FileFormat::from_path(&path("notes.txt")), None);
        assert_eq!(FileFormat::from_media_type("image/JPG"), Some(FileFormat::Jpeg));
        assert_eq!(FileFormat::from_media_type("image/svg+xml"), None);
    }

    #[test]
    fn every_extension_maps_back_to_its_format() {
        for format in FileFormat::ALL {
            for ext in format.extensions() {
                assert_eq!(FileFormat::from_extension(ext), Some(format));
            }
        }
    }
}
