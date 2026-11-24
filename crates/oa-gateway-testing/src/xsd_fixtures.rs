//! Literal XSD text for the libxml2-backed validator, small enough to
//! write by hand for the tests that drive validation end to end.
//!
//! Real UCI/A-GRA traffic carries the OAM namespace on its root element,
//! so this schema targets that same namespace with
//! `elementFormDefault="qualified"`, or every document would fail on a
//! namespace mismatch before its shape was even considered.

use std::sync::OnceLock;

use oa_gateway_xsd_libxml::XsdLibxmlValidator;

const PING_XSD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
           xmlns="https://www.vdl.afrl.af.mil/programs/oam"
           targetNamespace="https://www.vdl.afrl.af.mil/programs/oam"
           elementFormDefault="qualified">
  <xs:element name="Ping" type="PingType"/>
  <xs:complexType name="PingType">
    <xs:sequence>
      <xs:element name="n" type="xs:int" minOccurs="0"/>
      <xs:element name="from" type="xs:string" minOccurs="0"/>
    </xs:sequence>
  </xs:complexType>
</xs:schema>
"#;

/// Compiled once, cloned out cheaply per test (a clone shares the same
/// compiled schema and dedicated thread).
#[must_use]
pub fn ping_validator() -> XsdLibxmlValidator {
    static VALIDATOR: OnceLock<XsdLibxmlValidator> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| XsdLibxmlValidator::compile_str(PING_XSD).expect("PING_XSD should compile"))
        .clone()
}
