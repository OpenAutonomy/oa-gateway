/// Addressing key used by the engine.
///
/// `topic` is the only required coordinate. `type_hint` is an optional
/// protocol-defined discriminator (OWP message name, DIS PDU type, …).
/// A subscription with `type_hint = None` matches every type on that topic.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RouteKey {
    pub topic: String,
    pub type_hint: Option<String>,
}

impl RouteKey {
    #[must_use]
    pub fn topic(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            type_hint: None,
        }
    }

    #[must_use]
    pub fn typed(topic: impl Into<String>, type_hint: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            type_hint: Some(type_hint.into()),
        }
    }

    #[must_use]
    pub fn is_wildcard(&self) -> bool {
        self.type_hint.is_none()
    }
}

impl std::fmt::Display for RouteKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.type_hint {
            Some(hint) => write!(f, "{}/{}", self.topic, hint),
            None => write!(f, "{}/", self.topic.trim_end_matches('/')),
        }
    }
}
