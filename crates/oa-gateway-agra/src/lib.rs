//! A-GRA offboard payload envelopes.
//!
//! External MA interfaces (MA-C2, MA-MA) wrap inner UCI messages as hexBinary
//! inside [`MA_RxDataPayload`](WrapperKind::Rx) (offboard → MA) or
//! [`MA_TxDataPayloadCommand`](WrapperKind::Tx) (MA → offboard). Platform
//! interfaces (MA-VI, MA-MS) use native MTs and do not go through this codec.
//!
//! The engine stays wrapper-agnostic: this crate turns envelopes inside out.

use std::collections::BTreeMap;

use bytes::Bytes;
use oa_gateway_core::{ContentType, Envelope, RouteKey};
use serde_json::{json, Value};

pub const RX_ELEMENT: &str = "MA_RxDataPayload";
pub const TX_ELEMENT: &str = "MA_TxDataPayloadCommand";
pub const TX_STATUS_ELEMENT: &str = "MA_TxDataPayloadCommandStatus";

const HDR_WRAPPER: &str = "agra.wrapper";
const HDR_MESSAGE_TYPE: &str = "agra.message_type";
const HDR_ORIGINATOR: &str = "agra.originator_uuid";
const HDR_RX_ID: &str = "agra.rx_payload_id";
const HDR_COMMAND_ID: &str = "agra.command_id";
const HDR_DEST_ROUTING: &str = "agra.destination_routing";
const HDR_INNER_CT: &str = "agra.inner_content_type";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapperKind {
    /// Offboard → MA ([`RX_ELEMENT`]). Data-1.
    Rx,
    /// MA → offboard ([`TX_ELEMENT`]). Command-2.
    Tx,
}

impl WrapperKind {
    #[must_use]
    pub fn element_name(self) -> &'static str {
        match self {
            Self::Rx => RX_ELEMENT,
            Self::Tx => TX_ELEMENT,
        }
    }

    #[must_use]
    pub fn from_element(name: &str) -> Option<Self> {
        match name {
            RX_ELEMENT => Some(Self::Rx),
            TX_ELEMENT => Some(Self::Tx),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgraError {
    #[error("not an MA Rx/Tx data-payload wrapper")]
    NotAWrapper,
    #[error("invalid OMS JSON: {0}")]
    Json(String),
    #[error("wrapper missing MessageData.{0}")]
    MissingField(&'static str),
    #[error("EncodedPayload is not hexBinary: {0}")]
    BadHex(String),
    #[error("EncodedPayload decodes to more than {MAX_DECODED_PAYLOAD} bytes")]
    PayloadTooLarge,
}

/// Metadata lifted off the wrapper (not the inner UCI message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperMeta {
    pub kind: WrapperKind,
    pub message_type_enum: String,
    pub originator_uuid: Option<String>,
    pub rx_payload_id: Option<String>,
    pub command_id: Option<String>,
    pub destination_routing: Option<String>,
}

/// Result of peeling an Rx/Tx wrapper.
#[derive(Debug, Clone)]
pub struct Unwrapped {
    pub wrapper: Envelope,
    pub inner: Envelope,
    pub meta: WrapperMeta,
}

/// Detect whether OMS JSON (or XML document element) is an Rx/Tx wrapper.
#[must_use]
pub fn wrapper_kind(payload: &[u8]) -> Option<WrapperKind> {
    if let Ok(text) = std::str::from_utf8(payload) {
        let trimmed = text.trim_start();
        if trimmed.starts_with('{') {
            if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                if let Some(obj) = value.as_object() {
                    if obj.len() == 1 {
                        if let Some(name) = obj.keys().next() {
                            return WrapperKind::from_element(name);
                        }
                    }
                }
            }
        }
        if trimmed.starts_with('<') {
            if let Some(name) = xml_root_local_name(trimmed) {
                return WrapperKind::from_element(&name);
            }
        }
    }
    None
}

/// Builds wrapper and inner envelopes from structured Rx/Tx fields.
///
/// The DDS adapter's wire type is these fields plus raw
/// `EncodedPayload` bytes, not XML or JSON text. The wrapper envelope
/// is a minimal JSON document so [`wrapper_kind`] and [`unwrap`] still
/// recognize it.
///
/// # Errors
///
/// Returns [`AgraError::PayloadTooLarge`] if `encoded` is over
/// [`MAX_DECODED_PAYLOAD`], or [`AgraError::Json`] if the wrapper
/// document cannot be serialized.
pub fn unwrapped_from_parts(
    topic: &str,
    meta: WrapperMeta,
    encoded: Bytes,
) -> Result<Unwrapped, AgraError> {
    if encoded.len() > MAX_DECODED_PAYLOAD {
        return Err(AgraError::PayloadTooLarge);
    }
    let inner_bytes = encoded.to_vec();
    let inner_ct = sniff_content_type(&inner_bytes);
    let inner_hint = type_hint_from_inner(&inner_bytes, &meta.message_type_enum);
    let wrapper_json = minimal_wrapper_json(&meta, &inner_bytes)?;
    let wrapper = Envelope::new(
        RouteKey::typed(topic, meta.kind.element_name()),
        wrapper_json,
    )
    .with_content_type(ContentType::json());
    let inner = inner_envelope(topic, inner_bytes, inner_ct, inner_hint, &meta);
    Ok(Unwrapped {
        wrapper,
        inner,
        meta,
    })
}

fn minimal_wrapper_json(meta: &WrapperMeta, inner: &[u8]) -> Result<Bytes, AgraError> {
    let mut data = json!({
        "MessageType": meta.message_type_enum,
        "EncodedPayload": hex::encode_upper(inner),
    });
    if let Some(v) = &meta.originator_uuid {
        data["DataPayloadOriginatorID"] = json!({ "UUID": v });
    }
    if let Some(v) = &meta.rx_payload_id {
        data["RxDataPayloadID"] = json!({ "UUID": v });
    }
    if let Some(v) = &meta.command_id {
        data["CommandID"] = json!({ "UUID": v });
    }
    if let Some(v) = &meta.destination_routing {
        data["DestinationRouting"] = json!(v);
    }
    let root = json!({
        meta.kind.element_name(): { "MessageData": data }
    });
    let text = serde_json::to_string(&root).map_err(|e| AgraError::Json(e.to_string()))?;
    Ok(Bytes::from(text))
}

/// Unwrap OMS JSON or XML Rx/Tx envelopes into wrapper + inner envelopes.
pub fn unwrap(topic: &str, payload: &[u8]) -> Result<Unwrapped, AgraError> {
    let text = std::str::from_utf8(payload)
        .map_err(|_| AgraError::Json("wrapper payload is not UTF-8".into()))?;
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        unwrap_json(topic, trimmed)
    } else if trimmed.starts_with('<') {
        unwrap_xml(topic, trimmed)
    } else {
        Err(AgraError::NotAWrapper)
    }
}

fn unwrap_json(topic: &str, text: &str) -> Result<Unwrapped, AgraError> {
    let root: Value = serde_json::from_str(text).map_err(|e| AgraError::Json(e.to_string()))?;
    let obj = root
        .as_object()
        .ok_or_else(|| AgraError::Json("root must be an object".into()))?;
    if obj.len() != 1 {
        return Err(AgraError::NotAWrapper);
    }
    let (name, body) = obj.iter().next().expect("len == 1");
    let kind = WrapperKind::from_element(name).ok_or(AgraError::NotAWrapper)?;
    let data = body
        .get("MessageData")
        .ok_or(AgraError::MissingField("MessageData"))?;

    let message_type_enum = json_str(data, "MessageType")?;
    let hex_payload = json_str(data, "EncodedPayload")?;
    let inner_bytes = decode_hex(hex_payload)?;
    let inner_ct = sniff_content_type(&inner_bytes);
    let inner_hint = type_hint_from_inner(&inner_bytes, message_type_enum);

    let meta = WrapperMeta {
        kind,
        message_type_enum: message_type_enum.to_owned(),
        originator_uuid: data
            .pointer("/DataPayloadOriginatorID/UUID")
            .and_then(Value::as_str)
            .map(str::to_owned),
        rx_payload_id: data
            .pointer("/RxDataPayloadID/UUID")
            .and_then(Value::as_str)
            .map(str::to_owned),
        command_id: data
            .pointer("/CommandID/UUID")
            .and_then(Value::as_str)
            .map(str::to_owned),
        destination_routing: data
            .get("DestinationRouting")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };

    let wrapper = Envelope::new(
        RouteKey::typed(topic, kind.element_name()),
        Bytes::copy_from_slice(text.as_bytes()),
    )
    .with_content_type(ContentType::json());
    let inner = inner_envelope(topic, inner_bytes, inner_ct, inner_hint, &meta);
    Ok(Unwrapped {
        wrapper,
        inner,
        meta,
    })
}

fn unwrap_xml(topic: &str, text: &str) -> Result<Unwrapped, AgraError> {
    let root = xml_root_local_name(text).ok_or(AgraError::NotAWrapper)?;
    let kind = WrapperKind::from_element(&root).ok_or(AgraError::NotAWrapper)?;
    let message_type_enum =
        xml_local_text(text, "MessageType").ok_or(AgraError::MissingField("MessageType"))?;
    let hex_payload =
        xml_local_text(text, "EncodedPayload").ok_or(AgraError::MissingField("EncodedPayload"))?;
    let inner_bytes = decode_hex(hex_payload.trim())?;
    let inner_ct = sniff_content_type(&inner_bytes);
    let inner_hint = type_hint_from_inner(&inner_bytes, message_type_enum);

    let meta = WrapperMeta {
        kind,
        message_type_enum: message_type_enum.to_owned(),
        originator_uuid: xml_nested_uuid(text, "DataPayloadOriginatorID"),
        rx_payload_id: xml_nested_uuid(text, "RxDataPayloadID"),
        command_id: xml_nested_uuid(text, "CommandID"),
        destination_routing: xml_local_text(text, "DestinationRouting").map(str::to_owned),
    };

    let wrapper = Envelope::new(
        RouteKey::typed(topic, kind.element_name()),
        Bytes::copy_from_slice(text.as_bytes()),
    )
    .with_content_type(ContentType::xml());
    let inner = inner_envelope(topic, inner_bytes, inner_ct, inner_hint, &meta);
    Ok(Unwrapped {
        wrapper,
        inner,
        meta,
    })
}

fn inner_envelope(
    topic: &str,
    inner_bytes: Vec<u8>,
    inner_ct: ContentType,
    inner_hint: String,
    meta: &WrapperMeta,
) -> Envelope {
    let mut env = Envelope::new(RouteKey::typed(topic, inner_hint), inner_bytes)
        .with_content_type(inner_ct.clone())
        .with_header(HDR_WRAPPER, meta.kind.element_name())
        .with_header(HDR_MESSAGE_TYPE, meta.message_type_enum.clone())
        .with_header(HDR_INNER_CT, inner_ct.as_str());
    if let Some(v) = &meta.originator_uuid {
        env = env.with_header(HDR_ORIGINATOR, v);
    }
    if let Some(v) = &meta.rx_payload_id {
        env = env.with_header(HDR_RX_ID, v);
    }
    if let Some(v) = &meta.command_id {
        env = env.with_header(HDR_COMMAND_ID, v);
    }
    if let Some(v) = &meta.destination_routing {
        env = env.with_header(HDR_DEST_ROUTING, v);
    }
    env
}

/// Classification / header shell supplied by the caller — we do not invent markings.
#[derive(Debug, Clone)]
pub struct WrapShell {
    pub security_information: Value,
    pub message_header: Value,
}

/// Inputs for building an Rx or Tx wrapper around an inner UCI payload.
#[derive(Debug, Clone)]
pub struct WrapRequest {
    pub topic: String,
    pub kind: WrapperKind,
    pub inner: Bytes,
    pub message_type_enum: String,
    pub shell: WrapShell,
    pub destination_routing: String,
    pub originator_uuid: Option<String>,
    pub specific_destination_uuids: Vec<String>,
    pub timestamp: String,
    pub priority: u16,
    pub precedence: u16,
}

/// Wrap an inner UCI payload (XML or OMS JSON bytes) as hexBinary inside Rx/Tx OMS JSON.
pub fn wrap(req: WrapRequest) -> Result<Envelope, AgraError> {
    let encoded = hex::encode_upper(&req.inner);
    let ranking = json!({
        "Priority": req.priority,
        "PrecedenceWithinPriority": req.precedence,
    });
    let destinations: Vec<Value> = req
        .specific_destination_uuids
        .iter()
        .map(|u| json!({ "UUID": u }))
        .collect();

    let mut data = match req.kind {
        WrapperKind::Rx => {
            let rx_id = uuid::Uuid::new_v4().to_string();
            let originator = req
                .originator_uuid
                .clone()
                .unwrap_or_else(|| uuid::Uuid::nil().to_string());
            json!({
                "RxDataPayloadID": { "UUID": rx_id },
                "DataPayloadOriginatorID": { "UUID": originator },
                "EncodedPayload": encoded,
                "Timestamp": req.timestamp,
                "MessageType": req.message_type_enum,
                "DestinationRouting": req.destination_routing,
            })
        }
        WrapperKind::Tx => {
            let command_id = uuid::Uuid::new_v4().to_string();
            json!({
                "CommandID": { "UUID": command_id },
                "CommandState": "NEW",
                "EncodedPayload": encoded,
                "MessageType": req.message_type_enum,
                "Priority": ranking,
                "Timestamp": req.timestamp,
                "DestinationRouting": req.destination_routing,
            })
        }
    };

    if req.kind == WrapperKind::Rx {
        if let Value::Object(map) = &mut data {
            map.insert("Priority".into(), ranking);
        }
    }
    if !destinations.is_empty() {
        if let Value::Object(map) = &mut data {
            map.insert("SpecificDestinationID".into(), Value::Array(destinations));
        }
    }

    let root = json!({
        req.kind.element_name(): {
            "SecurityInformation": req.shell.security_information,
            "MessageHeader": req.shell.message_header,
            "MessageData": data,
        }
    });
    let text = serde_json::to_string(&root).map_err(|e| AgraError::Json(e.to_string()))?;
    Ok(Envelope::new(
        RouteKey::typed(req.topic, req.kind.element_name()),
        text.into_bytes(),
    )
    .with_content_type(ContentType::json()))
}

/// Convert a global element name (`PositionReport`) to `MessageTypeEnum` (`POSITION_REPORT`).
#[must_use]
pub fn element_to_enum(name: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for ch in name.chars() {
        if ch == '_' {
            out.push('_');
            prev = Some('_');
            continue;
        }
        if ch.is_ascii_uppercase()
            && prev.is_some_and(|p| p.is_ascii_lowercase())
            && !out.ends_with('_')
        {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
        prev = Some(ch);
    }
    out
}

fn type_hint_from_inner(bytes: &[u8], enum_name: &str) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        let t = text.trim();
        if t.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                if let Some(obj) = v.as_object() {
                    if obj.len() == 1 {
                        if let Some(k) = obj.keys().next() {
                            return k.clone();
                        }
                    }
                }
            }
        }
        if t.starts_with('<') {
            if let Some(name) = xml_root_local_name(t) {
                return name;
            }
        }
    }
    enum_name.to_owned()
}

fn sniff_content_type(bytes: &[u8]) -> ContentType {
    match std::str::from_utf8(bytes).map(str::trim_start) {
        Ok(t) if t.starts_with('{') => ContentType::json(),
        Ok(t) if t.starts_with('<') => ContentType::xml(),
        _ => ContentType::octet_stream(),
    }
}

/// Largest payload an `EncodedPayload` element may decode to.
///
/// The adapters cap an inbound frame at 16 MiB, and hex doubles what it encodes,
/// so this is the bound that already applies — stated here so that raising a
/// frame limit does not silently raise this one too, and so the crate holds on
/// its own if some future caller has no frame cap of its own.
pub const MAX_DECODED_PAYLOAD: usize = 8 * 1024 * 1024;

/// Decode `xs:hexBinary`, tolerating the whitespace the type permits.
///
/// Hand-rolled rather than `hex::decode` on a filtered copy, which would hold
/// the whole payload twice: once stripped, once decoded. Refuses before
/// allocating past [`MAX_DECODED_PAYLOAD`] rather than after.
fn decode_hex(s: &str) -> Result<Vec<u8>, AgraError> {
    // An upper bound, not the answer: whitespace makes the input longer than
    // twice the output, never shorter.
    let bound = s.len() / 2;
    if bound > MAX_DECODED_PAYLOAD {
        // Only refuse once the digits are known to be there, since a huge run of
        // whitespace is malformed rather than oversized.
        let digits = s.bytes().filter(|b| !b.is_ascii_whitespace()).count();
        if digits / 2 > MAX_DECODED_PAYLOAD {
            return Err(AgraError::PayloadTooLarge);
        }
    }

    let mut out = Vec::with_capacity(bound.min(MAX_DECODED_PAYLOAD));
    let mut high: Option<u8> = None;
    for b in s.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        let nibble = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            other => {
                return Err(AgraError::BadHex(format!(
                    "invalid character {:?}",
                    char::from(other)
                )))
            }
        };
        match high {
            None => high = Some(nibble),
            Some(hi) => {
                out.push((hi << 4) | nibble);
                high = None;
            }
        }
    }
    if high.is_some() {
        return Err(AgraError::BadHex("odd number of digits".into()));
    }
    Ok(out)
}

fn json_str<'a>(data: &'a Value, field: &'static str) -> Result<&'a str, AgraError> {
    data.get(field)
        .and_then(Value::as_str)
        .ok_or(AgraError::MissingField(field))
}

/// Local name of an XML document element, or `None` if there is no element.
///
/// A declaration, comment, or DOCTYPE ahead of the root is stepped over rather
/// than treated as the element. Producers emit `<?xml …?>` as a matter of
/// course, so stopping at the first `<` reports no element at all — which makes
/// [`wrapper_kind`] answer "not a wrapper" for perfectly ordinary input and
/// silently disables unwrapping.
#[must_use]
pub fn xml_root_local_name(xml: &str) -> Option<String> {
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

fn xml_local_text<'a>(xml: &'a str, local: &str) -> Option<&'a str> {
    let patterns = [format!("<{local}>"), format!(":{local}>")];
    for pat in &patterns {
        if let Some(idx) = xml.find(pat) {
            let after = idx + pat.len();
            if let Some(rel) = xml[after..].find("</") {
                return Some(&xml[after..after + rel]);
            }
        }
    }
    None
}

fn xml_nested_uuid(xml: &str, parent_local: &str) -> Option<String> {
    let open = xml
        .find(&format!("<{parent_local}"))
        .or_else(|| xml.find(&format!(":{parent_local}")))?;
    let slice = &xml[open..];
    xml_local_text(slice, "UUID").map(str::to_owned)
}

/// Copy wrapper metadata onto an existing header map (adapters).
#[must_use]
pub fn merge_meta_headers(
    mut headers: BTreeMap<String, String>,
    meta: &WrapperMeta,
) -> BTreeMap<String, String> {
    headers.insert(HDR_WRAPPER.into(), meta.kind.element_name().into());
    headers.insert(HDR_MESSAGE_TYPE.into(), meta.message_type_enum.clone());
    if let Some(v) = &meta.originator_uuid {
        headers.insert(HDR_ORIGINATOR.into(), v.clone());
    }
    if let Some(v) = &meta.command_id {
        headers.insert(HDR_COMMAND_ID.into(), v.clone());
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner_json() -> &'static str {
        r#"{"PositionReport":{"SecurityInformation":{"Classification":"U"},"MessageHeader":{"SchemaVersion":"002.5.0"},"MessageData":{"n":1}}}"#
    }

    fn shell() -> WrapShell {
        WrapShell {
            security_information: json!({"Classification":"U","OwnerProducer":[{"US":true}]}),
            message_header: json!({
                "SystemID": {"UUID": "00000000-0000-4000-8000-000000000001"},
                "Timestamp": "2026-08-09T21:00:00Z",
                "SchemaVersion": "002.5.0",
                "Mode": "SIMULATION"
            }),
        }
    }

    #[test]
    fn element_enum_round_names() {
        assert_eq!(element_to_enum("PositionReport"), "POSITION_REPORT");
        assert_eq!(element_to_enum("MA_WEZ"), "MA_WEZ");
        assert_eq!(
            element_to_enum("MA_TxDataPayloadCommand"),
            "MA_TX_DATA_PAYLOAD_COMMAND"
        );
    }

    #[test]
    fn wrap_rx_then_unwrap_json() {
        let inner = Bytes::from(inner_json());
        let wrapped = wrap(WrapRequest {
            topic: "offboard".into(),
            kind: WrapperKind::Rx,
            inner: inner.clone(),
            message_type_enum: "POSITION_REPORT".into(),
            shell: shell(),
            destination_routing: "TOPIC_AND_SPECIFIC_DESTINATION".into(),
            originator_uuid: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
            specific_destination_uuids: vec![],
            timestamp: "2026-08-09T21:00:00Z".into(),
            priority: 4,
            precedence: 1,
        })
        .unwrap();

        assert_eq!(wrapped.route.type_hint.as_deref(), Some(RX_ELEMENT));
        assert!(wrapper_kind(&wrapped.payload).is_some());

        let u = unwrap("offboard", &wrapped.payload).unwrap();
        assert_eq!(u.meta.kind, WrapperKind::Rx);
        assert_eq!(u.inner.route.type_hint.as_deref(), Some("PositionReport"));
        assert_eq!(u.inner.payload, inner);
        assert_eq!(
            u.inner
                .headers
                .get("agra.originator_uuid")
                .map(String::as_str),
            Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
        );
        assert_eq!(u.wrapper.route.type_hint.as_deref(), Some(RX_ELEMENT));
    }

    #[test]
    fn parts_round_trip_to_inner_bytes() {
        let inner = Bytes::from(inner_json());
        let meta = WrapperMeta {
            kind: WrapperKind::Rx,
            message_type_enum: "POSITION_REPORT".into(),
            originator_uuid: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
            rx_payload_id: None,
            command_id: None,
            destination_routing: None,
        };
        let u = unwrapped_from_parts("demo", meta, inner.clone()).unwrap();
        assert_eq!(u.inner.payload, inner);
        assert_eq!(u.inner.route.type_hint.as_deref(), Some("PositionReport"));
        assert_eq!(u.wrapper.route.type_hint.as_deref(), Some(RX_ELEMENT));
        assert!(wrapper_kind(&u.wrapper.payload).is_some());
        let again = unwrap("demo", &u.wrapper.payload).unwrap();
        assert_eq!(again.inner.payload, inner);
    }

    #[test]
    fn wrap_tx_then_unwrap_preserves_command_id() {
        let wrapped = wrap(WrapRequest {
            topic: "offboard".into(),
            kind: WrapperKind::Tx,
            inner: Bytes::from(inner_json()),
            message_type_enum: "POSITION_REPORT".into(),
            shell: shell(),
            destination_routing: "SPECIFIC_DESTINATION_ONLY".into(),
            originator_uuid: None,
            specific_destination_uuids: vec!["bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into()],
            timestamp: "2026-08-09T21:00:00Z".into(),
            priority: 0,
            precedence: 0,
        })
        .unwrap();

        let u = unwrap("offboard", &wrapped.payload).unwrap();
        assert_eq!(u.meta.kind, WrapperKind::Tx);
        assert!(u.meta.command_id.is_some());
        assert_eq!(
            u.inner.headers.get("agra.command_id"),
            u.meta.command_id.as_ref()
        );
        assert_eq!(
            u.meta.destination_routing.as_deref(),
            Some("SPECIFIC_DESTINATION_ONLY")
        );
    }

    #[test]
    fn unwrap_xml_rx() {
        let hex = hex::encode_upper(inner_json().as_bytes());
        let xml = format!(
            r#"<MA_RxDataPayload xmlns="https://www.vdl.afrl.af.mil/programs/oam">
              <SecurityInformation><Classification>U</Classification></SecurityInformation>
              <MessageHeader><SchemaVersion>002.5.0</SchemaVersion></MessageHeader>
              <MessageData>
                <RxDataPayloadID><UUID>11111111-1111-4111-8111-111111111111</UUID></RxDataPayloadID>
                <DataPayloadOriginatorID><UUID>22222222-2222-4222-8222-222222222222</UUID></DataPayloadOriginatorID>
                <EncodedPayload>{hex}</EncodedPayload>
                <Timestamp>2026-08-09T21:00:00Z</Timestamp>
                <MessageType>POSITION_REPORT</MessageType>
                <DestinationRouting>TOPIC_AND_SPECIFIC_DESTINATION</DestinationRouting>
              </MessageData>
            </MA_RxDataPayload>"#
        );
        let u = unwrap("ext", xml.as_bytes()).unwrap();
        assert_eq!(u.meta.kind, WrapperKind::Rx);
        assert_eq!(u.inner.route.type_hint.as_deref(), Some("PositionReport"));
        assert_eq!(
            u.meta.rx_payload_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
    }

    #[test]
    fn native_mt_is_not_a_wrapper() {
        assert!(wrapper_kind(inner_json().as_bytes()).is_none());
        assert!(unwrap("demo", inner_json().as_bytes()).is_err());
    }

    /// A wrapper as an actual producer emits it: an XML declaration ahead of the
    /// root, and an inner payload carrying one of its own.
    ///
    /// Shaped after live `MA_TxDataPayloadCommand` traffic from an A-GRA MA. The
    /// declarations are the point of the fixture — without them this passes even
    /// when prolog handling is broken, which is how it stayed broken.
    fn tx_wrapper_with_prolog() -> String {
        let inner = concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<SystemStatus xmlns="https://www.vdl.afrl.af.mil/programs/oam">"#,
            r#"<MessageHeader><SchemaVersion>005.0a</SchemaVersion></MessageHeader>"#,
            r#"</SystemStatus>"#
        );
        format!(
            concat!(
                r#"<?xml version="1.0" encoding="utf-8"?>"#,
                "\n",
                r#"<MA_TxDataPayloadCommand xmlns="https://www.vdl.afrl.af.mil/programs/oam">"#,
                r#"<MessageHeader><SchemaVersion>005.0a</SchemaVersion></MessageHeader>"#,
                r#"<MessageData>"#,
                r#"<CommandID><UUID>7ea053eadcc545baac26d5bc909417dc</UUID></CommandID>"#,
                r#"<CommandState>NEW</CommandState>"#,
                r#"<EncodedPayload>{hex}</EncodedPayload>"#,
                r#"<MessageType>SYSTEM_STATUS</MessageType>"#,
                r#"</MessageData></MA_TxDataPayloadCommand>"#
            ),
            hex = hex::encode(inner.as_bytes())
        )
    }

    #[test]
    fn xml_declaration_does_not_hide_the_wrapper() {
        let xml = tx_wrapper_with_prolog();
        assert_eq!(wrapper_kind(xml.as_bytes()), Some(WrapperKind::Tx));

        let u = unwrap("MA_TxDataPayloadCommand", xml.as_bytes()).unwrap();
        assert_eq!(u.meta.kind, WrapperKind::Tx);
        // The element name, not the MessageType enum it falls back to.
        assert_eq!(u.inner.route.type_hint.as_deref(), Some("SystemStatus"));
        assert_eq!(u.wrapper.route.topic, "MA_TxDataPayloadCommand");
        assert_eq!(u.inner.route.topic, "MA_TxDataPayloadCommand");
    }

    #[test]
    fn hex_decoding_tolerates_whitespace_and_rejects_the_rest() {
        // xs:hexBinary permits whitespace, and producers wrap long payloads.
        assert_eq!(decode_hex("48 65\n6c 6C\t6f").unwrap(), b"Hello");
        assert_eq!(decode_hex("").unwrap(), b"");

        assert!(matches!(decode_hex("abc"), Err(AgraError::BadHex(_))));
        assert!(matches!(decode_hex("zz"), Err(AgraError::BadHex(_))));
    }

    #[test]
    fn hex_decoding_refuses_an_oversized_payload() {
        let too_big = "00".repeat(MAX_DECODED_PAYLOAD + 1);
        assert!(matches!(
            decode_hex(&too_big),
            Err(AgraError::PayloadTooLarge)
        ));

        // Whitespace inflates the input past the bound without inflating the
        // payload, and must not be mistaken for an oversized one.
        let padded = format!("{:width$}41", "", width = MAX_DECODED_PAYLOAD * 2 + 4);
        assert_eq!(decode_hex(&padded).unwrap(), b"A");
    }

    #[test]
    fn root_name_skips_prolog_comments_and_doctype() {
        assert_eq!(xml_root_local_name("<Ping/>").as_deref(), Some("Ping"));
        assert_eq!(
            xml_root_local_name(r#"<?xml version="1.0"?><Ping/>"#).as_deref(),
            Some("Ping")
        );
        assert_eq!(
            xml_root_local_name("<!-- note --><Ping/>").as_deref(),
            Some("Ping")
        );
        assert_eq!(
            xml_root_local_name("<!DOCTYPE Ping><Ping/>").as_deref(),
            Some("Ping")
        );
        // Prefixed names reduce to the local part.
        assert_eq!(
            xml_root_local_name(r#"<uci:Ping xmlns:uci="x"/>"#).as_deref(),
            Some("Ping")
        );
        assert_eq!(xml_root_local_name("not xml"), None);
        assert_eq!(xml_root_local_name(r#"<?xml version="1.0"?>"#), None);
        // Unterminated input must not spin or panic.
        assert_eq!(xml_root_local_name("<?xml"), None);
        assert_eq!(xml_root_local_name("<!-- open"), None);
        assert_eq!(xml_root_local_name("<Ping"), None);
    }
}
