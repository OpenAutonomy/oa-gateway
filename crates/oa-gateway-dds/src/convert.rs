//! UCI schema-violation check for inbound DDS samples.

use oa_gateway_uci::validate::Mode as ValidateMode;
use oa_gateway_uci::Schema;
use oa_gateway_xsd_libxml::{XsdError, XsdLibxmlValidator};

/// Everything in `payload` that `validator` does not permit.
///
/// Parses on its own account rather than reusing any earlier parse. A
/// payload that will not parse at all is not reported here — unlike OWP,
/// DDS has no conversion step of its own to have already failed on it, so
/// this simply has nothing to say about the schema rather than saying it
/// twice. libxml2 validates XML instances, so a JSON-origin sample is
/// converted to XML first via the hand-rolled `Schema` (libxml2 has no
/// JSON concept at all).
pub(crate) async fn violations_of(
    payload: &[u8],
    schema: Option<&Schema>,
    validator: Option<&XsdLibxmlValidator>,
    mode: ValidateMode,
) -> Vec<XsdError> {
    if !mode.is_on() {
        return Vec::new();
    }
    let (Some(schema), Some(validator)) = (schema, validator) else {
        return Vec::new();
    };
    let Ok(text) = std::str::from_utf8(payload) else {
        return Vec::new();
    };
    let xml = if oa_gateway_uci::looks_like_xml(payload) {
        text.to_owned()
    } else {
        let Ok(msg) = oa_gateway_uci::Message::from_json(text, schema) else {
            return Vec::new();
        };
        let Ok(xml) = msg.to_xml(schema) else {
            return Vec::new();
        };
        xml
    };
    validator.validate(xml).await.err().unwrap_or_default()
}
