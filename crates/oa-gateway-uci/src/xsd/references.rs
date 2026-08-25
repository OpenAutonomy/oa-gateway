use std::collections::BTreeSet;

use crate::schema::{ComplexContent, Schema};
use crate::UciError;

/// How many unresolved type names to name before truncating the error message.
const MAX_REPORTED: usize = 10;

/// Confirm every type reference lands on something the schema defines.
///
/// This is what turns a forgotten document into a precise error instead of an
/// `UnknownType` surfacing later against live traffic.
pub(super) fn check_references(schema: &Schema) -> Result<(), UciError> {
    let mut missing = BTreeSet::new();

    for global in schema.global_elements.values() {
        note_missing(schema, &global.type_name, &mut missing);
    }
    for ct in schema.complex_types.values() {
        let (base, groups) = match &ct.content {
            ComplexContent::Groups(groups) => (None, groups),
            ComplexContent::Extension { base, extra } => (Some(base), extra),
        };
        if let Some(base) = base {
            note_missing(schema, base, &mut missing);
        }
        for el in groups.iter().flat_map(|g| g.elements.iter()) {
            note_missing(schema, &el.type_name, &mut missing);
        }
    }
    for simple in schema.simple_types.values() {
        note_missing(schema, &simple.base, &mut missing);
    }

    if missing.is_empty() {
        return Ok(());
    }

    let shown: Vec<&str> = missing
        .iter()
        .take(MAX_REPORTED)
        .map(String::as_str)
        .collect();
    let suffix = if missing.len() > shown.len() {
        format!(", and {} more", missing.len() - shown.len())
    } else {
        String::new()
    };
    Err(UciError::Xsd(format!(
        "{} type reference(s) do not resolve, so a schema document is probably missing: {}{suffix}",
        missing.len(),
        shown.join(", ")
    )))
}

/// Records `type_name` if it is neither an `xs:` primitive nor a type
/// this schema defines.
fn note_missing(schema: &Schema, type_name: &str, missing: &mut BTreeSet<String>) {
    let known = type_name.starts_with("xs:")
        || schema.complex_types.contains_key(type_name)
        || schema.simple_types.contains_key(type_name);
    if !known {
        missing.insert(type_name.to_owned());
    }
}
