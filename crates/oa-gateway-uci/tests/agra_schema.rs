//! Golden OMS JSON against the real A-GRA 5.0a catalog — conversion only.
//!
//! These instances are the C2→MA contract: a RoutePlan, a MissionPlan that
//! references it, an activation of that mission, and the Rx wrapper that
//! carries the first of those as hex. This crate has no schema *validator*
//! any more (that moved to `oa-gateway-xsd-libxml`, which has its own copy
//! of this same test asserting these same fixtures are actually A-GRA-valid
//! via libxml2) — what's proven here is narrower: that `xsd::compile`
//! compiles the real catalog and `Message::from_json`/`to_xml` convert
//! these fixtures without error.
//!
//! CI fetches the real XSDs with `scripts/fetch-agra-schema.sh` (pinned to
//! the public `open-arsenal/a-gra/standard` v5.0a tag — not `main`, which has
//! since moved to 6.0a) and points `AGRA_SCHEMA_DIR` at the result. A local
//! run without that env var falls back to the sibling `open-ma` tree
//! (`third_party/a-gra/Schema`) in the demo checkout; if neither has the
//! files, the tests return rather than panic.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use oa_gateway_uci::{xsd, Message, Schema};

const ROUTE_PLAN: &str = include_str!("fixtures/MA_RoutePlan.json");
const MISSION_PLAN: &str = include_str!("fixtures/MA_MissionPlan.json");
const ACTIVATION: &str = include_str!("fixtures/MA_MissionPlanActivationCommand.json");
const RX_WRAPPER: &str = include_str!("fixtures/MA_RxDataPayload.json");

fn agra_schema_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGRA_SCHEMA_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../open-ma/third_party/a-gra/Schema")
}

fn agra_schema() -> Option<&'static Schema> {
    static SCHEMA: OnceLock<Option<Schema>> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            let dir = agra_schema_dir();
            let defs = fs::read_to_string(dir.join("A-GRA_MessageDefinitions_v5_0_a.xsd")).ok()?;
            let markings =
                fs::read_to_string(dir.join("A-GRA_SecurityMarkings_v5_0_a.xsd")).ok()?;
            Some(xsd::compile(&[&defs, &markings]).expect("A-GRA 5.0a should compile"))
        })
        .as_ref()
}

fn assert_converts(name: &str, json: &str) {
    let Some(schema) = agra_schema() else {
        eprintln!(
            "skipping {name}: A-GRA XSDs not present at {}",
            agra_schema_dir().display()
        );
        return;
    };
    let message =
        Message::from_json(json, schema).unwrap_or_else(|e| panic!("{name} should convert: {e}"));
    assert_eq!(message.type_hint(), name);
    message
        .to_xml(schema)
        .unwrap_or_else(|e| panic!("{name} should convert to xml: {e}"));
}

#[test]
fn golden_route_plan_converts() {
    assert_converts("MA_RoutePlan", ROUTE_PLAN);
}

#[test]
fn golden_mission_plan_converts() {
    assert_converts("MA_MissionPlan", MISSION_PLAN);
}

#[test]
fn golden_activation_command_converts() {
    assert_converts("MA_MissionPlanActivationCommand", ACTIVATION);
}

#[test]
fn golden_rx_wrapper_converts() {
    assert_converts("MA_RxDataPayload", RX_WRAPPER);
}

/// The wrapper's hex payload is the RoutePlan fixture, compacted — so peeling
/// it yields the same inner the C2 path publishes first.
#[test]
fn rx_wrapper_carries_the_golden_route_plan() {
    if agra_schema().is_none() {
        eprintln!(
            "skipping rx_wrapper_carries_the_golden_route_plan: A-GRA XSDs not present at {}",
            agra_schema_dir().display()
        );
        return;
    }
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
    assert_converts("MA_RoutePlan", inner_text);

    let expected = serde_json::from_str::<serde_json::Value>(ROUTE_PLAN).unwrap();
    let got = serde_json::from_str::<serde_json::Value>(inner_text).unwrap();
    assert_eq!(got, expected);
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
