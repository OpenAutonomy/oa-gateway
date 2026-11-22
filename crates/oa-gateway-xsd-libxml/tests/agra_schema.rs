//! Golden OMS JSON against the real A-GRA 5.0a schema, via libxml2.
//!
//! Parity test for the libxml2-backed validator: the same four fixtures
//! `oa-gateway-uci`'s own `agra_schema.rs` uses to prove conversion works
//! must also come back schema-valid here, now that validation itself has
//! moved to libxml2. Unlike that test, this one must produce real XML text
//! first (`Message::to_xml`) since libxml2 validates XML instances, not
//! the JSON-origin instance tree `oa-gateway-uci` converts through.
//!
//! CI fetches the real XSDs with `scripts/fetch-agra-schema.sh` (pinned to
//! the public `open-arsenal/a-gra/standard` v5.0a tag) and points
//! `AGRA_SCHEMA_DIR` at the result. A local run without that env var falls
//! back to the sibling `open-ma` tree (`third_party/a-gra/Schema`) in the
//! demo checkout; if neither has the files, these tests skip rather than
//! panic.

use std::path::PathBuf;
use std::sync::OnceLock;

use oa_gateway_uci::{xsd, Message, Schema};
use oa_gateway_xsd_libxml::XsdLibxmlValidator;

const ROUTE_PLAN: &str = include_str!("../../oa-gateway-uci/tests/fixtures/MA_RoutePlan.json");
const MISSION_PLAN: &str = include_str!("../../oa-gateway-uci/tests/fixtures/MA_MissionPlan.json");
const ACTIVATION: &str =
    include_str!("../../oa-gateway-uci/tests/fixtures/MA_MissionPlanActivationCommand.json");
const RX_WRAPPER: &str = include_str!("../../oa-gateway-uci/tests/fixtures/MA_RxDataPayload.json");

fn agra_schema_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGRA_SCHEMA_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../open-ma/third_party/a-gra/Schema")
}

/// The hand-rolled compiler needs every spanning document named explicitly
/// (it does not follow `xs:include`); libxml2 is the opposite, and wants
/// one entry point from which it resolves `xs:include`/`xs:import` itself.
/// `A-GRA_MessageDefinitions_v5_0_a.xsd` is that entry point here — it
/// `xs:include`s `A-GRA_SecurityMarkings_v5_0_a.xsd` from the same
/// directory.
fn agra_entry_path() -> PathBuf {
    agra_schema_dir().join("A-GRA_MessageDefinitions_v5_0_a.xsd")
}

/// The hand-rolled `Schema` is still what drives JSON -> XML conversion
/// here -- compiled from both files, exactly as `oa-gateway-uci`'s own
/// test does it.
fn conversion_schema() -> Option<&'static Schema> {
    static SCHEMA: OnceLock<Option<Schema>> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            let dir = agra_schema_dir();
            let defs =
                std::fs::read_to_string(dir.join("A-GRA_MessageDefinitions_v5_0_a.xsd")).ok()?;
            let markings =
                std::fs::read_to_string(dir.join("A-GRA_SecurityMarkings_v5_0_a.xsd")).ok()?;
            Some(xsd::compile(&[&defs, &markings]).expect("A-GRA 5.0a should compile"))
        })
        .as_ref()
}

fn libxml_validator() -> Option<&'static XsdLibxmlValidator> {
    static VALIDATOR: OnceLock<Option<XsdLibxmlValidator>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let entry = agra_entry_path();
            if !entry.exists() {
                return None;
            }
            Some(
                XsdLibxmlValidator::compile(&entry).expect("A-GRA 5.0a should compile via libxml2"),
            )
        })
        .as_ref()
}

async fn assert_schema_valid(name: &str, json: &str) {
    let (Some(schema), Some(validator)) = (conversion_schema(), libxml_validator()) else {
        eprintln!(
            "skipping {name}: A-GRA XSDs not present at {}",
            agra_schema_dir().display()
        );
        return;
    };
    let message =
        Message::from_json(json, schema).unwrap_or_else(|e| panic!("{name} should convert: {e}"));
    assert_eq!(message.type_hint(), name);
    let xml = message
        .to_xml(schema)
        .unwrap_or_else(|e| panic!("{name} should convert to xml: {e}"));
    if let Err(errors) = validator.validate(xml).await {
        panic!(
            "{name} departed from A-GRA per libxml2:\n  {}",
            oa_gateway_xsd_libxml::summarize(&errors)
        );
    }
}

#[tokio::test]
async fn golden_route_plan_is_schema_valid() {
    assert_schema_valid("MA_RoutePlan", ROUTE_PLAN).await;
}

#[tokio::test]
async fn golden_mission_plan_is_schema_valid() {
    assert_schema_valid("MA_MissionPlan", MISSION_PLAN).await;
}

#[tokio::test]
async fn golden_activation_command_is_schema_valid() {
    assert_schema_valid("MA_MissionPlanActivationCommand", ACTIVATION).await;
}

#[tokio::test]
async fn golden_rx_wrapper_is_schema_valid() {
    assert_schema_valid("MA_RxDataPayload", RX_WRAPPER).await;
}
