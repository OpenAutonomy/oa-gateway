use std::collections::BTreeMap;

use bytes::Bytes;

use crate::{AdapterId, MessageId, RouteKey, HDR_ORIGIN};

/// Label for opaque payload bytes. The engine does not interpret it.
///
/// The string is not validated as a MIME type. Adapters agree on the
/// constants below; a custom value is just another label a peer may or
/// may not understand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentType(String);

impl ContentType {
    pub const OCTET_STREAM: &'static str = "application/octet-stream";
    pub const JSON: &'static str = "application/json";
    pub const XML: &'static str = "application/xml";

    /// Wraps any string. No syntax check.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn octet_stream() -> Self {
        Self::new(Self::OCTET_STREAM)
    }

    #[must_use]
    pub fn json() -> Self {
        Self::new(Self::JSON)
    }

    #[must_use]
    pub fn xml() -> Self {
        Self::new(Self::XML)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ContentType {
    fn default() -> Self {
        Self::octet_stream()
    }
}

/// Protocol-agnostic unit of data that crosses the engine.
///
/// The engine reads only [`Self::route`]. Headers, content type, and
/// payload are opaque and are cloned to every matching subscriber.
/// Header names are namespaced by owner (`oag.`, `stomp.`, `agra.`);
/// that convention is not enforced here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Gateway-assigned id, not a protocol message id.
    pub id: MessageId,
    /// The only field [`crate::Engine::publish`] matches on.
    pub route: RouteKey,
    /// String pairs adapters stamp and read. The engine never looks here.
    pub headers: BTreeMap<String, String>,
    /// How the payload is labelled, not how the engine parses it.
    pub content_type: ContentType,
    /// Uninterpreted bytes. JSON, XML, or anything else.
    pub payload: Bytes,
}

impl Envelope {
    /// Builds an envelope with a fresh [`MessageId`], no headers, and
    /// [`ContentType::octet_stream`].
    ///
    /// Call [`Self::with_content_type`] when the bytes are known to be
    /// JSON or XML. The default is octet-stream because this constructor
    /// does not inspect `payload`.
    #[must_use]
    pub fn new(route: RouteKey, payload: impl Into<Bytes>) -> Self {
        Self {
            id: MessageId::new(),
            route,
            headers: BTreeMap::new(),
            content_type: ContentType::octet_stream(),
            payload: payload.into(),
        }
    }

    /// Sets the content-type label. Does not parse or convert `payload`.
    #[must_use]
    pub fn with_content_type(mut self, content_type: ContentType) -> Self {
        self.content_type = content_type;
        self
    }

    /// Inserts or replaces one header. Last write for a key wins.
    #[must_use]
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Stamps [`HDR_ORIGIN`] so a bridging adapter can refuse its own
    /// echo. Last write wins.
    #[must_use]
    pub fn with_origin(self, id: &AdapterId) -> Self {
        self.with_header(HDR_ORIGIN, id.as_str())
    }

    /// Value of [`HDR_ORIGIN`], if stamped.
    #[must_use]
    pub fn origin(&self) -> Option<&str> {
        self.headers.get(HDR_ORIGIN).map(String::as_str)
    }

    /// Whether this envelope was published by `id`'s inbound path.
    ///
    /// Bridging adapters skip outbound-to-the-same-bus when this is
    /// true. The engine does not skip: one adapter id may cover many
    /// connections (OWP).
    #[must_use]
    pub fn is_echo_of(&self, id: &AdapterId) -> bool {
        self.origin() == Some(id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RouteKey;

    #[test]
    fn origin_helpers_round_trip() {
        let id = AdapterId::new("stomp");
        let env = Envelope::new(RouteKey::topic("demo"), b"x".as_slice()).with_origin(&id);
        assert_eq!(env.origin(), Some("stomp"));
        assert!(env.is_echo_of(&id));
        assert!(!env.is_echo_of(&AdapterId::new("owp")));
    }

    #[test]
    fn unstamped_is_not_an_echo() {
        let env = Envelope::new(RouteKey::topic("demo"), b"x".as_slice());
        assert_eq!(env.origin(), None);
        assert!(!env.is_echo_of(&AdapterId::new("stomp")));
    }
}
