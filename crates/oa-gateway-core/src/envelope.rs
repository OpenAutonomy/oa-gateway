use std::collections::BTreeMap;

use bytes::Bytes;

use crate::{MessageId, RouteKey};

/// MIME-ish label for opaque payload bytes. The engine does not interpret it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentType(String);

impl ContentType {
    pub const OCTET_STREAM: &'static str = "application/octet-stream";
    pub const JSON: &'static str = "application/json";
    pub const XML: &'static str = "application/xml";

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub id: MessageId,
    pub route: RouteKey,
    pub headers: BTreeMap<String, String>,
    pub content_type: ContentType,
    pub payload: Bytes,
}

impl Envelope {
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

    #[must_use]
    pub fn with_content_type(mut self, content_type: ContentType) -> Self {
        self.content_type = content_type;
        self
    }

    #[must_use]
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
}
