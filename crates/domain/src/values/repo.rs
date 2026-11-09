use std::{fmt, str::FromStr};

use crate::InvalidValue;

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A user or organization login.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnerName(String);

impl OwnerName {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        let value = value.trim().trim_start_matches('@');
        if valid_segment(value) {
            Ok(Self(value.to_string()))
        } else {
            Err(InvalidValue::new("owner name", format!("{value:?}")))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Logins are case-insensitive on GitHub.
    pub fn same_as(&self, other: &str) -> bool {
        self.0.eq_ignore_ascii_case(other)
    }
}

impl fmt::Display for OwnerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A repository name, without its owner.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoName(String);

impl RepoName {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into();
        if valid_segment(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidValue::new("repository name", format!("{value:?}")))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `owner/name`: how a repository is addressed on the forge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoSlug {
    owner: OwnerName,
    name: RepoName,
}

impl RepoSlug {
    pub fn new(owner: OwnerName, name: RepoName) -> Self {
        Self { owner, name }
    }

    /// Accepts `owner/name`, `https://host/owner/name(.git)` or
    /// `git@host:owner/name.git`.
    pub fn parse(input: &str) -> Result<Self, InvalidValue> {
        let s = input.trim().trim_end_matches('/');
        let s = s.strip_suffix(".git").unwrap_or(s);
        let path = if let Some((_, rest)) = s.split_once("://") {
            rest.split_once('/').map(|(_, p)| p).unwrap_or("")
        } else if let Some((_, p)) = s.split_once(':').filter(|(host, _)| host.contains('@')) {
            p
        } else {
            s
        };
        let mut parts = path.split('/').filter(|p| !p.is_empty());
        match (parts.next(), parts.next(), parts.next()) {
            (Some(owner), Some(name), None) => Ok(Self::new(OwnerName::new(owner)?, RepoName::new(name)?)),
            _ => Err(InvalidValue::new("repository", format!("expected owner/name or a URL, got {input:?}"))),
        }
    }

    pub fn owner(&self) -> &OwnerName {
        &self.owner
    }
    pub fn name(&self) -> &RepoName {
        &self.name
    }
}

impl fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl FromStr for RepoSlug {
    type Err = InvalidValue;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_reference_form() {
        for input in [
            "octo/hello-world",
            "https://github.com/octo/hello-world",
            "https://github.com/octo/hello-world.git",
            "https://github.com/octo/hello-world/",
            "git@github.com:octo/hello-world.git",
            " octo/hello-world ",
        ] {
            let slug = RepoSlug::parse(input).unwrap();
            assert_eq!(slug.to_string(), "octo/hello-world", "{input}");
        }
    }

    #[test]
    fn refuses_invalid_references() {
        for input in ["octo", "octo/hello world", "a/b/c", "../x", "octo/..", ""] {
            assert!(RepoSlug::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn owner_names_drop_a_leading_at() {
        assert_eq!(OwnerName::new("@octo").unwrap().as_str(), "octo");
        assert!(OwnerName::new("Octo").unwrap().same_as("octo"));
    }
}
