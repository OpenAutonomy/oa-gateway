//! JSON ↔ XML convert cost on the PositionReport fixture.

use std::collections::BTreeMap;
use std::time::Instant;

use oa_gateway_testing::fixtures::{POSITION_REPORT_JSON, POSITION_REPORT_XML};
use oa_gateway_uci::Message;
use serde_json::json;

use crate::cli::{UciArgs, UciDirection};
use crate::report;

/// Runs the codec scenario.
///
/// # Errors
///
/// Returns a message if a convert fails, nothing is measured, or the
/// JSON file cannot be written.
pub(crate) fn run(args: UciArgs) -> Result<(), String> {
    let schema = oa_gateway_uci::slice::v25();
    let started = Instant::now();
    let mut samples = Vec::with_capacity(args.iterations as usize);

    for _ in 0..args.iterations {
        let t0 = Instant::now();
        match args.direction {
            UciDirection::JsonToXml => {
                let msg = Message::from_json(POSITION_REPORT_JSON, schema)
                    .map_err(|err| format!("from_json: {err}"))?;
                msg.to_xml(schema).map_err(|err| format!("to_xml: {err}"))?;
            }
            UciDirection::XmlToJson => {
                let msg = Message::from_xml(POSITION_REPORT_XML, schema)
                    .map_err(|err| format!("from_xml: {err}"))?;
                msg.to_json(schema)
                    .map_err(|err| format!("to_json: {err}"))?;
            }
        }
        samples.push(t0.elapsed().as_nanos() as u64);
    }

    if samples.is_empty() {
        return Err("uci ran 0 iterations".into());
    }

    let direction = match args.direction {
        UciDirection::JsonToXml => "json-to-xml",
        UciDirection::XmlToJson => "xml-to-json",
    };
    let mut flags = BTreeMap::new();
    flags.insert("iterations".into(), json!(args.iterations));
    flags.insert("direction".into(), json!(direction));

    let payload_bytes = match args.direction {
        UciDirection::JsonToXml => POSITION_REPORT_JSON.len(),
        UciDirection::XmlToJson => POSITION_REPORT_XML.len(),
    } as u64;

    let mut report = report::blank("uci", flags);
    report.sent = args.iterations;
    report.received = args.iterations;
    report.duration_secs = started.elapsed().as_secs_f64();
    let report = report.finish(samples, None, payload_bytes);
    report.emit(args.json.as_deref())
}
