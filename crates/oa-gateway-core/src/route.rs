//! Addressing key used by [`crate::Engine`].
//!
//! The engine compares these for equality only. It does not treat `topic`
//! as a path and does not interpret `type_hint`.

/// Topic plus an optional type hint.
///
/// `topic` is the only required coordinate. `type_hint` is whatever
/// discriminator the publishing adapter has: an OWP message name, a UCI
/// type, a PDU type. A subscription with `type_hint: None` matches every
/// type on that topic. A publish with no hint reaches wildcards only.
///
/// Neither field is validated. Matching is exact string equality, not a
/// hierarchy, even though [`Display`](Self#impl-Display-for-RouteKey)
/// looks like a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RouteKey {
    /// Engine topic. Often a UCI message name when bridging ActiveMQ.
    pub topic: String,
    /// Protocol discriminator. `None` is a wildcard subscription, or an
    /// untyped publish.
    pub type_hint: Option<String>,
}

impl RouteKey {
    /// A wildcard: every type on `topic`.
    #[must_use]
    pub fn topic(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            type_hint: None,
        }
    }

    /// One type on `topic`. A publish with this hint also reaches
    /// [`Self::topic`] subscribers on the same topic.
    #[must_use]
    pub fn typed(topic: impl Into<String>, type_hint: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            type_hint: Some(type_hint.into()),
        }
    }

    /// Whether this key has no type hint.
    ///
    /// True for both a wildcard subscription and an untyped publish.
    /// Those two roles match different sets; this predicate does not
    /// distinguish them.
    #[must_use]
    pub fn is_wildcard(&self) -> bool {
        self.type_hint.is_none()
    }
}

impl std::fmt::Display for RouteKey {
    /// Formats as `topic/hint`, or `topic/` when there is no hint.
    ///
    /// Trailing slashes on `topic` are stripped only in the wildcard
    /// form, so `demo/` and `demo` display the same when untyped.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.type_hint {
            Some(hint) => write!(f, "{}/{}", self.topic, hint),
            None => write!(f, "{}/", self.topic.trim_end_matches('/')),
        }
    }
}
