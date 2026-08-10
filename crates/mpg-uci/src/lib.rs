//! Schema-aware UCI XML ↔ OMS JSON.
//!
//! The engine stays on opaque bytes. Adapters call [`Message::from_json`] /
//! [`Message::from_xml`] and emit the other serialization. v0 ships a hand-built
//! [`slice`] of UCI 2.5, not the full XSD catalog.

mod error;
mod instance;
mod json;
mod schema;
pub mod slice;
mod xml;

pub use error::UciError;
pub use instance::{Complex, Field, Message, Node, Simple};
pub use schema::{el, el_many, el_opt, ComplexContent, ComplexType, Element, MaxOccurs, Schema};

impl Message {
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
        let src = mpg_testing::fixtures::POSITION_REPORT_JSON;
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
        let xml = mpg_testing::fixtures::POSITION_REPORT_XML;
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
