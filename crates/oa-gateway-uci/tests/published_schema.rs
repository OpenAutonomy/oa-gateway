//! Compiles the real UCI schema. Ignored unless a local copy is available.
//!
//! The standard is not vendored, so point `OAG_UCI_XSD` at the documents that
//! make up the schema, separated the way `PATH` is on your platform:
//!
//!   OAG_UCI_XSD=/path/UCI_MessageDefinitions_v2_5_0.xsd:/path/UCI_SecurityMarkings_v2_5_0.xsd \
//!     cargo test -p oa-gateway-uci -- --ignored

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use oa_gateway_uci::{xsd, Message, Schema};

/// Convert a real message that the hand-written fixture never covered.
///
/// `SubsystemStatus` is a useful subject because its leaves are named simple
/// types rather than `xs:` primitives — `Timestamp` is a `DateTimeType` and
/// `Mode` a `MessageModeEnum`. Before the schema carried a simple-type map those
/// would have been mistaken for complex types and failed outright.
fn round_trips_a_message_the_fixture_never_had(schema: &Schema) {
    let src = r#"{
        "SubsystemStatus": {
            "MessageHeader": {
                "Timestamp": "2026-01-22T00:00:00Z",
                "SchemaVersion": "002.5.0",
                "Mode": "SIMULATION"
            },
            "MessageData": {
                "SubsystemState": "OPERATE"
            }
        }
    }"#;

    let message = Message::from_json(src, schema).expect("SubsystemStatus should convert");
    assert_eq!(message.type_hint(), "SubsystemStatus");

    let xml = message.to_xml(schema).expect("should emit XML");
    assert!(xml.contains("<SubsystemStatus"), "{xml}");
    assert!(
        xml.contains("<Timestamp>2026-01-22T00:00:00Z</Timestamp>"),
        "{xml}"
    );
    assert!(
        xml.contains("<SubsystemState>OPERATE</SubsystemState>"),
        "{xml}"
    );

    let back = Message::from_xml(&xml, schema).expect("should read its own XML");
    let value: serde_json::Value =
        serde_json::from_str(&back.to_json(schema).expect("should emit JSON")).unwrap();

    // A dateTime and an enumeration are both strings on the way back, not the
    // objects a missing simple-type map would have produced.
    assert_eq!(
        value
            .pointer("/SubsystemStatus/MessageHeader/Timestamp")
            .and_then(serde_json::Value::as_str),
        Some("2026-01-22T00:00:00Z")
    );
    assert_eq!(
        value
            .pointer("/SubsystemStatus/MessageData/SubsystemState")
            .and_then(serde_json::Value::as_str),
        Some("OPERATE")
    );
}

fn schema_documents() -> Vec<PathBuf> {
    let raw = env::var_os("OAG_UCI_XSD").unwrap_or_else(|| {
        panic!(
            "set OAG_UCI_XSD to the UCI schema documents, separated like PATH \
             (UCI_MessageDefinitions and UCI_SecurityMarkings are both required)"
        )
    });
    let paths: Vec<PathBuf> = env::split_paths(&raw).collect();
    assert!(!paths.is_empty(), "OAG_UCI_XSD is set but lists no paths");
    paths
}

#[test]
#[ignore = "requires a local copy of the UCI schema (set OAG_UCI_XSD)"]
fn the_published_schema_compiles_and_every_type_resolves() {
    let paths = schema_documents();
    let texts: Vec<String> = paths
        .iter()
        .map(|p| {
            fs::read_to_string(p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        })
        .collect();
    let documents: Vec<&str> = texts.iter().map(String::as_str).collect();
    let bytes: usize = texts.iter().map(String::len).sum();

    let started = Instant::now();
    let schema = xsd::compile(&documents).expect("the published schema should compile");
    let elapsed = started.elapsed();

    // Flattening exercises every extension chain, so this proves each base
    // resolves and no chain is cyclic.
    for name in schema.complex_types.keys() {
        schema
            .flatten(name)
            .unwrap_or_else(|e| panic!("complexType '{name}' does not flatten: {e}"));
    }

    // Every message must land on a type the schema defines, or a payload of that
    // type could never be converted.
    for (name, global) in &schema.global_elements {
        let target = &global.type_name;
        assert!(
            schema.is_complex(target) || schema.is_simple(target),
            "message '{name}' refers to undefined type '{target}'"
        );
    }

    // Every named simple type must bottom out at an xs: primitive, since leaf
    // coercion matches on primitive names.
    for name in schema.simple_types.keys() {
        let primitive = schema.primitive(name);
        assert!(
            primitive.starts_with("xs:"),
            "simpleType '{name}' resolves to '{primitive}', which is not a primitive"
        );
    }

    // Guard against pointing the test at a subset template and quietly proving
    // nothing: the real catalog is far larger than the hand-written slice.
    assert!(
        schema.global_elements.len() > 500,
        "expected the full message catalog, found {} messages",
        schema.global_elements.len()
    );
    assert!(
        schema.complex_types.len() > 4000,
        "expected thousands of complexTypes, found {}",
        schema.complex_types.len()
    );
    assert!(
        schema.simple_types.len() > 900,
        "expected hundreds of simpleTypes, found {}",
        schema.simple_types.len()
    );

    round_trips_a_message_the_fixture_never_had(&schema);

    eprintln!(
        "compiled {} documents ({:.1} MiB) in {:?}: {} messages, {} complexTypes, {} simpleTypes",
        documents.len(),
        bytes as f64 / (1024.0 * 1024.0),
        elapsed,
        schema.global_elements.len(),
        schema.complex_types.len(),
        schema.simple_types.len(),
    );
}
