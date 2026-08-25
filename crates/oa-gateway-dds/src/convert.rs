//! UCI schema-violation check for inbound DDS samples.

use oa_gateway_uci::validate::{Mode as ValidateMode, Violation};
use oa_gateway_uci::Schema;

/// Everything in `payload` that `schema` does not permit.
///
/// Parses on its own account rather than reusing any earlier parse. A
/// payload that will not parse at all is not reported here — unlike OWP,
/// DDS has no conversion step of its own to have already failed on it, so
/// this simply has nothing to say about the schema rather than saying it
/// twice.
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
