//! A string that must not reach a log.

use std::fmt;

use serde::Deserialize;

/// Wraps a credential — the STOMP `passcode` — so a stray `Debug` or
/// `Display` on a config struct cannot print it.
///
/// `Debug` and `Display` render `<redacted>` for any non-empty value (and
/// an empty string for an unset one, so a config dump still shows whether
/// a passcode is configured). [`Self::expose`] is the one, greppable way
/// to read the value back, and is called only where the CONNECT frame is
/// assembled.
#[derive(Clone, Default, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// The wrapped value. Named so every real use of the secret is
    /// grep-visible.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether no credential is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("Secret(\"\")")
        } else {
            f.write_str("Secret(<redacted>)")
        }
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            Ok(())
        } else {
            f.write_str("<redacted>")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "hunter2-do-not-log";

    #[test]
    fn debug_does_not_leak() {
        let s = Secret::from(SECRET.to_owned());
        let shown = format!("{s:?}");
        assert!(!shown.contains(SECRET), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
    }

    #[test]
    fn display_does_not_leak() {
        let s = Secret::from(SECRET.to_owned());
        assert!(!format!("{s}").contains(SECRET));
    }

    #[test]
    fn expose_returns_the_value() {
        let s = Secret::from(SECRET.to_owned());
        assert_eq!(s.expose(), SECRET);
        assert!(!s.is_empty());
        assert!(Secret::default().is_empty());
    }

    #[test]
    fn deserializes_transparently_from_a_string() {
        let s: Secret = serde_json::from_str("\"pw\"").unwrap();
        assert_eq!(s.expose(), "pw");
    }
}
