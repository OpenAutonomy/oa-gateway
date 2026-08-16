//! Newtypes so an adapter id, a subscription id, and a message id cannot
//! be mixed at the call site.
//!
//! None of these are validated. An empty [`AdapterId`] is accepted and
//! will show up as a blank name in logs.

use std::fmt;

/// Id of one running adapter instance.
///
/// Comes from the host config (`id = "owp"`). The host logs it, the
/// engine keys subscriptions on it, and bridging adapters stamp
/// `oag.origin_adapter` with it. Two adapters must not share a value if
/// echo suppression is going to work.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AdapterId(String);

impl AdapterId {
    /// Wraps any string. No uniqueness or format check.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for AdapterId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for AdapterId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Id of one subscription inside one adapter.
///
/// Uniqueness is per adapter: `"s1"` on `owp` and `"s1"` on `stomp` are
/// different keys. Reusing the same pair on one adapter is
/// [`crate::EngineError::DuplicateSub`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubId(String);

impl SubId {
    /// Wraps any string. The adapter chooses the scheme (a counter, a
    /// connection-local name, a UUID).
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SubId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SubId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SubId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Id assigned to an [`crate::Envelope`] when it is constructed.
///
/// This is a gateway UUID, not a protocol message-id. STOMP and OWP keep
/// their own identifiers in headers. [`Self::default`] mints a new v4,
/// not a nil UUID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(uuid::Uuid);

impl MessageId {
    /// A fresh random v4 UUID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    #[must_use]
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
