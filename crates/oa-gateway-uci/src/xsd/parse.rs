use roxmltree::Node;

use crate::schema::{
    choice, sequence, ComplexContent, ComplexType, Element, Facets, MaxOccurs, Pattern, SimpleType,
};
use crate::UciError;

use super::node_util::{definitions, required, type_ref};

/// One named `complexType`: a sequence, a choice, or an extension.
pub(super) fn complex_type(node: Node<'_, '_>) -> Result<ComplexType, UciError> {
    let name = required(node, "name")?.to_owned();
    let abstract_ = node.attribute("abstract") == Some("true");

    let mut content = ComplexContent::Groups(Vec::new());
    let mut defined = false;
    for child in definitions(node)? {
        if defined {
            return Err(UciError::Xsd(format!(
                "complexType '{name}' declares more than one content model"
            )));
        }
        defined = true;
        content = match child.tag_name().name() {
            "sequence" => ComplexContent::Groups(vec![sequence(compositor(child, &name)?)]),
            "choice" => ComplexContent::Groups(vec![choice(compositor(child, &name)?)]),
            "complexContent" => complex_content(child, &name)?,
            other => {
                return Err(UciError::Xsd(format!(
                    "complexType '{name}' uses unsupported content model 'xs:{other}'"
                )))
            }
        };
    }

    Ok(ComplexType {
        name,
        abstract_,
        content,
    })
}

/// `xs:complexContent`. Only `xs:extension` is accepted.
fn complex_content(node: Node<'_, '_>, owner: &str) -> Result<ComplexContent, UciError> {
    let derivation = definitions(node)?.next().ok_or_else(|| {
        UciError::Xsd(format!(
            "complexType '{owner}' has an empty xs:complexContent"
        ))
    })?;

    let tag = derivation.tag_name().name();
    if tag != "extension" {
        return Err(UciError::Xsd(format!(
            "complexType '{owner}' derives by 'xs:{tag}'; only extension is supported"
        )));
    }

    let base = type_ref(derivation, required(derivation, "base")?);
    let mut extra = Vec::new();
    for inner in definitions(derivation)? {
        match inner.tag_name().name() {
            "sequence" => extra.push(sequence(compositor(inner, owner)?)),
            "choice" => extra.push(choice(compositor(inner, owner)?)),
            other => {
                return Err(UciError::Xsd(format!(
                    "complexType '{owner}' extends its base with unsupported 'xs:{other}'"
                )))
            }
        }
    }
    Ok(ComplexContent::Extension { base, extra })
}

/// Children of a sequence or choice. Nested compositors and compositor
/// occurrence ranges are refused.
fn compositor(node: Node<'_, '_>, owner: &str) -> Result<Vec<Element>, UciError> {
    if node.attribute("minOccurs").is_some() || node.attribute("maxOccurs").is_some() {
        return Err(UciError::Xsd(format!(
            "complexType '{owner}' puts occurrence constraints on a compositor, which the flat element model cannot represent"
        )));
    }
    let mut out = Vec::new();
    for child in definitions(node)? {
        match child.tag_name().name() {
            "element" => out.push(local_element(child, owner)?),
            other => {
                return Err(UciError::Xsd(format!(
                    "complexType '{owner}' nests 'xs:{other}' inside a compositor, which is not supported"
                )))
            }
        }
    }
    Ok(out)
}

/// A local element declaration. Anonymous inline types are refused.
fn local_element(node: Node<'_, '_>, owner: &str) -> Result<Element, UciError> {
    let name = required(node, "name")
        .map_err(|_| UciError::Xsd(format!("an element of '{owner}' has no name=")))?
        .to_owned();
    let type_name = match node.attribute("type") {
        Some(t) => type_ref(node, t),
        None => {
            return Err(UciError::Xsd(format!(
                "element '{owner}/{name}' has no type=; anonymous inline types are not supported"
            )))
        }
    };
    Ok(Element {
        name,
        type_name,
        min_occurs: min_occurs(node, owner)?,
        max_occurs: max_occurs(node, owner)?,
    })
}

/// `minOccurs`, defaulting to 1.
fn min_occurs(node: Node<'_, '_>, owner: &str) -> Result<u32, UciError> {
    match node.attribute("minOccurs") {
        None => Ok(1),
        Some(raw) => raw
            .parse()
            .map_err(|_| UciError::Xsd(format!("'{owner}' has an unreadable minOccurs='{raw}'"))),
    }
}

/// `maxOccurs`, defaulting to 1. `unbounded` is the only non-numeric
/// value accepted.
fn max_occurs(node: Node<'_, '_>, owner: &str) -> Result<MaxOccurs, UciError> {
    match node.attribute("maxOccurs") {
        None => Ok(MaxOccurs::Bounded(1)),
        Some("unbounded") => Ok(MaxOccurs::Unbounded),
        Some(raw) => raw
            .parse()
            .map(MaxOccurs::Bounded)
            .map_err(|_| UciError::Xsd(format!("'{owner}' has an unreadable maxOccurs='{raw}'"))),
    }
}

/// A named `simpleType` that restricts a base. `whiteSpace` is ignored
/// (normalization, not a constraint). An unsupported facet fails the
/// compile rather than being dropped.
pub(super) fn simple_type(node: Node<'_, '_>) -> Result<(String, SimpleType), UciError> {
    let name = required(node, "name")?.to_owned();
    let restriction = definitions(node)?
        .next()
        .ok_or_else(|| UciError::Xsd(format!("simpleType '{name}' has no xs:restriction")))?;

    let tag = restriction.tag_name().name();
    if tag != "restriction" {
        return Err(UciError::Xsd(format!(
            "simpleType '{name}' uses 'xs:{tag}'; only restriction is supported"
        )));
    }

    let base = type_ref(restriction, required(restriction, "base")?);
    let mut facets = Facets::default();
    for facet in definitions(restriction)? {
        let tag = facet.tag_name().name();
        if tag == "whiteSpace" {
            // Normalization, not a constraint: there is no value that violates
            // it, and conversion already trims what it reads.
            continue;
        }
        let value = required(facet, "value").map_err(|_| {
            UciError::Xsd(format!(
                "simpleType '{name}' has an 'xs:{tag}' facet with no value="
            ))
        })?;
        let count = |what: &str| -> Result<usize, UciError> {
            value.parse::<usize>().map_err(|_| {
                UciError::Xsd(format!(
                    "simpleType '{name}' has {what} of '{value}', which is not a length"
                ))
            })
        };
        let number = |what: &str| -> Result<f64, UciError> {
            value.parse::<f64>().map_err(|_| {
                UciError::Xsd(format!(
                    "simpleType '{name}' has {what} of '{value}', which is not a number"
                ))
            })
        };
        match tag {
            "enumeration" => facets.enumeration.push(value.to_owned()),
            "pattern" => facets.patterns.push(Pattern::new(value)),
            "length" => facets.length = Some(count("a length")?),
            "minLength" => facets.min_length = Some(count("a minLength")?),
            "maxLength" => facets.max_length = Some(count("a maxLength")?),
            "minInclusive" => facets.min_inclusive = Some(number("a minInclusive")?),
            "maxInclusive" => facets.max_inclusive = Some(number("a maxInclusive")?),
            "minExclusive" => facets.min_exclusive = Some(number("a minExclusive")?),
            "maxExclusive" => facets.max_exclusive = Some(number("a maxExclusive")?),
            other => {
                // Refused rather than skipped, on the same grounds as the rest of
                // this compiler: a constraint silently dropped is worse than one
                // that stops the schema from loading, because nothing later can
                // tell the difference between unconstrained and unread.
                return Err(UciError::Xsd(format!(
                    "simpleType '{name}' uses unsupported facet 'xs:{other}'"
                )));
            }
        }
    }

    Ok((name, SimpleType { base, facets }))
}
