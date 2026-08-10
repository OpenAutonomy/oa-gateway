//! Sample payloads. Paths stay valid when crates move; do not `include_str!`
//! from the workspace root.

/// Minimal UCI XML `PositionReport` used on the ASB / STOMP path.
pub const POSITION_REPORT_XML: &str = include_str!("../fixtures/PositionReport.xml");

/// Same document as bytes (loopback / STOMP SEND).
pub const POSITION_REPORT_XML_BYTES: &[u8] = POSITION_REPORT_XML.as_bytes();

/// OMS JSON `PositionReport` matching the sleet v2.5 fixture shape.
pub const POSITION_REPORT_JSON: &str = include_str!("../fixtures/PositionReport.json");

/// Filesystem path to the XML fixture (scripts / non-Rust tools).
#[must_use]
pub fn position_report_xml_path() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/PositionReport.xml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_are_nonempty() {
        assert!(POSITION_REPORT_XML.contains("<PositionReport"));
        assert!(POSITION_REPORT_JSON.contains("\"PositionReport\""));
        assert_eq!(POSITION_REPORT_XML_BYTES, POSITION_REPORT_XML.as_bytes());
    }
}
