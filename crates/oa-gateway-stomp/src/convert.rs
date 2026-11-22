//! UCI schema-violation check for STOMP frames, in either direction.

use oa_gateway_adapter::ValidateMode;
use oa_gateway_xsd_libxml::{XsdError, XsdLibxmlValidator};

/// Everything in `payload` that `validator` does not permit.
///
/// libxml2 validates XML instances directly; `payload` is already UCI
/// XML, since STOMP never carries anything else.
pub(crate) async fn violations_of(
    payload: &[u8],
    validator: Option<&XsdLibxmlValidator>,
    mode: ValidateMode,
) -> Vec<XsdError> {
    if !mode.is_on() {
        return Vec::new();
    }
    let Some(validator) = validator else {
        return Vec::new();
    };
    let Ok(text) = std::str::from_utf8(payload) else {
        return Vec::new();
    };
    validator
        .validate(text.to_owned())
        .await
        .err()
        .unwrap_or_default()
}
