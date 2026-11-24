//! STOMP destination ↔ engine [`RouteKey`] mapping.
//!
//! ActiveMQ Classic maps `/topic/Name` to JMS topic `Name`. Engine
//! `topic` is that name. `type_hint` rides in [`HDR_TYPE_HINT`] and/or
//! is sniffed from the body. Inbound MESSAGE frames are stamped so
//! outbound SEND can refuse the echo and so `stomp.*` headers are not
//! copied back onto the wire.

use oa_gateway_agra::xml_root_local_name;
use oa_gateway_core::{ContentType, RouteKey};

pub use oa_gateway_core::{HDR_ID, HDR_ORIGIN, HDR_TOPIC, HDR_TYPE_HINT};

/// STOMP `destination` as received. Stamped inbound; stripped on
/// outbound because `stomp.*` headers are not copied.
pub const HDR_STOMP_DEST: &str = "stomp.destination";

/// `/topic/` prefix used by ActiveMQ STOMP for JMS topics.
///
/// [`Self::new`] forces a trailing slash so `to_stomp` / `from_stomp`
/// stay inverses. A topic with a leading slash is stripped on the way
/// out so `demo` and `/demo` share one destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationMap {
    prefix: String,
}

impl DestinationMap {
    /// Builds a map that prepends `prefix` to engine topics.
    ///
    /// A missing trailing `/` is added.
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        let mut prefix = prefix.into();
        if !prefix.ends_with('/') {
            prefix.push('/');
        }
        Self { prefix }
    }

    /// The normalized prefix, always ending in `/`.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// STOMP destination for an engine topic (`{prefix}{topic}`).
    #[must_use]
    pub fn to_stomp(&self, topic: &str) -> String {
        format!("{}{}", self.prefix, topic.trim_start_matches('/'))
    }

    /// Engine topic if `dest` starts with this prefix and the suffix is
    /// not empty. `/queue/…` and the bare prefix return [`None`].
    #[must_use]
    pub fn from_stomp(&self, dest: &str) -> Option<String> {
        dest.strip_prefix(&self.prefix)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }
}

impl Default for DestinationMap {
    /// ActiveMQ topic prefix: `/topic/`.
    fn default() -> Self {
        Self::new("/topic/")
    }
}

/// Engine route for an inbound STOMP MESSAGE.
///
/// A non-empty type hint becomes [`RouteKey::typed`]; missing or empty
/// is [`RouteKey::topic`] (wildcard).
#[must_use]
pub fn inbound_route(topic: &str, type_hint: Option<String>) -> RouteKey {
    match type_hint {
        Some(hint) if !hint.is_empty() => RouteKey::typed(topic, hint),
        _ => RouteKey::topic(topic),
    }
}

/// Best-effort type hint from an XML body: the root element's local name
/// (prefix stripped). Non-UTF-8 or non-XML returns [`None`].
#[must_use]
pub fn sniff_type_hint(payload: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(payload).ok()?;
    xml_root_local_name(text)
}

/// MIME type from a STOMP `content-type` header, or `application/xml`
/// when the header is absent or empty. Does not fail — a payload that
/// turns out not to be well-formed XML is a parse/validate failure
/// downstream, not a content-type-detection concern.
#[must_use]
pub fn sniff_content_type(header: Option<&str>) -> ContentType {
    if let Some(ct) = header {
        let mime = ct.split(';').next().unwrap_or(ct).trim();
        if !mime.is_empty() {
            return ContentType::new(mime);
        }
    }
    ContentType::xml()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_round_trip() {
        let map = DestinationMap::new("/topic/");
        assert_eq!(map.to_stomp("PositionReport"), "/topic/PositionReport");
        assert_eq!(
            map.from_stomp("/topic/PositionReport").as_deref(),
            Some("PositionReport")
        );
        assert_eq!(map.from_stomp("/queue/x"), None);
    }

    #[test]
    fn sniff_xml_with_prolog() {
        let xml = b"<?xml version=\"1.0\"?>\n<uci:PositionReport xmlns:uci=\"x\"/>";
        assert_eq!(sniff_type_hint(xml).as_deref(), Some("PositionReport"));
    }
}
