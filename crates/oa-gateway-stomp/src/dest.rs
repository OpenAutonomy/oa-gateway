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

/// Best-effort type hint from a JSON or XML body.
///
/// JSON must be a single-key object; the key is the hint. XML uses the
/// root local name (prefix stripped). Multi-key JSON, non-UTF-8, and
/// other shapes return [`None`].
#[must_use]
pub fn sniff_type_hint(payload: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(payload).ok()?.trim_start();
    if text.starts_with('{') {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let obj = value.as_object()?;
        if obj.len() == 1 {
            return obj.keys().next().cloned();
        }
        return None;
    }
    if text.starts_with('<') {
        return xml_root_local_name(text);
    }
    None
}

/// MIME type from a STOMP `content-type` header, or a sniff of the
/// body.
///
/// The header wins when its type (before `;`) is non-empty. Otherwise
/// a leading `{` is JSON, a leading `<` is XML, and everything else is
/// `application/octet-stream`. Does not fail.
#[must_use]
pub fn sniff_content_type(payload: &[u8], header: Option<&str>) -> ContentType {
    if let Some(ct) = header {
        let mime = ct.split(';').next().unwrap_or(ct).trim();
        if !mime.is_empty() {
            return ContentType::new(mime);
        }
    }
    match std::str::from_utf8(payload).map(str::trim_start) {
        Ok(t) if t.starts_with('{') => ContentType::json(),
        Ok(t) if t.starts_with('<') => ContentType::xml(),
        _ => ContentType::octet_stream(),
    }
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
    fn sniff_json_and_xml_with_prolog() {
        assert_eq!(
            sniff_type_hint(br#"{"Ping":{"n":1}}"#).as_deref(),
            Some("Ping")
        );
        let xml = b"<?xml version=\"1.0\"?>\n<uci:PositionReport xmlns:uci=\"x\"/>";
        assert_eq!(sniff_type_hint(xml).as_deref(), Some("PositionReport"));
    }
}
