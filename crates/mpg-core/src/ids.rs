use std::fmt;

/// Identify a running adapter instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AdapterId(String);

impl AdapterId {
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

/// Identify a subscription within an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubId(String);

impl SubId {
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

/// Unique id assigned to every envelope that enters the engine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(uuid::Uuid);

impl MessageId {
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
