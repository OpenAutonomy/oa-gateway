//! STOMP destination ↔ engine [`RouteKey`] mapping.
//!
//! ActiveMQ Classic maps `/topic/Name` to JMS topic `Name`. Engine `topic` is that
//! name. `type_hint` rides in `oag.type_hint` and/or is sniffed from the body.

use oa_gateway_core::{ContentType, RouteKey};

pub const HDR_ORIGIN: &str = "oag.origin_adapter";
pub const HDR_TYPE_HINT: &str = "oag.type_hint";
pub const HDR_TOPIC: &str = "oag.topic";
pub const HDR_ID: &str = "oag.id";
pub const HDR_STOMP_DEST: &str = "stomp.destination";

/// `/topic/` prefix used by ActiveMQ STOMP for JMS topics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationMap {
    prefix: String,
}

impl DestinationMap {
    #[must_use]
    pub fn new(prefix: impl Into<String>) -> Self {
        let mut prefix = prefix.into();
        if !prefix.ends_with('/') {
            prefix.push('/');
        }
        Self { prefix }
    }

    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    #[must_use]
    pub fn to_stomp(&self, topic: &str) -> String {
        format!("{}{}", self.prefix, topic.trim_start_matches('/'))
    }

    #[must_use]
    pub fn from_stomp(&self, dest: &str) -> Option<String> {
        dest.strip_prefix(&self.prefix)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }
}

impl Default for DestinationMap {
    fn default() -> Self {
        Self::new("/topic/")
    }
}

/// Build the engine route for an inbound STOMP MESSAGE.
#[must_use]
pub fn inbound_route(topic: &str, type_hint: Option<String>) -> RouteKey {
    match type_hint {
        Some(hint) if !hint.is_empty() => RouteKey::typed(topic, hint),
        _ => RouteKey::topic(topic),
    }
}

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

fn xml_root_local_name(xml: &str) -> Option<String> {
    let mut i = 0;
    while let Some(rel) = xml[i..].find('<') {
        let start = i + rel;
        let rest = &xml[start + 1..];
        if rest.starts_with('?') {
            let close = rest.find("?>")?;
            i = start + 1 + close + 2;
            continue;
        }
        if let Some(inner) = rest.strip_prefix("!--") {
            let close = inner.find("-->")?;
            i = start + 4 + close + 3;
            continue;
        }
        if rest.starts_with('!') {
            let close = rest.find('>')?;
            i = start + 1 + close + 1;
            continue;
        }
        if rest.starts_with('/') {
            return None;
        }
        let name_end = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
        let qname = &rest[..name_end];
        return Some(qname.rsplit(':').next().unwrap_or(qname).to_owned());
    }
    None
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
