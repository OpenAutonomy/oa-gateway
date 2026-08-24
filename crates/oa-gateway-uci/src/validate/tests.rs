use super::*;
use crate::instance::Message;
use crate::schema::{choice, el, el_many, el_opt, sequence, Element, Facets, MaxOccurs, Schema};
use crate::xsd;

/// One required child, one optional, one repeating with a ceiling of two.
fn schema() -> Schema {
    let mut s = Schema::new();
    s.complex("LegType", vec![el("Distance", "xs:double")])
        .complex(
            "TripType",
            vec![
                el("Name", "xs:string"),
                el_opt("Note", "xs:string"),
                Element {
                    name: "Leg".into(),
                    type_name: "LegType".into(),
                    min_occurs: 1,
                    max_occurs: MaxOccurs::Bounded(2),
                },
            ],
        )
        .element("Trip", "TripType");
    s
}

fn violations(json: &str, schema: &Schema) -> Vec<String> {
    let message = Message::from_json(json, schema).expect("the fixtures all convert");
    validate(&message, schema)
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn a_message_that_agrees_with_the_schema_reports_nothing() {
    let schema = schema();
    let json = r#"{"Trip":{"Name":"a","Leg":[{"Distance":1.0}]}}"#;
    assert_eq!(violations(json, &schema), Vec::<String>::new());
}

#[test]
fn a_required_element_that_is_absent_is_reported() {
    let schema = schema();
    let json = r#"{"Trip":{"Leg":[{"Distance":1.0}]}}"#;
    assert_eq!(
        violations(json, &schema),
        vec!["Trip: 'Name' is required and absent"]
    );
}

/// The case conversion is deliberately forgiving about: an element the type
/// does not declare is carried as a string rather than refused.
#[test]
fn an_element_the_type_does_not_declare_is_reported() {
    let schema = schema();
    let json = r#"{"Trip":{"Name":"a","Nmae":"typo","Leg":[{"Distance":1.0}]}}"#;
    assert_eq!(
        violations(json, &schema),
        vec!["Trip: 'Nmae' is not declared by this type"]
    );
}

#[test]
fn an_occurrence_range_is_checked_at_both_ends() {
    let schema = schema();
    let over =
        r#"{"Trip":{"Name":"a","Leg":[{"Distance":1.0},{"Distance":2.0},{"Distance":3.0}]}}"#;
    assert_eq!(
        violations(over, &schema),
        vec!["Trip: 'Leg' appears 3 times, maxOccurs is 2"]
    );

    let none = r#"{"Trip":{"Name":"a","Leg":[]}}"#;
    assert_eq!(
        violations(none, &schema),
        vec!["Trip: 'Leg' is required and absent"]
    );
}

#[test]
fn violations_are_reported_from_where_they_occur() {
    let schema = schema();
    // The nested type is missing its own required child.
    let json = r#"{"Trip":{"Name":"a","Leg":[{"Distance":1.0},{}]}}"#;
    assert_eq!(
        violations(json, &schema),
        vec!["Trip.Leg[1]: 'Distance' is required and absent"]
    );
}

#[test]
fn an_alternation_takes_exactly_one_branch() {
    let mut s = Schema::new();
    s.complex_groups(
        "EitherType",
        vec![
            sequence(vec![el("Tag", "xs:string")]),
            choice(vec![el("ByName", "xs:string"), el("ById", "xs:int")]),
        ],
    )
    .element("Either", "EitherType");

    let one = r#"{"Either":{"Tag":"t","ByName":"x"}}"#;
    assert_eq!(violations(one, &s), Vec::<String>::new());

    let neither = r#"{"Either":{"Tag":"t"}}"#;
    assert_eq!(
        violations(neither, &s),
        vec!["Either: none of 'ByName', 'ById' is present, and one of them must be"]
    );

    let both = r#"{"Either":{"Tag":"t","ByName":"x","ById":1}}"#;
    assert_eq!(
        violations(both, &s),
        vec!["Either: 'ByName', 'ById' are all present, and they are alternatives"]
    );
}

/// A choice member is not required just because its own minOccurs defaults
/// to 1 — reading it that way would demand every branch at once.
#[test]
fn choice_members_are_not_each_required() {
    let schema = xsd::compile(&[r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="Either" type="EitherType"/>
          <xs:complexType name="EitherType">
            <xs:choice>
              <xs:element name="ByName" type="xs:string"/>
              <xs:element name="ById" type="xs:int"/>
            </xs:choice>
          </xs:complexType>
        </xs:schema>
    "#])
    .expect("compiles");

    assert_eq!(
        violations(r#"{"Either":{"ByName":"x"}}"#, &schema),
        Vec::<String>::new()
    );
}

/// An alternation inside an extension is still an alternation. Only one type
/// in the published catalog is shaped this way, which is exactly why it would
/// go unnoticed if the schema model flattened it into siblings.
#[test]
fn an_alternation_survives_an_extension() {
    let schema = xsd::compile(&[r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="Derived" type="DerivedType"/>
          <xs:complexType name="BaseType">
            <xs:sequence>
              <xs:element name="Tag" type="xs:string"/>
            </xs:sequence>
          </xs:complexType>
          <xs:complexType name="DerivedType">
            <xs:complexContent>
              <xs:extension base="BaseType">
                <xs:choice>
                  <xs:element name="ByName" type="xs:string"/>
                  <xs:element name="ById" type="xs:int"/>
                </xs:choice>
              </xs:extension>
            </xs:complexContent>
          </xs:complexType>
        </xs:schema>
    "#])
    .expect("compiles");

    assert_eq!(
        violations(r#"{"Derived":{"Tag":"t","ById":1}}"#, &schema),
        Vec::<String>::new()
    );
    assert_eq!(
        violations(r#"{"Derived":{"Tag":"t","ByName":"x","ById":1}}"#, &schema),
        vec!["Derived: 'ByName', 'ById' are all present, and they are alternatives"]
    );
}

#[test]
fn an_abstract_type_has_to_be_made_concrete() {
    let mut s = Schema::new();
    s.complex_abstract("ShapeType", vec![el("Sides", "xs:int")])
        .extend("SquareType", "ShapeType", vec![])
        .element("Shape", "ShapeType");

    assert_eq!(
        violations(r#"{"Shape":{"Sides":4}}"#, &s),
        vec!["Shape: 'ShapeType' is abstract, so a concrete type has to be named"]
    );

    // Naming one settles it.
    assert_eq!(
        violations(r#"{"Shape":{"$type":"SquareType","Sides":4}}"#, &s),
        Vec::<String>::new()
    );
}

/// A state enumeration, a fixed-length code, and a bounded percentage —
/// the three facet families the published catalog actually uses.
fn faceted() -> Schema {
    let mut s = Schema::new();
    s.simple_with(
        "StateType",
        "xs:string",
        Facets {
            enumeration: vec!["OPERATE".into(), "FAULT".into(), "OFF".into()],
            ..Facets::default()
        },
    )
    .simple_with(
        "CodeType",
        "xs:string",
        Facets {
            length: Some(4),
            ..Facets::default()
        },
    )
    .simple_with(
        "PercentType",
        "xs:double",
        Facets {
            min_inclusive: Some(0.0),
            max_inclusive: Some(100.0),
            ..Facets::default()
        },
    )
    .complex(
        "ReadingType",
        vec![
            el("State", "StateType"),
            el_opt("Code", "CodeType"),
            el_opt("Level", "PercentType"),
        ],
    )
    .element("Reading", "ReadingType");
    s
}

#[test]
fn values_within_their_facets_report_nothing() {
    let schema = faceted();
    let json = r#"{"Reading":{"State":"FAULT","Code":"AB12","Level":99.5}}"#;
    assert_eq!(violations(json, &schema), Vec::<String>::new());
}

#[test]
fn a_value_outside_its_enumeration_is_reported() {
    let schema = faceted();
    let json = r#"{"Reading":{"State":"ONLINE"}}"#;
    assert_eq!(
        violations(json, &schema),
        vec![
            "Reading.State: 'ONLINE' is not one of the 3 values this type allows: \
             'OPERATE', 'FAULT', 'OFF'"
        ]
    );
}

#[test]
fn a_value_of_the_wrong_length_is_reported() {
    let schema = faceted();
    assert_eq!(
        violations(r#"{"Reading":{"State":"OFF","Code":"AB1"}}"#, &schema),
        vec!["Reading.Code: 'AB1' is 3 characters, and has to be exactly 4"]
    );
}

/// A-GRA's UUID is `xs:hexBinary` with `length="16"` — sixteen octets, which
/// is thirty-two hex characters. Counting characters instead would reject
/// every well-formed identifier the schema was written to accept.
#[test]
fn hex_binary_length_is_counted_in_octets() {
    let mut s = Schema::new();
    s.simple_with(
        "UuidType",
        "xs:hexBinary",
        Facets {
            length: Some(16),
            ..Facets::default()
        },
    )
    .complex("HoldsType", vec![el("UUID", "UuidType")])
    .element("Holds", "HoldsType");

    let thirty_two = "7ea053eadcc545baac26d5bc909417dc";
    assert_eq!(
        violations(&format!(r#"{{"Holds":{{"UUID":"{thirty_two}"}}}}"#), &s),
        Vec::<String>::new()
    );

    let sixteen = "7ea053eadcc545ba";
    assert_eq!(
        violations(&format!(r#"{{"Holds":{{"UUID":"{sixteen}"}}}}"#), &s),
        vec!["Holds.UUID: '7ea053eadcc545ba' is 8 octets, and has to be exactly 16"]
    );
}

#[test]
fn a_number_outside_its_bounds_is_reported() {
    let schema = faceted();
    assert_eq!(
        violations(r#"{"Reading":{"State":"OFF","Level":100.5}}"#, &schema),
        vec!["Reading.Level: 100.5 is out of range: it has to be at most 100"]
    );
    assert_eq!(
        violations(r#"{"Reading":{"State":"OFF","Level":-1}}"#, &schema),
        vec!["Reading.Level: -1 is out of range: it has to be at least 0"]
    );
    // The bounds themselves are allowed: they are inclusive.
    assert_eq!(
        violations(r#"{"Reading":{"State":"OFF","Level":100}}"#, &schema),
        Vec::<String>::new()
    );
}

/// A chain spreads facets over several links, and every link still applies.
#[test]
fn facets_hold_all_the_way_down_a_restriction_chain() {
    let schema = xsd::compile(&[r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="Label" type="LabelType"/>
          <xs:complexType name="LabelType">
            <xs:sequence>
              <xs:element name="Text" type="ShortTextType"/>
            </xs:sequence>
          </xs:complexType>
          <xs:simpleType name="TextType">
            <xs:restriction base="xs:string">
              <xs:minLength value="2"/>
              <xs:maxLength value="10"/>
            </xs:restriction>
          </xs:simpleType>
          <xs:simpleType name="ShortTextType">
            <xs:restriction base="TextType">
              <xs:maxLength value="4"/>
            </xs:restriction>
          </xs:simpleType>
        </xs:schema>
    "#])
    .expect("compiles");

    // The derived maxLength is the tighter one and wins.
    assert_eq!(
        violations(r#"{"Label":{"Text":"abcdef"}}"#, &schema),
        vec!["Label.Text: 'abcdef' is 6 characters, and has to be at most 4"]
    );
    // The base's minLength is inherited rather than dropped.
    assert_eq!(
        violations(r#"{"Label":{"Text":"a"}}"#, &schema),
        vec!["Label.Text: 'a' is 1 characters, and has to be at least 2"]
    );
    assert_eq!(
        violations(r#"{"Label":{"Text":"abc"}}"#, &schema),
        Vec::<String>::new()
    );
}

#[test]
fn a_long_enumeration_is_sampled_rather_than_recited() {
    let mut s = Schema::new();
    s.simple_with(
        "ManyType",
        "xs:string",
        Facets {
            enumeration: (0..40).map(|i| format!("V{i}")).collect(),
            ..Facets::default()
        },
    )
    .complex("HoldsType", vec![el("Value", "ManyType")])
    .element("Holds", "HoldsType");

    let reported = violations(r#"{"Holds":{"Value":"nope"}}"#, &s);
    assert_eq!(reported.len(), 1);
    assert!(
        reported[0].contains("not one of the 40 values"),
        "{reported:?}"
    );
    assert!(reported[0].ends_with('…'), "{reported:?}");
}

/// One element named Text, typed by the simple type this XSD defines.
fn patterned(simple: &str) -> Schema {
    xsd::compile(&[&format!(
        r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="Label" type="LabelType"/>
          <xs:complexType name="LabelType">
            <xs:sequence><xs:element name="Text" type="TextType"/></xs:sequence>
          </xs:complexType>
          {simple}
        </xs:schema>"#
    )])
    .expect("compiles")
}

fn label(text: &str) -> String {
    format!(r#"{{"Label":{{"Text":"{text}"}}}}"#)
}

/// An XSD pattern constrains the whole value, not some part of it.
#[test]
fn a_pattern_has_to_match_the_value_entire() {
    let schema = patterned(
        r#"<xs:simpleType name="TextType">
             <xs:restriction base="xs:string">
               <xs:pattern value="[0-9]{3}"/>
             </xs:restriction>
           </xs:simpleType>"#,
    );
    assert_eq!(violations(&label("123"), &schema), Vec::<String>::new());
    assert_eq!(
        violations(&label("1234"), &schema),
        vec!["Label.Text: '1234' does not match the pattern '[0-9]{3}'"]
    );
    assert_eq!(violations(&label("x123"), &schema).len(), 1);
}

/// XSD's regex grammar has no anchors, so these are ordinary characters.
#[test]
fn a_dollar_sign_in_a_pattern_is_a_character_not_an_anchor() {
    let schema = patterned(
        r#"<xs:simpleType name="TextType">
             <xs:restriction base="xs:string">
               <xs:pattern value="US$[0-9]+"/>
             </xs:restriction>
           </xs:simpleType>"#,
    );
    assert_eq!(violations(&label("US$50"), &schema), Vec::<String>::new());
    assert_eq!(violations(&label("US50"), &schema).len(), 1);
}

/// Several patterns in one restriction are alternatives. Six types in the
/// published catalog rely on this, one of them with eight alternatives.
#[test]
fn patterns_in_one_restriction_are_alternatives() {
    let schema = patterned(
        r#"<xs:simpleType name="TextType">
             <xs:restriction base="xs:string">
               <xs:pattern value="[A-Z]{2}"/>
               <xs:pattern value="[0-9]{4}"/>
             </xs:restriction>
           </xs:simpleType>"#,
    );
    assert_eq!(violations(&label("AB"), &schema), Vec::<String>::new());
    assert_eq!(violations(&label("1234"), &schema), Vec::<String>::new());
    assert_eq!(
        violations(&label("AB12"), &schema),
        vec!["Label.Text: 'AB12' does not match any of the 2 patterns this type allows"]
    );
}

/// Patterns in different restrictions all have to hold.
#[test]
fn patterns_down_a_chain_all_apply() {
    let schema = patterned(
        r#"<xs:simpleType name="BroadType">
             <xs:restriction base="xs:string">
               <xs:pattern value="[A-Z0-9]+"/>
             </xs:restriction>
           </xs:simpleType>
           <xs:simpleType name="TextType">
             <xs:restriction base="BroadType">
               <xs:pattern value=".{4}"/>
             </xs:restriction>
           </xs:simpleType>"#,
    );
    assert_eq!(violations(&label("AB12"), &schema), Vec::<String>::new());
    // Fits the base but not the derived length.
    assert_eq!(violations(&label("AB123"), &schema).len(), 1);
    // Fits the derived length but not the base's alphabet.
    assert_eq!(violations(&label("ab12"), &schema).len(), 1);
}

/// XSD's regex language is wider than this one in a couple of corners. A
/// pattern from one of them is reported as unread, and enforces nothing,
/// rather than rejecting every value or refusing to load the schema.
#[test]
fn a_pattern_this_build_cannot_express_is_reported_not_guessed_at() {
    let schema = patterned(
        r#"<xs:simpleType name="TextType">
             <xs:restriction base="xs:string">
               <xs:pattern value="\i\c*"/>
             </xs:restriction>
           </xs:simpleType>"#,
    );
    assert_eq!(
        schema.unchecked_patterns(),
        vec![("TextType", r"\i\c*")],
        "an untranslatable pattern has to be visible"
    );
    assert_eq!(
        violations(&label("anything"), &schema),
        Vec::<String>::new()
    );
}

/// A leaf is checked against the primitive under its type before the facets
/// that narrow it.
#[test]
fn a_value_that_is_not_of_its_primitive_is_reported() {
    let schema = xsd::compile(&[r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="R" type="RT"/>
          <xs:complexType name="RT"><xs:sequence>
            <xs:element name="When" type="StampType" minOccurs="0"/>
            <xs:element name="Count" type="xs:int" minOccurs="0"/>
            <xs:element name="Flag" type="xs:boolean" minOccurs="0"/>
          </xs:sequence></xs:complexType>
          <xs:simpleType name="StampType">
            <xs:restriction base="xs:dateTime"/>
          </xs:simpleType>
        </xs:schema>"#])
    .expect("compiles");

    // Reached through a named type, as the catalog declares its timestamps.
    assert_eq!(
        violations(r#"{"R":{"When":"not-a-timestamp"}}"#, &schema),
        vec![
            "R.When: 'not-a-timestamp' is not a valid xs:dateTime: expected a date and \
             time, as CCYY-MM-DDThh:mm:ss with an optional fraction and time zone"
        ]
    );
    assert_eq!(
        violations(r#"{"R":{"When":"2026-01-22T00:00:00Z"}}"#, &schema),
        Vec::<String>::new()
    );

    // A range that no machine type would have caught on its own.
    assert_eq!(
        violations(r#"{"R":{"Count":99999999999999}}"#, &schema),
        vec![
            "R.Count: '99999999999999' is not a valid xs:int: expected between \
             -2147483648 and 2147483647"
        ]
    );
    assert_eq!(
        violations(r#"{"R":{"Count":-5}}"#, &schema),
        Vec::<String>::new()
    );
    assert_eq!(violations(r#"{"R":{"Flag":"yes"}}"#, &schema).len(), 1);
}

/// A value that is not of its type has nothing to say to the constraints
/// narrowing that type, so it is reported once rather than twice.
#[test]
fn a_value_of_the_wrong_kind_is_reported_once() {
    let schema = xsd::compile(&[r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="R" type="RT"/>
          <xs:complexType name="RT"><xs:sequence>
            <xs:element name="Level" type="LevelType"/>
          </xs:sequence></xs:complexType>
          <xs:simpleType name="LevelType">
            <xs:restriction base="xs:int">
              <xs:maxInclusive value="100"/>
            </xs:restriction>
          </xs:simpleType>
        </xs:schema>"#])
    .expect("compiles");

    let reported = violations(r#"{"R":{"Level":"not-a-number"}}"#, &schema);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(reported[0].contains("not a valid xs:int"), "{reported:?}");

    // A number of the right kind is still held to the bound.
    let reported = violations(r#"{"R":{"Level":500}}"#, &schema);
    assert_eq!(
        reported,
        vec!["R.Level: 500 is out of range: it has to be at most 100"]
    );
}

/// A type this build has no check for is named, so bringing an unfamiliar
/// schema does not quietly mean bringing unexamined values.
#[test]
fn a_primitive_with_no_check_behind_it_is_reported() {
    let schema = xsd::compile(&[r#"
        <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
          <xs:element name="R" type="RT"/>
          <xs:complexType name="RT"><xs:sequence>
            <xs:element name="Image" type="xs:base64Binary"/>
            <xs:element name="Where" type="LinkType"/>
            <xs:element name="Name" type="xs:string"/>
            <xs:element name="When" type="xs:dateTime"/>
          </xs:sequence></xs:complexType>
          <xs:simpleType name="LinkType">
            <xs:restriction base="xs:anyURI"/>
          </xs:simpleType>
        </xs:schema>"#])
    .expect("compiles");

    // Named through an element and through a simple type alike, and
    // xs:string is not among them: it has nothing to check.
    assert_eq!(
        schema.unchecked_primitives(),
        vec!["xs:anyURI", "xs:base64Binary"]
    );
    assert!(crate::slice::v25().unchecked_primitives().is_empty());
}

#[test]
fn a_mode_reads_back_the_way_it_is_written() {
    use std::str::FromStr;

    for mode in [Mode::Off, Mode::Warn, Mode::Reject] {
        assert_eq!(Mode::from_str(&mode.to_string()), Ok(mode));
    }
    assert!(Mode::from_str("strict").is_err());
    // Loaded-schema default: report, do not refuse.
    assert_eq!(Mode::default(), Mode::Warn);
}

#[test]
fn a_summary_names_the_first_few_and_counts_the_rest() {
    let schema = schema();
    let json = r#"{"Trip":{"A":1,"B":2,"C":3,"D":4}}"#;
    let message = Message::from_json(json, &schema).unwrap();
    let summary = summarize(&message.violations(&schema));

    assert!(summary.contains("'A' is not declared"), "{summary}");
    assert!(summary.contains("and 3 more"), "{summary}");
}

#[test]
fn a_report_is_capped_rather_than_as_long_as_the_payload() {
    let mut s = Schema::new();
    s.complex("WideType", vec![el_many("Item", "xs:string")])
        .element("Wide", "WideType");

    let fields: Vec<String> = (0..MAX_VIOLATIONS * 4)
        .map(|i| format!("\"Undeclared{i}\":\"x\""))
        .collect();
    let json = format!(r#"{{"Wide":{{{}}}}}"#, fields.join(","));

    let message = Message::from_json(&json, &s).unwrap();
    assert_eq!(validate(&message, &s).len(), MAX_VIOLATIONS);
}
