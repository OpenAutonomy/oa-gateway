//! Golden OMS JSON against the A-GRA 5.0a schema compose mounts.
//!
//! These instances are the C2→MA contract: a RoutePlan, a MissionPlan that
//! references it, an activation of that mission, and the Rx wrapper that
//! carries the first of those as hex. `Message::from_json` plus an empty
//! violation list is the bar — conversion without a schema check is not.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use oa_gateway_uci::{xsd, Message, Schema};

const ROUTE_PLAN: &str = include_str!("fixtures/MA_RoutePlan.json");
const MISSION_PLAN: &str = include_str!("fixtures/MA_MissionPlan.json");
const ACTIVATION: &str = include_str!("fixtures/MA_MissionPlanActivationCommand.json");
const RX_WRAPPER: &str = include_str!("fixtures/MA_RxDataPayload.json");

fn agra_schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../open-ma/third_party/a-gra/Schema");
        let defs = fs::read_to_string(dir.join("A-GRA_MessageDefinitions_v5_0_a.xsd"))
            .unwrap_or_else(|e| panic!("cannot read A-GRA message definitions: {e}"));
        let markings = fs::read_to_string(dir.join("A-GRA_SecurityMarkings_v5_0_a.xsd"))
            .unwrap_or_else(|e| panic!("cannot read A-GRA security markings: {e}"));
        xsd::compile(&[&defs, &markings]).expect("A-GRA 5.0a should compile")
    })
}

fn assert_schema_valid(name: &str, json: &str) {
    let schema = agra_schema();
    let message =
        Message::from_json(json, schema).unwrap_or_else(|e| panic!("{name} should convert: {e}"));
    assert_eq!(message.type_hint(), name);
    let reported: Vec<String> = message
        .violations(schema)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        reported.is_empty(),
        "{name} departed from A-GRA:\n  {}",
        reported.join("\n  ")
    );
}

#[test]
fn golden_route_plan_is_schema_valid() {
    assert_schema_valid("MA_RoutePlan", ROUTE_PLAN);
}

#[test]
fn golden_mission_plan_is_schema_valid() {
    assert_schema_valid("MA_MissionPlan", MISSION_PLAN);
}

#[test]
fn golden_activation_command_is_schema_valid() {
    assert_schema_valid("MA_MissionPlanActivationCommand", ACTIVATION);
}

#[test]
fn golden_rx_wrapper_is_schema_valid() {
    assert_schema_valid("MA_RxDataPayload", RX_WRAPPER);
}

/// The wrapper's hex payload is the RoutePlan fixture, compacted — so peeling
/// it yields the same schema-valid inner the C2 path publishes first.
#[test]
fn rx_wrapper_carries_the_golden_route_plan() {
    let schema = agra_schema();
    let wrapper: serde_json::Value =
        serde_json::from_str(RX_WRAPPER).expect("the wrapper fixture is JSON");
    let hex = wrapper
        .pointer("/MA_RxDataPayload/MessageData/EncodedPayload")
        .and_then(serde_json::Value::as_str)
        .expect("EncodedPayload is present");
    assert_eq!(
        wrapper.pointer("/MA_RxDataPayload/MessageData/MessageType"),
        Some(&serde_json::Value::String("MA_ROUTE_PLAN".into()))
    );

    let inner_bytes = decode_hex(hex);
    let inner_text = std::str::from_utf8(&inner_bytes).expect("inner is UTF-8 JSON");
    assert_schema_valid("MA_RoutePlan", inner_text);

    let expected = serde_json::from_str::<serde_json::Value>(ROUTE_PLAN).unwrap();
    let got = serde_json::from_str::<serde_json::Value>(inner_text).unwrap();
    assert_eq!(got, expected);

    // A 32-character A-GRA UUID must not be reported as the wrong length.
    let message = Message::from_json(inner_text, schema).unwrap();
    assert!(
        !message
            .violations(schema)
            .iter()
            .any(|v| v.to_string().contains("characters")),
        "hexBinary UUIDs must be counted in octets"
    );
}

fn decode_hex(s: &str) -> Vec<u8> {
    assert!(
        s.len() % 2 == 0,
        "EncodedPayload is an even number of digits"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit"))
        .collect()
}
