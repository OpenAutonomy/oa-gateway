//! JSON ↔ XML at the socket, plus A-GRA `EncodedPayload` hex.

use oa_gateway_core::{ContentType, Envelope};
use oa_gateway_uci::validate::{Mode as ValidateMode, Violation};
use oa_gateway_uci::Schema;

/// Everything in `payload` that `schema` does not permit.
///
/// Parses on its own account rather than reusing the conversion's parse. The two
/// run on different paths — conversion only when the XML baseline is on, this
/// whenever a schema is loaded and validation is not off — and one obvious check
/// is worth more than one saved parse. A payload that will not parse at all is a
/// conversion failure, reported where conversion happens, so nothing is said
/// about it twice.
pub(crate) fn violations_of(
    payload: &[u8],
    schema: Option<&Schema>,
    mode: ValidateMode,
) -> Vec<Violation> {
    if !mode.is_on() {
        return Vec::new();
    }
    let Some(schema) = schema else {
        return Vec::new();
    };
    let Ok(text) = std::str::from_utf8(payload) else {
        return Vec::new();
    };
    let parsed = if oa_gateway_uci::looks_like_xml(payload) {
        oa_gateway_uci::Message::from_xml(text, schema)
    } else {
        oa_gateway_uci::Message::from_json(text, schema)
    };
    parsed.map(|m| m.violations(schema)).unwrap_or_default()
}

/// Converts `env` to UCI XML when it is OMS JSON.
///
/// Already-XML payloads are labelled and returned. The type hint is
/// replaced with the converted message name. A-GRA `EncodedPayload`
/// hex is converted the same way as the wrapper.
///
/// # Errors
///
/// Returns a message if the bytes are not UTF-8, the JSON will not
/// parse as a UCI message, or the inner hex cannot be transcoded.
pub(crate) fn toward_xml(mut env: Envelope, schema: &Schema) -> Result<Envelope, String> {
    if oa_gateway_uci::looks_like_xml(&env.payload) {
        env.content_type = ContentType::xml();
        return Ok(env);
    }
    let text = std::str::from_utf8(&env.payload).map_err(|e| e.to_string())?;
    let text = transcode_wrapper_inner(text, schema, true)?;
    let msg = oa_gateway_uci::Message::from_json(&text, schema).map_err(|e| e.to_string())?;
    env.route.type_hint = Some(msg.name.clone());
    env.payload = bytes::Bytes::from(msg.to_xml(schema).map_err(|e| e.to_string())?);
    env.content_type = ContentType::xml();
    Ok(env)
}

/// Convert a payload the engine carried in XML into the OMS JSON a client
/// subscribed for.
///
/// A payload that is not XML is already what the client asked for and passes
/// through. Anything else is either converted or refused: the caller drops the
/// delivery and says so, rather than forwarding a document in a format the
/// client has no way to distinguish from an expected one.
///
/// # Errors
///
/// Returns a message if `raw` is XML and no schema is loaded, or the
/// document will not convert.
pub(crate) fn xml_to_oms_json(raw: &str, schema: Option<&Schema>) -> Result<String, String> {
    if !oa_gateway_uci::looks_like_xml(raw.as_bytes()) {
        return Ok(raw.to_owned());
    }
    // The host refuses to start with xml_baseline and no schema, so this is a
    // guard against an embedding that skips that check, not a reachable path.
    let schema = schema.ok_or("no UCI schema is loaded")?;
    let json = oa_gateway_uci::Message::from_xml(raw, schema)
        .and_then(|m| m.to_json(schema))
        .map_err(|e| e.to_string())?;
    transcode_wrapper_inner(&json, schema, false)
}

/// A-GRA `EncodedPayload` is opaque hex. OWP clients put OMS JSON in it; MA
/// parses XML. `xml_baseline` has to convert the inner the same way it
/// converts the wrapper, or MA logs `expected XML` and drops the path.
///
/// Non-wrapper JSON is returned unchanged. `want_xml` is the inner
/// format after conversion.
///
/// # Errors
///
/// Returns a message if the hex is not hexBinary or the inner document
/// will not convert.
fn transcode_wrapper_inner(text: &str, schema: &Schema, want_xml: bool) -> Result<String, String> {
    let mut root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let obj = match root.as_object_mut() {
        Some(obj) if obj.len() == 1 => obj,
        _ => return Ok(text.to_owned()),
    };
    let name = obj.keys().next().cloned().unwrap_or_default();
    if name != oa_gateway_agra::RX_ELEMENT && name != oa_gateway_agra::TX_ELEMENT {
        return Ok(text.to_owned());
    }
    let Some(encoded) = obj
        .get_mut(&name)
        .and_then(|body| body.get_mut("MessageData"))
        .and_then(|data| data.get_mut("EncodedPayload"))
    else {
        return Ok(text.to_owned());
    };
    let Some(hex) = encoded.as_str() else {
        return Ok(text.to_owned());
    };
    let inner_bytes = decode_hex(hex)?;
    let inner_text = std::str::from_utf8(&inner_bytes).map_err(|e| e.to_string())?;
    let inner_is_xml = inner_text.trim_start().starts_with('<');
    if want_xml == inner_is_xml {
        return Ok(text.to_owned());
    }
    let converted = if want_xml {
        let msg =
            oa_gateway_uci::Message::from_json(inner_text, schema).map_err(|e| e.to_string())?;
        msg.to_xml(schema).map_err(|e| e.to_string())?
    } else {
        let msg =
            oa_gateway_uci::Message::from_xml(inner_text, schema).map_err(|e| e.to_string())?;
        msg.to_json(schema).map_err(|e| e.to_string())?
    };
    *encoded = serde_json::Value::String(encode_hex_upper(converted.as_bytes()));
    serde_json::to_string(&root).map_err(|e| e.to_string())
}

/// Decodes hexBinary, ignoring ASCII whitespace.
///
/// # Errors
///
/// Returns a message if the digit count is odd or a character is not hex.
fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if digits.len() % 2 != 0 {
        return Err("EncodedPayload is not hexBinary: odd number of digits".into());
    }
    let mut out = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks_exact(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        other => Err(format!(
            "EncodedPayload is not hexBinary: invalid character {:?}",
            char::from(other)
        )),
    }
}

/// Encodes bytes as uppercase hex with no whitespace.
fn encode_hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_of(text: &str) -> String {
        encode_hex_upper(text.as_bytes())
    }

    #[test]
    fn xml_baseline_converts_json_encoded_payload_to_xml() {
        let inner = r#"{"PositionReport":{"MessageData":{"n":1}}}"#;
        let wrapper = format!(
            r#"{{"MA_RxDataPayload":{{"MessageData":{{"EncodedPayload":"{}","MessageType":"POSITION_REPORT"}}}}}}"#,
            hex_of(inner)
        );
        let schema = oa_gateway_uci::slice::v25();
        let converted = transcode_wrapper_inner(&wrapper, schema, true).unwrap();
        let hex = serde_json::from_str::<serde_json::Value>(&converted)
            .unwrap()
            .pointer("/MA_RxDataPayload/MessageData/EncodedPayload")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_owned();
        let inner_xml = String::from_utf8(decode_hex(&hex).unwrap()).unwrap();
        assert!(
            inner_xml.trim_start().starts_with('<'),
            "inner should be XML, got {inner_xml}"
        );
        assert!(inner_xml.contains("PositionReport"), "{inner_xml}");
    }

    #[test]
    fn xml_baseline_leaves_xml_encoded_payload_alone() {
        let inner = "<PositionReport><MessageData><n>1</n></MessageData></PositionReport>";
        let wrapper = format!(
            r#"{{"MA_RxDataPayload":{{"MessageData":{{"EncodedPayload":"{}"}}}}}}"#,
            hex_of(inner)
        );
        let schema = oa_gateway_uci::slice::v25();
        let converted = transcode_wrapper_inner(&wrapper, schema, true).unwrap();
        assert_eq!(converted, wrapper);
    }
}
