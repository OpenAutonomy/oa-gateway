//! Checking a converted message against the schema it claims to follow.
//!
//! Conversion and validation answer different questions. Conversion asks whether
//! a payload can be mapped between OMS JSON and UCI XML, and it is deliberately
//! forgiving: an element it cannot place is carried as a string, and an
//! alternation is mapped as though its branches were siblings. That is what makes
//! the gateway useful before a program's message set is fully understood, and it
//! is also why a payload can convert cleanly and still not be a valid instance of
//! the standard.
//!
//! What is checked here is what the compiled schema actually states: every
//! element is declared, required elements are present, occurrence ranges hold,
//! exactly one branch of an alternation is taken, and no abstract type is
//! instantiated without naming a concrete one. Facets — enumerations, patterns,
//! lengths, ranges — are not read by the compiler yet, so a value of the right
//! shape but the wrong content still passes.
//!
//! Every violation is reported rather than the first, because an operator
//! comparing a producer against the standard wants the list, not a bisection.

use std::fmt;

use crate::instance::{Field, Message, Node};
use crate::primitive;
use crate::schema::{GroupKind, MaxOccurs, Schema};
use crate::MAX_DEPTH;

/// What an adapter does about a message that does not follow the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Do not check. Nothing is parsed on validation's behalf.
    Off,
    /// Check and report, but carry the message anyway.
    ///
    /// The default where a schema is loaded: a gateway that holds the standard
    /// and stays quiet about a producer departing from it is the silent kind of
    /// wrong, while refusing traffic that flowed yesterday is a decision an
    /// operator should make deliberately.
    #[default]
    Warn,
    /// Refuse the message and tell the peer.
    Reject,
}

impl Mode {
    #[must_use]
    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Reject => "reject",
        })
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(Self::Off),
            "warn" => Ok(Self::Warn),
            "reject" => Ok(Self::Reject),
            other => Err(format!(
                "unknown validation mode '{other}'; expected off, warn, or reject"
            )),
        }
    }
}

/// A report short enough for a log line or an error frame.
///
/// Names the first few violations and how many were left out, since the first
/// one is usually the one to act on and the count says whether to go looking.
#[must_use]
pub fn summarize(violations: &[Violation]) -> String {
    const SHOWN: usize = 3;
    let shown: Vec<String> = violations
        .iter()
        .take(SHOWN)
        .map(ToString::to_string)
        .collect();
    let mut out = shown.join("; ");
    if violations.len() > SHOWN {
        out.push_str(&format!(" (and {} more)", violations.len() - SHOWN));
    }
    out
}

/// Violations reported for one message before the rest are elided.
///
/// A payload built to produce one violation per element would otherwise be
/// answered with a report as large as itself, which is a poor thing to put in a
/// log line or an error frame.
pub const MAX_VIOLATIONS: usize = 32;

/// One way in which a message departs from the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Dotted path to the offending element, starting at the message name.
    pub path: String,
    pub kind: ViolationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    /// An element the type does not declare.
    Undeclared { element: String },
    /// A required element that is absent.
    Missing { element: String },
    /// Present, but fewer times than `minOccurs`.
    TooFew {
        element: String,
        min: u32,
        found: usize,
    },
    /// Present more times than `maxOccurs`.
    TooMany {
        element: String,
        max: u32,
        found: usize,
    },
    /// An alternation with no branch taken.
    NoAlternative { alternatives: Vec<String> },
    /// An alternation with more than one branch taken.
    ManyAlternatives { taken: Vec<String> },
    /// An abstract type used without naming a concrete one.
    AbstractType { type_name: String },
    /// A declared type the schema does not define, or a cyclic chain.
    UnusableType { type_name: String, reason: String },
    /// Nesting past the depth conversion would have refused.
    TooDeep,
    /// A value outside the enumeration its type declares.
    ///
    /// `allowed` is a sample, since some enumerations run to hundreds of values.
    NotEnumerated {
        value: String,
        allowed: Vec<String>,
        total: usize,
    },
    /// A value of the wrong length.
    Length {
        value: String,
        len: usize,
        requirement: String,
    },
    /// A number outside the bounds its type declares.
    Range { value: String, requirement: String },
    /// A value matching none of the patterns declared at one link of its type's
    /// restriction chain.
    PatternMismatch {
        value: String,
        patterns: Vec<String>,
    },
    /// A value that is not one of the primitive underneath its type at all.
    NotLexical {
        value: String,
        primitive: String,
        expected: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.kind)
    }
}

impl fmt::Display for ViolationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undeclared { element } => {
                write!(f, "'{element}' is not declared by this type")
            }
            Self::Missing { element } => write!(f, "'{element}' is required and absent"),
            Self::TooFew {
                element,
                min,
                found,
            } => write!(f, "'{element}' appears {found} times, minOccurs is {min}"),
            Self::TooMany {
                element,
                max,
                found,
            } => write!(f, "'{element}' appears {found} times, maxOccurs is {max}"),
            Self::NoAlternative { alternatives } => write!(
                f,
                "none of {} is present, and one of them must be",
                quoted(alternatives)
            ),
            Self::ManyAlternatives { taken } => write!(
                f,
                "{} are all present, and they are alternatives",
                quoted(taken)
            ),
            Self::AbstractType { type_name } => write!(
                f,
                "'{type_name}' is abstract, so a concrete type has to be named"
            ),
            Self::UnusableType { type_name, reason } => {
                write!(f, "declared type '{type_name}' cannot be used: {reason}")
            }
            Self::TooDeep => write!(f, "nests deeper than {MAX_DEPTH} elements"),
            Self::NotEnumerated {
                value,
                allowed,
                total,
            } => {
                let sample = quoted(allowed);
                let more = if *total > allowed.len() { ", …" } else { "" };
                write!(
                    f,
                    "'{}' is not one of the {total} values this type allows: {sample}{more}",
                    abbreviated(value)
                )
            }
            Self::Length {
                value,
                len,
                requirement,
            } => write!(
                f,
                "'{}' is {len} characters, and has to be {requirement}",
                abbreviated(value)
            ),
            Self::Range { value, requirement } => {
                write!(
                    f,
                    "{} is out of range: it has to be {requirement}",
                    abbreviated(value)
                )
            }
            Self::NotLexical {
                value,
                primitive,
                expected,
            } => write!(
                f,
                "'{}' is not a valid {primitive}: expected {expected}",
                abbreviated(value)
            ),
            Self::PatternMismatch { value, patterns } => match patterns.as_slice() {
                [only] => write!(
                    f,
                    "'{}' does not match the pattern '{}'",
                    abbreviated(value),
                    abbreviated(only)
                ),
                many => write!(
                    f,
                    "'{}' does not match any of the {} patterns this type allows",
                    abbreviated(value),
                    many.len()
                ),
            },
        }
    }
}

/// Values arrive from the wire and leave through logs and error frames, so a
/// long one is cut rather than repeated whole.
fn abbreviated(text: &str) -> String {
    const MAX: usize = 60;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    format!("{}…", text.chars().take(MAX).collect::<String>())
}

fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Check `message` against `schema`, reporting everything that does not hold.
///
/// An empty result means the message agrees with every constraint the compiled
/// schema carries. It does not mean the message is correct: see the module
/// documentation for what is not yet read from the XSD.
#[must_use]
pub fn validate(message: &Message, schema: &Schema) -> Vec<Violation> {
    let mut out = Vec::new();
    let declared = schema
        .global_type(&message.name)
        .unwrap_or(message.name.as_str());
    check(&message.body, schema, declared, &message.name, 0, &mut out);
    out
}

fn note(out: &mut Vec<Violation>, path: &str, kind: ViolationKind) {
    if out.len() < MAX_VIOLATIONS {
        out.push(Violation {
            path: path.to_owned(),
            kind,
        });
    }
}

fn full(out: &[Violation]) -> bool {
    out.len() >= MAX_VIOLATIONS
}

fn check(
    node: &Node,
    schema: &Schema,
    type_name: &str,
    path: &str,
    depth: usize,
    out: &mut Vec<Violation>,
) {
    if full(out) {
        return;
    }
    if depth > MAX_DEPTH {
        note(out, path, ViolationKind::TooDeep);
        return;
    }
    let Node::Complex(complex) = node else {
        if let Node::Simple(value) = node {
            check_value(&value.as_text(), schema, type_name, path, out);
        }
        return;
    };

    let actual = complex.type_name.as_deref().unwrap_or(type_name);
    if complex.type_name.is_none() {
        if let Some(ct) = schema.complex_types.get(actual) {
            if ct.abstract_ {
                note(
                    out,
                    path,
                    ViolationKind::AbstractType {
                        type_name: actual.to_owned(),
                    },
                );
            }
        }
    }

    let groups = match schema.groups(actual) {
        Ok(groups) => groups,
        Err(err) => {
            note(
                out,
                path,
                ViolationKind::UnusableType {
                    type_name: actual.to_owned(),
                    reason: err.to_string(),
                },
            );
            return;
        }
    };

    let count = |name: &str| match complex.get(name) {
        None => 0,
        Some(Field::One(_)) => 1,
        Some(Field::Many(items)) => items.len(),
    };

    for (name, _) in &complex.fields {
        let declared = groups
            .iter()
            .flat_map(|g| g.elements.iter())
            .any(|e| &e.name == name);
        if !declared {
            note(
                out,
                path,
                ViolationKind::Undeclared {
                    element: name.clone(),
                },
            );
        }
    }

    for group in &groups {
        match group.kind {
            GroupKind::Sequence => {
                for decl in &group.elements {
                    let found = count(&decl.name);
                    if found == 0 {
                        if decl.min_occurs >= 1 {
                            note(
                                out,
                                path,
                                ViolationKind::Missing {
                                    element: decl.name.clone(),
                                },
                            );
                        }
                        continue;
                    }
                    if found < decl.min_occurs as usize {
                        note(
                            out,
                            path,
                            ViolationKind::TooFew {
                                element: decl.name.clone(),
                                min: decl.min_occurs,
                                found,
                            },
                        );
                    }
                    check_max(decl.max_occurs, &decl.name, found, path, out);
                }
            }
            GroupKind::Choice => {
                // Every compositor the XSD compiler accepts carries the default
                // occurrence range, since it refuses one that does not — so an
                // alternation here always means exactly one branch, and the
                // members' own minOccurs says nothing about which.
                let taken: Vec<String> = group
                    .elements
                    .iter()
                    .filter(|e| count(&e.name) > 0)
                    .map(|e| e.name.clone())
                    .collect();
                let names = || group.elements.iter().map(|e| e.name.clone()).collect();
                match taken.len() {
                    1 => {
                        let decl = &group.elements[0];
                        let chosen = group
                            .elements
                            .iter()
                            .find(|e| e.name == taken[0])
                            .unwrap_or(decl);
                        check_max(chosen.max_occurs, &taken[0], count(&taken[0]), path, out);
                    }
                    0 => note(
                        out,
                        path,
                        ViolationKind::NoAlternative {
                            alternatives: names(),
                        },
                    ),
                    _ => note(out, path, ViolationKind::ManyAlternatives { taken }),
                }
            }
        }
    }

    // Children are checked whatever the parent reported: a report that stopped
    // at the first bad level would hide everything below it.
    for (name, field) in &complex.fields {
        let Some(decl) = groups
            .iter()
            .flat_map(|g| g.elements.iter())
            .find(|e| &e.name == name)
        else {
            continue;
        };
        let child_path = format!("{path}.{name}");
        match field {
            Field::One(child) => {
                check(child, schema, &decl.type_name, &child_path, depth + 1, out);
            }
            Field::Many(items) => {
                for (i, child) in items.iter().enumerate() {
                    check(
                        child,
                        schema,
                        &decl.type_name,
                        &format!("{child_path}[{i}]"),
                        depth + 1,
                        out,
                    );
                }
            }
        }
    }
}

/// Check a leaf against the facets of the type it was declared as.
///
/// Length is counted in characters, which is what XSD means for the string types
/// these facets appear on. It is not what `xs:hexBinary` means — there a length
/// counts octets — so a length facet on binary content would be read too
/// strictly. Nothing in the published catalog does that.
fn check_value(text: &str, schema: &Schema, type_name: &str, path: &str, out: &mut Vec<Violation>) {
    // What the value is comes before what it is narrowed to. A value that is not
    // a number at all has nothing to say to a bound, and reporting both would
    // describe one mistake twice.
    let primitive = schema.primitive(type_name);
    if let Some(expected) = primitive::refuses(&primitive::kind(primitive), text) {
        note(
            out,
            path,
            ViolationKind::NotLexical {
                value: text.to_owned(),
                primitive: primitive.to_owned(),
                expected,
            },
        );
        return;
    }

    let facets = schema.effective_facets(type_name);
    if facets.is_empty() {
        return;
    }

    if let Some(allowed) = facets.enumeration {
        if !allowed.iter().any(|value| value == text) {
            const SAMPLE: usize = 4;
            note(
                out,
                path,
                ViolationKind::NotEnumerated {
                    value: text.to_owned(),
                    allowed: allowed.iter().take(SAMPLE).cloned().collect(),
                    total: allowed.len(),
                },
            );
        }
    }

    let len = text.chars().count();
    let mut length = |requirement: String| {
        note(
            out,
            path,
            ViolationKind::Length {
                value: text.to_owned(),
                len,
                requirement,
            },
        );
    };
    if let Some(exact) = facets.length {
        if len != exact {
            length(format!("exactly {exact}"));
        }
    }
    if let Some(min) = facets.min_length {
        if len < min {
            length(format!("at least {min}"));
        }
    }
    if let Some(max) = facets.max_length {
        if len > max {
            length(format!("at most {max}"));
        }
    }

    for alternatives in &facets.patterns {
        if !alternatives.iter().any(|pattern| pattern.accepts(text)) {
            note(
                out,
                path,
                ViolationKind::PatternMismatch {
                    value: text.to_owned(),
                    patterns: alternatives
                        .iter()
                        .map(|pattern| pattern.source().to_owned())
                        .collect(),
                },
            );
        }
    }

    // A bound only means something against a number. A value that will not parse
    // as one is a conversion concern, and conversion has already had its say.
    let bounded = facets.min_inclusive.is_some()
        || facets.max_inclusive.is_some()
        || facets.min_exclusive.is_some()
        || facets.max_exclusive.is_some();
    if !bounded {
        return;
    }
    let Ok(number) = text.parse::<f64>() else {
        return;
    };
    let mut range = |requirement: String| {
        note(
            out,
            path,
            ViolationKind::Range {
                value: text.to_owned(),
                requirement,
            },
        );
    };
    if let Some(min) = facets.min_inclusive {
        if number < min {
            range(format!("at least {min}"));
        }
    }
    if let Some(max) = facets.max_inclusive {
        if number > max {
            range(format!("at most {max}"));
        }
    }
    if let Some(min) = facets.min_exclusive {
        if number <= min {
            range(format!("greater than {min}"));
        }
    }
    if let Some(max) = facets.max_exclusive {
        if number >= max {
            range(format!("less than {max}"));
        }
    }
}

fn check_max(max: MaxOccurs, element: &str, found: usize, path: &str, out: &mut Vec<Violation>) {
    if let MaxOccurs::Bounded(max) = max {
        if found > max as usize {
            note(
                out,
                path,
                ViolationKind::TooMany {
                    element: element.to_owned(),
                    max,
                    found,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{choice, el, el_many, el_opt, sequence, Element, Facets, MaxOccurs};
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
}
