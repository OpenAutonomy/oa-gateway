//! Golden A-GRA XML against the real A-GRA 5.0a schema, via libxml2.
//!
//! The four fixtures are real UCI/A-GRA XML, generated once from the
//! project's former JSON fixtures by the hand-rolled converter this
//! session replaced, then checked to validate before being committed
//! here — see `CHANGELOG.md`. There is no conversion step in this test
//! at all now: libxml2 validates the XML text directly.
//!
//! CI fetches the real XSDs with `scripts/fetch-agra-schema.sh` (pinned to
//! the public `open-arsenal/a-gra/standard` v5.0a tag) and points
//! `AGRA_SCHEMA_DIR` at the result. A local run without that env var falls
//! back to the sibling `open-ma` tree (`third_party/a-gra/Schema`) in the
//! demo checkout; if neither has the files, these tests skip rather than
//! panic.

use std::path::PathBuf;
use std::sync::OnceLock;

use oa_gateway_xsd_libxml::XsdLibxmlValidator;

const ROUTE_PLAN: &str = include_str!("fixtures/MA_RoutePlan.xml");
const MISSION_PLAN: &str = include_str!("fixtures/MA_MissionPlan.xml");
const ACTIVATION: &str = include_str!("fixtures/MA_MissionPlanActivationCommand.xml");
const RX_WRAPPER: &str = include_str!("fixtures/MA_RxDataPayload.xml");

fn agra_schema_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGRA_SCHEMA_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../open-ma/third_party/a-gra/Schema")
}

/// libxml2 wants one entry point, from which it resolves
/// `xs:include`/`xs:import` itself. `A-GRA_MessageDefinitions_v5_0_a.xsd`
/// is that entry point here — it `xs:include`s
/// `A-GRA_SecurityMarkings_v5_0_a.xsd` from the same directory.
fn agra_entry_path() -> PathBuf {
    agra_schema_dir().join("A-GRA_MessageDefinitions_v5_0_a.xsd")
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

async fn assert_schema_valid(name: &str, xml: &str) {
    let Some(validator) = libxml_validator() else {
        eprintln!(
            "skipping {name}: A-GRA XSDs not present at {}",
            agra_schema_dir().display()
        );
        return;
    };
    if let Err(errors) = validator.validate(xml.to_owned()).await {
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
