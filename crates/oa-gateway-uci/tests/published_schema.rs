//! Compiles the real UCI schema. Ignored unless a local copy is available.
//!
//! The standard is not vendored. Fetch it once with `scripts/fetch-uci-schema.sh`
//! and this finds it on its own:
//!
//!   cargo test -p oa-gateway-uci -- --ignored
//!
//! To use a copy from somewhere else — a program-specific Message Set, say —
//! point `OAG_UCI_XSD` at the documents, separated the way `PATH` is on your
//! platform:
//!
//!   OAG_UCI_XSD=/path/UCI_MessageDefinitions_v2_5_0.xsd:/path/UCI_SecurityMarkings_v2_5_0.xsd \
//!     cargo test -p oa-gateway-uci -- --ignored

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use oa_gateway_uci::{xsd, Facets, Message, Schema, MAX_DEPTH};

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

/// Validation against the real schema, on the message the conversion test uses.
///
/// It converts cleanly in both directions and it is not a valid instance of the
/// standard: four elements UCI requires are absent. That gap is the whole reason
/// validation is separate from conversion, and it is worth pinning to a real
/// message rather than a fixture built to fail.
fn reports_what_conversion_accepts(schema: &Schema) {
    let src = r#"{
        "SubsystemStatus": {
            "MessageHeader": {
                "Timestamp": "2026-01-22T00:00:00Z",
                "SchemaVersion": "002.5.0",
                "Mode": "SIMULATION"
            },
            "MessageData": { "SubsystemState": "OPERATE" }
        }
    }"#;
    let message = Message::from_json(src, schema).expect("it converts; that is the point");
    let reported: Vec<String> = message
        .violations(schema)
        .iter()
        .map(ToString::to_string)
        .collect();

    for expected in [
        "SubsystemStatus: 'SecurityInformation' is required and absent",
        "SubsystemStatus.MessageHeader: 'SystemID' is required and absent",
        "SubsystemStatus.MessageData: 'SubsystemID' is required and absent",
    ] {
        assert!(
            reported.iter().any(|v| v == expected),
            "expected to be told {expected:?}, got {reported:?}"
        );
    }

    // Real values satisfying real facets: SIMULATION is a MessageModeEnum member
    // and 002.5.0 fits the schema-version constraints, so neither is reported.
    // A facet check that fired on those would be worse than none.
    assert!(
        !reported.iter().any(|v| v.contains("SIMULATION")),
        "a valid enumeration member must not be reported: {reported:?}"
    );

    // The same message as XML has to report the same list: validation reads the
    // schema, not the serialization it arrived in.
    let xml = message.to_xml(schema).expect("emits XML");
    let from_xml = Message::from_xml(&xml, schema).expect("reads its own XML");
    let reported_from_xml: Vec<String> = from_xml
        .violations(schema)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(reported, reported_from_xml);
}

/// Facet checks against the values the standard actually declares.
///
/// The UUID case is not hypothetical. UCI declares
/// `UniversallyUniqueIdentifierType` as exactly 36 characters in the hyphenated
/// RFC 4122 form, while A-GRA carries the same identifier as `hexBinary` — and
/// the live traffic in `repos/open-ma` sends the unhyphenated 32-character form.
/// A gateway between the two is exactly where that shows up, so it is worth a
/// test that says so against the published schema rather than a fixture.
fn reports_values_the_standard_does_not_allow(schema: &Schema) {
    let with_mode = |mode: &str, uuid: &str| {
        format!(
            r#"{{"SubsystemStatus":{{
                 "MessageHeader":{{
                   "Timestamp":"2026-01-22T00:00:00Z",
                   "SchemaVersion":"002.5.0",
                   "Mode":"{mode}",
                   "SystemID":{{"UUID":"{uuid}"}}
                 }},
                 "MessageData":{{"SubsystemState":"OPERATE"}}
               }}}}"#
        )
    };
    let hyphenated = "7ea053ea-dcc5-45ba-ac26-d5bc909417dc";
    let unhyphenated = "7ea053eadcc545baac26d5bc909417dc";

    let report = |json: &str| -> Vec<String> {
        Message::from_json(json, schema)
            .expect("all of these convert")
            .violations(schema)
            .iter()
            .map(ToString::to_string)
            .collect()
    };

    // A mode outside MessageModeEnum is named, and so are the values it could
    // have been.
    let reported = report(&with_mode("OFFLINE", hyphenated));
    let mode = reported
        .iter()
        .find(|v| v.contains("MessageHeader.Mode"))
        .unwrap_or_else(|| panic!("expected the mode to be reported: {reported:?}"));
    assert!(mode.contains("'OFFLINE' is not one of"), "{mode}");
    assert!(mode.contains("SIMULATION"), "{mode}");

    // The A-GRA form of a UUID is not the UCI form, and both the length and the
    // RFC 4122 pattern say so.
    let reported = report(&with_mode("SIMULATION", unhyphenated));
    let uuid: Vec<&String> = reported
        .iter()
        .filter(|v| v.contains("SystemID.UUID"))
        .collect();
    assert!(
        uuid.iter()
            .any(|v| v.contains("32 characters") && v.contains("exactly 36")),
        "expected the length to be reported: {reported:?}"
    );
    assert!(
        uuid.iter()
            .any(|v| v.contains("does not match the pattern")),
        "expected the pattern to be reported: {reported:?}"
    );

    // The UCI form passes, so the check is about the form and not about UUIDs.
    let reported = report(&with_mode("SIMULATION", hyphenated));
    assert!(
        !reported.iter().any(|v| v.contains("SystemID.UUID")),
        "a well-formed UUID must not be reported: {reported:?}"
    );
}

/// Every pattern in the catalog translates into something this build can check.
///
/// XSD's regex language is wider than Rust's in two corners — character-class
/// subtraction and the `\i` and `\c` name shorthands — and neither appears in
/// the 143 patterns the standard declares. This is the test that would notice if
/// a later revision added one, since the alternative is a constraint that reads
/// as satisfied because nothing could evaluate it.
fn every_pattern_is_checkable(schema: &Schema) {
    assert_eq!(
        schema.unchecked_patterns(),
        Vec::new(),
        "these patterns would go unchecked"
    );

    let declared: usize = schema
        .simple_types
        .values()
        .map(|simple| simple.facets.patterns.len())
        .sum();
    assert!(
        declared > 100,
        "expected the catalog's patterns to be read; found {declared}"
    );
}

/// The deepest message the catalog can express, and how deep it is.
///
/// `MAX_DEPTH` has to sit above anything the standard declares, or the limit
/// would refuse real traffic instead of hostile traffic. Measured rather than
/// assumed, since nothing else would notice if a program-specific Message Set
/// nested further than expected.
///
/// A type that reappears on the current path stops the walk: the type graph has
/// cycles by reference — a type can contain a type that contains it again — while
/// any one instance is finite. Depths recorded while a cycle was cut can only be
/// under-estimates, which is why the assertion below leaves room rather than
/// treating the number as exact.
fn deepest_message(schema: &Schema) -> (String, usize) {
    fn depth_of(
        schema: &Schema,
        type_name: &str,
        on_path: &mut Vec<String>,
        memo: &mut std::collections::HashMap<String, usize>,
    ) -> usize {
        if on_path.iter().any(|t| t == type_name) {
            return 0;
        }
        if let Some(&known) = memo.get(type_name) {
            return known;
        }
        if !schema.is_complex(type_name) {
            return 1;
        }
        on_path.push(type_name.to_owned());
        let children = schema.flatten(type_name).unwrap_or_default();
        let deepest = children
            .iter()
            .map(|e| depth_of(schema, &e.type_name, on_path, memo))
            .max()
            .unwrap_or(0);
        on_path.pop();
        memo.insert(type_name.to_owned(), 1 + deepest);
        1 + deepest
    }

    let mut memo = std::collections::HashMap::new();
    schema
        .global_elements
        .iter()
        .map(|(name, global)| {
            let depth = depth_of(schema, &global.type_name, &mut Vec::new(), &mut memo);
            (name.clone(), depth)
        })
        .max_by_key(|(_, depth)| *depth)
        .expect("the catalog is not empty")
}

/// Where `scripts/fetch-uci-schema.sh` leaves the schema, relative to this crate.
const FETCHED: [&str; 2] = [
    "../../schema/uci/UCI_MessageDefinitions_v2_5_0.xsd",
    "../../schema/uci/UCI_SecurityMarkings_v2_5_0.xsd",
];

fn schema_documents() -> Vec<PathBuf> {
    if let Some(raw) = env::var_os("OAG_UCI_XSD") {
        let paths: Vec<PathBuf> = env::split_paths(&raw).collect();
        assert!(!paths.is_empty(), "OAG_UCI_XSD is set but lists no paths");
        return paths;
    }

    let fetched: Vec<PathBuf> = FETCHED.iter().map(PathBuf::from).collect();
    assert!(
        fetched.iter().all(|p| p.exists()),
        "no UCI schema found. Run scripts/fetch-uci-schema.sh, or set OAG_UCI_XSD \
         to the documents yourself, separated like PATH (UCI_MessageDefinitions \
         and UCI_SecurityMarkings are both required)"
    );
    fetched
}

#[test]
#[ignore = "requires a local copy of the UCI schema (scripts/fetch-uci-schema.sh)"]
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

    reports_what_conversion_accepts(&schema);
    reports_values_the_standard_does_not_allow(&schema);
    every_pattern_is_checkable(&schema);

    let (deepest, depth) = deepest_message(&schema);
    assert!(
        depth * 2 < MAX_DEPTH,
        "the conversion depth limit ({MAX_DEPTH}) leaves too little room above this \
         schema: '{deepest}' already nests {depth} deep, and the measurement cuts \
         cycles so the real figure can only be larger"
    );

    eprintln!("deepest message: {deepest} at {depth} levels (limit {MAX_DEPTH})");
    eprintln!(
        "compiled {} documents ({:.1} MiB) in {:?}: {} messages, {} complexTypes, \
         {} simpleTypes carrying {} enumerated values and {} patterns",
        documents.len(),
        bytes as f64 / (1024.0 * 1024.0),
        elapsed,
        schema.global_elements.len(),
        schema.complex_types.len(),
        schema.simple_types.len(),
        facets(&schema, |f| f.enumeration.len()),
        facets(&schema, |f| f.patterns.len()),
    );
}

/// Sum something over every simple type's facets.
fn facets(schema: &Schema, of: impl Fn(&Facets) -> usize) -> usize {
    schema
        .simple_types
        .values()
        .map(|simple| of(&simple.facets))
        .sum()
}
