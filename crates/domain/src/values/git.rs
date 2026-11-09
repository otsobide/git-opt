use std::fmt;

use crate::InvalidValue;

fn object_id(kind: &'static str, value: &str) -> Result<String, InvalidValue> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(InvalidValue::new(kind, format!("{value:?} is not a SHA-1/SHA-256 object id")))
    }
}

macro_rules! object_id_type {
    ($(#[$doc:meta])* $name:ident, $kind:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl AsRef<str>) -> Result<Self, InvalidValue> {
                object_id($kind, value.as_ref()).map(Self)
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            /// The usual 7-character abbreviation.
            pub fn short(&self) -> &str {
                &self.0[..7]
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

object_id_type!(
    /// A commit id.
    CommitSha,
    "commit id"
);
object_id_type!(
    /// The id of a file's content: equal bytes always have the same id.
    BlobId,
    "blob id"
);

impl BlobId {
    /// The all-zero id git uses for "no blob".
    pub fn is_null(&self) -> bool {
        self.0.bytes().all(|b| b == b'0')
    }
}

/// A branch name, following `git check-ref-format` rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchName(String);

impl BranchName {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        let bad = value.is_empty()
            || value.len() > 255
            || value.starts_with(['-', '/', '.'])
            || value.ends_with(['/', '.'])
            || value.ends_with(".lock")
            || value.contains("..")
            || value.contains("//")
            || value.contains("@{")
            || value.chars().any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c));
        if bad { Err(InvalidValue::new("branch name", format!("{value:?}"))) } else { Ok(Self(value)) }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A file path relative to the repository root.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoPath(String);

impl RepoPath {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        let bad = value.is_empty()
            || value.starts_with('/')
            || value.contains('\0')
            || value.split('/').any(|s| s.is_empty() || s == "." || s == "..")
            || value.split('/').next() == Some(".git");
        if bad { Err(InvalidValue::new("repository path", format!("{value:?}"))) } else { Ok(Self(value)) }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Lower-case extension, without the dot.
    pub fn extension(&self) -> Option<String> {
        let file = self.0.rsplit('/').next()?;
        let (stem, ext) = file.rsplit_once('.')?;
        (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a repository is cloned from or pushed to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CloneUrl(String);

impl CloneUrl {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        let known = ["https://", "http://", "ssh://", "file://", "git@"].iter().any(|p| value.starts_with(p));
        if known && !value.chars().any(char::is_whitespace) {
            Ok(Self(value))
        } else {
            Err(InvalidValue::new("clone URL", format!("{value:?}")))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// `https://host/` for HTTPS URLs: the scope credentials apply to.
    pub fn https_prefix(&self) -> Option<String> {
        let rest = self.0.strip_prefix("https://")?;
        let host = rest.split('/').next().filter(|h| !h.is_empty())?;
        Some(format!("https://{host}/"))
    }
}

impl fmt::Display for CloneUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_ids_are_hex_of_the_right_length() {
        let sha = CommitSha::new("8B5D34AC234DB5823DF03017AE885E309E46F3AE").unwrap();
        assert_eq!(sha.as_str(), "8b5d34ac234db5823df03017ae885e309e46f3ae");
        assert_eq!(sha.short(), "8b5d34a");
        assert!(CommitSha::new("8b5d34a").is_err());
        assert!(BlobId::new("z".repeat(40)).is_err());
        assert!(BlobId::new("0".repeat(40)).unwrap().is_null());
    }

    #[test]
    fn branch_names_follow_git_rules() {
        assert!(BranchName::new("git-opt/optimize-images").is_ok());
        for bad in ["", "-x", "a..b", "a b", "a/", "x.lock", "a~1", "a@{1}", "/a"] {
            assert!(BranchName::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repo_paths_are_relative_and_clean() {
        let path = RepoPath::new("docs/img/Logo.PNG").unwrap();
        assert_eq!(path.extension().as_deref(), Some("png"));
        assert_eq!(RepoPath::new("README").unwrap().extension(), None);
        assert_eq!(RepoPath::new(".gitignore").unwrap().extension(), None);
        for bad in ["", "/etc/passwd", "a/../b", "a//b", "./a", ".git/config"] {
            assert!(RepoPath::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn clone_urls_know_their_credential_scope() {
        let url = CloneUrl::new("https://github.com/o/r.git").unwrap();
        assert_eq!(url.https_prefix().as_deref(), Some("https://github.com/"));
        assert_eq!(CloneUrl::new("file:///tmp/r.git").unwrap().https_prefix(), None);
        assert!(CloneUrl::new("ftp://x").is_err());
    }
}
