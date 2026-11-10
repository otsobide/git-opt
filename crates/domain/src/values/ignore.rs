use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use super::RepoPath;
use crate::InvalidValue;

/// Newline-separated ignore patterns, in a simplified `.gitignore` syntax:
/// a pattern without `/` matches at any depth, `dir/` matches a directory,
/// `*` never crosses `/`, `#` starts a comment. Valid by construction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IgnoreRules(String);

impl IgnoreRules {
    pub fn new(text: impl Into<String>) -> Result<Self, InvalidValue> {
        let rules = Self(text.into());
        rules.globs()?;
        Ok(rules)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn globs(&self) -> Result<Vec<globset::Glob>, InvalidValue> {
        let mut globs = Vec::new();
        for line in self.0.lines() {
            let pattern = line.trim();
            if pattern.is_empty() || pattern.starts_with('#') {
                continue;
            }
            let pattern = pattern.trim_start_matches('/');
            let pattern = match pattern.strip_suffix('/') {
                Some(dir) => format!("{dir}/**"),
                None => pattern.to_string(),
            };
            let pattern = if pattern.contains('/') { pattern } else { format!("**/{pattern}") };
            let glob = GlobBuilder::new(&pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| InvalidValue::new("ignore pattern", format!("{line:?}: {e}")))?;
            globs.push(glob);
        }
        Ok(globs)
    }
}

/// Several rule sets compiled together (global + per repository).
#[derive(Debug, Clone)]
pub struct IgnoreMatcher(GlobSet);

impl IgnoreMatcher {
    pub fn compile(rules: &[&IgnoreRules]) -> Result<Self, InvalidValue> {
        let mut builder = GlobSetBuilder::new();
        for r in rules {
            for glob in r.globs()? {
                builder.add(glob);
            }
        }
        builder.build().map(Self).map_err(|e| InvalidValue::new("ignore patterns", e.to_string()))
    }
    pub fn is_ignored(&self, path: &RepoPath) -> bool {
        self.0.is_match(path.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_behave_like_gitignore() {
        let global = IgnoreRules::new("*.min.png\ndocs/\n# comment\n/vendor/**").unwrap();
        let repo = IgnoreRules::new("assets/raw/*.jpg").unwrap();
        let m = IgnoreMatcher::compile(&[&global, &repo]).unwrap();
        let ignored = |p: &str| m.is_ignored(&RepoPath::new(p).unwrap());
        assert!(ignored("logo.min.png"));
        assert!(ignored("a/b/logo.min.png"));
        assert!(ignored("docs/img/x.png"));
        assert!(ignored("vendor/lib/x.png"));
        assert!(ignored("assets/raw/p.jpg"));
        assert!(!ignored("assets/raw/sub/p.jpg"), "* does not cross /");
        assert!(!ignored("assets/p.jpg"));
        assert!(!ignored("logo.png"));
    }

    #[test]
    fn invalid_patterns_are_refused() {
        assert!(IgnoreRules::new("a[").is_err());
    }
}
