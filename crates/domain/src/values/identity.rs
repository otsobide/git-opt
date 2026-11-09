use std::fmt;

use crate::InvalidValue;

/// Author of a commit: the developer, never a bot account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    name: String,
    email: String,
}

impl Identity {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Result<Self, InvalidValue> {
        let (name, email) = (name.into().trim().to_string(), email.into().trim().to_string());
        if name.is_empty() || name.contains(['\n', '\r', '<', '>']) {
            return Err(InvalidValue::new("author name", format!("{name:?}")));
        }
        let valid_email = email.split_once('@').is_some_and(|(user, host)| !user.is_empty() && !host.is_empty())
            && !email.chars().any(|c| c.is_whitespace() || c == '<' || c == '>');
        if !valid_email {
            return Err(InvalidValue::new("author email", format!("{email:?}")));
        }
        Ok(Self { name, email })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn email(&self) -> &str {
        &self.email
    }
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} <{}>", self.name, self.email)
    }
}

/// A forge credential. Never printed: `Debug` and `Display` are redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken(String);

impl AccessToken {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
        let value = value.into().trim().to_string();
        if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            Err(InvalidValue::new("access token", "empty or contains whitespace"))
        } else {
            Ok(Self(value))
        }
    }
    /// The secret itself. Only adapters that talk to the forge call this.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessToken(***)")
    }
}

impl fmt::Display for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_are_validated() {
        let id = Identity::new(" Jane Developer ", "jane@example.com").unwrap();
        assert_eq!(id.to_string(), "Jane Developer <jane@example.com>");
        assert!(Identity::new("", "jane@example.com").is_err());
        assert!(Identity::new("Jane", "not-an-email").is_err());
        assert!(Identity::new("Jane\nEvil", "jane@example.com").is_err());
    }

    #[test]
    fn tokens_never_print() {
        let token = AccessToken::new("ghp_secret").unwrap();
        assert_eq!(format!("{token:?} {token}"), "AccessToken(***) ***");
        assert_eq!(token.expose(), "ghp_secret");
        assert!(AccessToken::new("  ").is_err());
    }
}
