//! Schema-aware UCI XML ↔ OMS JSON.
//!
//! The engine stays on opaque bytes. Adapters call [`Message::from_json`] /
//! [`Message::from_xml`] and emit the other serialization.
//!
//! A [`Schema`] tells the converter what it cannot infer from a single document:
//! whether a field repeats, and whether a leaf is a number, a boolean, or a
//! string. Compile one from the published XSD with [`xsd::compile`]. The
//! [`mod@slice`] module holds a small hand-written schema for tests only.

mod error;
mod instance;
mod json;
mod schema;
pub mod slice;
pub mod validate;
mod xml;
pub mod xsd;

/// Deepest element nesting accepted when converting a payload.
///
/// Conversion walks a document recursively, so nesting is a stack-depth
/// question: unbounded, a document deep enough ends the process rather than the
/// message.
///
/// The number is bracketed rather than chosen. `SystemReadiness`, the deepest
/// message in the published catalog, declares 39 levels — so a limit anywhere
/// near that would refuse real traffic, and the ignored `published_schema` test
/// re-measures it against whatever schema you compile. serde_json refuses at
/// 128 nested values on its own, so a limit above that would leave JSON and XML
/// failing at two different depths for the same payload. This sits between the
/// two, with room over the catalog and none borrowed from serde_json.
pub const MAX_DEPTH: usize = 96;

pub use error::UciError;
pub use instance::{Complex, Field, Message, Node, Simple};
pub use schema::{
    choice, el, el_many, el_opt, sequence, ComplexContent, ComplexType, Element, Group, GroupKind,
    MaxOccurs, Schema,
};
pub use validate::{validate, Mode as ValidateMode, Violation, ViolationKind};

impl Message {
    /// Every way this message departs from `schema`; empty when none.
    ///
    /// See [`mod@validate`] for what that does and does not cover.
    #[must_use]
    pub fn violations(&self, schema: &Schema) -> Vec<Violation> {
        validate::validate(self, schema)
    }

    pub fn from_json(text: &str, schema: &Schema) -> Result<Self, UciError> {
        json::from_json(text, schema)
    }

    pub fn from_xml(text: &str, schema: &Schema) -> Result<Self, UciError> {
        xml::from_xml(text, schema)
    }

    pub fn to_json(&self, schema: &Schema) -> Result<String, UciError> {
        json::to_json(self, schema)
    }

    pub fn to_xml(&self, schema: &Schema) -> Result<String, UciError> {
        xml::to_xml(self, schema)
    }

    #[must_use]
    pub fn type_hint(&self) -> &str {
        &self.name
    }
}

/// Detect XML vs OMS JSON without a schema.
#[must_use]
pub fn looks_like_xml(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .is_some_and(|s| s.trim_start().starts_with('<'))
}

#[must_use]
pub fn looks_like_json(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .is_some_and(|s| s.trim_start().starts_with('{'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn schema() -> &'static Schema {
        slice::v25()
    }

    fn json_eq(a: &str, b: &str) {
        let va: Value = serde_json::from_str(a).unwrap();
        let vb: Value = serde_json::from_str(b).unwrap();
        assert_eq!(va, vb);
    }

    /// A type that contains itself, so nesting can be declared without end.
    ///
    /// The published catalog has no such type, but a payload's depth is chosen
    /// by whoever sends it, and a schema is an input too.
    fn recursive_schema() -> Schema {
        let mut s = Schema::new();
        s.complex(
            "NestType",
            vec![el_opt("Nest", "NestType"), el_opt("leaf", "xs:string")],
        )
        .element("Nest", "NestType");
        s
    }

    fn nested_json(levels: usize) -> String {
        let mut out = String::from(r#"{"leaf":"x"}"#);
        for _ in 0..levels {
            out = format!(r#"{{"Nest":{out}}}"#);
        }
        format!(r#"{{"Nest":{out}}}"#)
    }

    fn nested_xml(levels: usize) -> String {
        let mut out = String::from("<leaf>x</leaf>");
        for _ in 0..levels {
            out = format!("<Nest>{out}</Nest>");
        }
        format!(r#"<Nest xmlns="https://www.vdl.afrl.af.mil/programs/oam">{out}</Nest>"#)
    }

    // The exact boundary is not a contract; that ordinary nesting converts and
    // hostile nesting fails cleanly is.
    #[test]
    fn nesting_within_the_limit_converts() {
        let schema = recursive_schema();
        let levels = MAX_DEPTH / 2;

        let json = nested_json(levels);
        let from_json = Message::from_json(&json, &schema).expect("json within the limit");
        assert_eq!(from_json.type_hint(), "Nest");
        from_json.to_xml(&schema).expect("xml out within the limit");

        let xml = nested_xml(levels);
        let from_xml = Message::from_xml(&xml, &schema).expect("xml within the limit");
        from_xml
            .to_json(&schema)
            .expect("json out within the limit");
    }

    #[test]
    fn nesting_past_the_limit_is_refused() {
        let schema = recursive_schema();
        let levels = MAX_DEPTH + 2;

        let err = Message::from_json(&nested_json(levels), &schema).unwrap_err();
        assert!(matches!(err, UciError::TooDeep { .. }), "json: {err}");

        let err = Message::from_xml(&nested_xml(levels), &schema).unwrap_err();
        assert!(matches!(err, UciError::TooDeep { .. }), "xml: {err}");
    }

    /// The point of the limit: a document deep enough to exhaust the stack has to
    /// come back as an error, from the parser or from us, and not as a crash.
    #[test]
    fn absurdly_deep_json_fails_instead_of_aborting() {
        let schema = recursive_schema();
        assert!(Message::from_json(&nested_json(50_000), &schema).is_err());
    }

    #[test]
    fn absurdly_deep_xml_fails_instead_of_aborting() {
        let schema = recursive_schema();
        assert!(Message::from_xml(&nested_xml(50_000), &schema).is_err());
    }

    #[test]
    fn a_cyclic_extension_chain_is_reported_not_followed() {
        let schema = xsd::compile(&[r#"
            <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
              <xs:complexType name="AType">
                <xs:complexContent>
                  <xs:extension base="BType"/>
                </xs:complexContent>
              </xs:complexType>
              <xs:complexType name="BType">
                <xs:complexContent>
                  <xs:extension base="AType"/>
                </xs:complexContent>
              </xs:complexType>
            </xs:schema>
        "#])
        .expect("both bases resolve, so compiling is not where this fails");

        let err = schema.flatten("AType").unwrap_err();
        match err {
            UciError::Xsd(message) => assert!(
                message.contains("cyclic extension chain"),
                "unexpected message: {message}"
            ),
            other => panic!("expected an XSD error, got {other}"),
        }
    }

    #[test]
    fn ping_json_xml_json() {
        let src = r#"{"Ping":{"n":7}}"#;
        let msg = Message::from_json(src, schema()).unwrap();
        assert_eq!(msg.type_hint(), "Ping");
        let xml = msg.to_xml(schema()).unwrap();
        assert!(xml.contains("<Ping"));
        assert!(xml.contains("<n>7</n>"));
        let back = Message::from_xml(&xml, schema()).unwrap();
        json_eq(&back.to_json(schema()).unwrap(), src);
    }

    #[test]
    fn position_report_sleet_fixture_roundtrip() {
        let src = oa_gateway_testing::fixtures::POSITION_REPORT_JSON;
        let msg = Message::from_json(src, schema()).unwrap();
        let xml = msg.to_xml(schema()).unwrap();
        assert!(xml.contains("<OwnerProducer>"));
        let owners = match &msg.body {
            Node::Complex(c) => c.get("SecurityInformation"),
            _ => None,
        };
        assert!(owners.is_some());
        let back = Message::from_xml(&xml, schema()).unwrap();
        json_eq(&back.to_json(schema()).unwrap(), src);
    }

    #[test]
    fn fixture_xml_to_json() {
        let xml = oa_gateway_testing::fixtures::POSITION_REPORT_XML;
        let msg = Message::from_xml(xml, schema()).unwrap();
        let value: Value = serde_json::from_str(&msg.to_json(schema()).unwrap()).unwrap();
        assert_eq!(
            value.pointer("/PositionReport/MessageData/n"),
            Some(&json!(1))
        );
        assert_eq!(
            value.pointer("/PositionReport/SecurityInformation/Classification"),
            Some(&json!("U"))
        );
    }

    #[test]
    fn owner_producer_is_json_array() {
        let src = r#"{
            "PositionReport": {
                "SecurityInformation": {
                    "Classification": "U",
                    "OwnerProducer": [
                        {"GovernmentIdentifier": "USA"},
                        {"GovernmentIdentifier": "GBR"}
                    ]
                }
            }
        }"#;
        let msg = Message::from_json(src, schema()).unwrap();
        let xml = msg.to_xml(schema()).unwrap();
        assert_eq!(xml.matches("<OwnerProducer>").count(), 2);
        let back: Value = serde_json::from_str(
            &Message::from_xml(&xml, schema())
                .unwrap()
                .to_json(schema())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            back.pointer("/PositionReport/SecurityInformation/OwnerProducer")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn poly_sample_type_attribute() {
        let src = r#"{
            "PolySample": {
                "Detail": {
                    "$type": "InertialDetail",
                    "kind": "pos",
                    "Latitude": 1.0,
                    "Longitude": 2.0
                }
            }
        }"#;
        let msg = Message::from_json(src, schema()).unwrap();
        let xml = msg.to_xml(schema()).unwrap();
        assert!(xml.contains("xsi:type=\"InertialDetail\""), "{xml}");
        let back = Message::from_xml(&xml, schema()).unwrap();
        let json = back.to_json(schema()).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v.pointer("/PolySample/Detail/$type"),
            Some(&json!("InertialDetail"))
        );
        assert_eq!(v.pointer("/PolySample/Detail/Latitude"), Some(&json!(1.0)));
    }

    #[test]
    fn ma_rx_hex_payload_survives() {
        let src = r#"{
            "MA_RxDataPayload": {
                "MessageData": {
                    "EncodedPayload": "DEADBEEF",
                    "MessageType": "POSITION_REPORT"
                }
            }
        }"#;
        let xml = Message::from_json(src, schema())
            .unwrap()
            .to_xml(schema())
            .unwrap();
        assert!(xml.contains("<EncodedPayload>DEADBEEF</EncodedPayload>"));
        let back = Message::from_xml(&xml, schema())
            .unwrap()
            .to_json(schema())
            .unwrap();
        json_eq(&back, src);
    }
}
