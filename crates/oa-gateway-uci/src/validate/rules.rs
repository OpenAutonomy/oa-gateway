use super::violation::{Violation, ViolationKind, MAX_VIOLATIONS};
use crate::instance::{Field, Message, Node};
use crate::primitive;
use crate::schema::{GroupKind, MaxOccurs, Schema};
use crate::MAX_DEPTH;

/// Check `message` against `schema`, reporting everything that does not hold.
///
/// An empty result means the message agrees with every constraint this
/// crate reads from the schema. It does not mean the message is correct
/// against a construct the compiler refused or a primitive left
/// unchecked; see the module documentation. Stops at
/// [`MAX_VIOLATIONS`].
#[must_use]
pub fn validate(message: &Message, schema: &Schema) -> Vec<Violation> {
    let mut out = Vec::new();
    let declared = schema
        .global_type(&message.name)
        .unwrap_or(message.name.as_str());
    check(&message.body, schema, declared, &message.name, 0, &mut out);
    out
}

/// Pushes a violation unless the report is already at [`MAX_VIOLATIONS`].
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

/// Walks one node. Children are checked even when the parent already
/// reported a violation, so a bad level does not hide the ones below.
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
/// Length is counted in characters for the string types, which is what XSD
/// means there. On `xs:hexBinary` a length counts octets — two hex digits each
/// — so A-GRA's 32-character UUID is 16 octets, not a violation of `length="16"`.
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

    let (len, unit) = value_length(text, primitive);
    let mut length = |requirement: String| {
        note(
            out,
            path,
            ViolationKind::Length {
                value: text.to_owned(),
                len,
                unit,
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

/// How long `text` is, in the unit its primitive's length facet uses.
///
/// Hex digits that remain after dropping whitespace are the value; two of them
/// are one octet. Everything else is counted in characters.
fn value_length(text: &str, primitive: &str) -> (usize, &'static str) {
    if matches!(primitive::kind(primitive), primitive::Kind::HexBinary) {
        let digits = text.bytes().filter(|b| b.is_ascii_hexdigit()).count();
        (digits / 2, "octets")
    } else {
        (text.chars().count(), "characters")
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
