//! Compile the published UCI XSD into a [`Schema`].
//!
//! This is not a general XSD processor. UCI is written against its own Schema
//! Style & Design Specification, which confines it to a narrow subset of XML
//! Schema: every type is top-level and named, every element declaration refers
//! to a type by name, compositors never nest, and the only derivation is
//! extension. This module accepts that subset and rejects everything else
//! rather than guessing, so a schema revision that starts using a new construct
//! fails loudly instead of quietly losing data.

use std::collections::BTreeSet;

use roxmltree::{Document, Node};

use crate::schema::{
    choice, sequence, ComplexContent, ComplexType, Element, Facets, GlobalElement, MaxOccurs,
    Pattern, Schema, SimpleType,
};
use crate::UciError;

/// The XML Schema namespace. Documents are free to bind it to any prefix.
const XS: &str = "http://www.w3.org/2001/XMLSchema";

/// How many unresolved type names to name before truncating the error message.
const MAX_REPORTED: usize = 10;

/// Compile XSD documents into a single [`Schema`].
///
/// Pass every document the schema spans; `xs:include` and `xs:import`
/// directives are not followed, because resolving them would mean reading paths
/// out of the schema text. Anything left dangling is reported by name, so a
/// forgotten document surfaces as a clear error rather than a missing type at
/// conversion time.
///
/// # Errors
///
/// Returns [`UciError::Xsd`] if a document is not well-formed, uses a construct
/// outside the supported subset, defines the same name twice, or leaves a type
/// reference unresolved.
///
/// # Examples
///
/// ```
/// # use oa_gateway_uci::xsd;
/// let schema = xsd::compile(&[r#"
///     <xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
///                xmlns:uci="urn:example"
///                targetNamespace="urn:example">
///       <xs:element name="Ping" type="uci:PingType"/>
///       <xs:complexType name="PingType">
///         <xs:sequence>
///           <xs:element name="n" type="xs:int" minOccurs="0"/>
///         </xs:sequence>
///       </xs:complexType>
///     </xs:schema>
/// "#])?;
///
/// assert_eq!(schema.global_type("Ping"), Some("PingType"));
/// # Ok::<(), oa_gateway_uci::UciError>(())
/// ```
pub fn compile(documents: &[&str]) -> Result<Schema, UciError> {
    let mut schema = Schema::new();
    for (index, text) in documents.iter().enumerate() {
        // A schema is operator-supplied rather than peer-supplied, so this is
        // about a truncated download or the wrong file, not an attacker. It
        // still has to fail as an error: the parser would otherwise exhaust the
        // stack, and a gateway that aborts at startup says nothing about why.
        if crate::xml::nesting_exceeds(text, crate::MAX_DEPTH) {
            return Err(UciError::Xsd(format!(
                "document {index} nests deeper than {} elements",
                crate::MAX_DEPTH
            )));
        }
        let doc = Document::parse(text)
            .map_err(|e| UciError::Xsd(format!("document {index} is not well-formed XML: {e}")))?;
        add_document(&doc, &mut schema)?;
    }
    check_references(&schema)?;
    Ok(schema)
}

/// Merges one `<xs:schema>` into `schema`. `include` / `import` are
/// ignored; the caller must pass those documents separately.
fn add_document(doc: &Document<'_>, schema: &mut Schema) -> Result<(), UciError> {
    let root = doc.root_element();
    if !is_xs(root) || root.tag_name().name() != "schema" {
        return Err(UciError::Xsd(format!(
            "root element is <{}>, expected <xs:schema>",
            root.tag_name().name()
        )));
    }

    for child in definitions(root)? {
        match child.tag_name().name() {
            "element" => {
                let name = required(child, "name")?.to_owned();
                let type_name = type_ref(child, required(child, "type").map_err(|_| {
                    UciError::Xsd(format!(
                        "global element '{name}' has no type=; anonymous inline types are not supported"
                    ))
                })?);
                claim(schema, &name, Definition::Element)?;
                schema
                    .global_elements
                    .insert(name, GlobalElement { type_name });
            }
            "complexType" => {
                let ct = complex_type(child)?;
                claim(schema, &ct.name, Definition::Complex)?;
                schema.complex_types.insert(ct.name.clone(), ct);
            }
            "simpleType" => {
                let (name, simple) = simple_type(child)?;
                claim(schema, &name, Definition::Simple)?;
                schema.simple_types.insert(name, simple);
            }
            // Resolved by the caller supplying every document, not by us
            // reading paths out of the schema text.
            "include" | "import" => {}
            other => {
                return Err(UciError::Xsd(format!(
                    "top-level 'xs:{other}' is outside the supported subset"
                )))
            }
        }
    }
    Ok(())
}

/// Which map a name would land in, used to report redefinitions precisely.
#[derive(Clone, Copy)]
enum Definition {
    Element,
    Complex,
    Simple,
}

/// Reject a redefinition rather than letting the last document silently win.
///
/// The most likely cause is passing the same document twice — the standard ships
/// more than one copy of the message definitions — which would otherwise look
/// like it worked.
fn claim(schema: &Schema, name: &str, kind: Definition) -> Result<(), UciError> {
    let taken = match kind {
        Definition::Element => schema.global_elements.contains_key(name),
        Definition::Complex => schema.complex_types.contains_key(name),
        Definition::Simple => schema.simple_types.contains_key(name),
    };
    if taken {
        let what = match kind {
            Definition::Element => "global element",
            Definition::Complex => "complexType",
            Definition::Simple => "simpleType",
        };
        return Err(UciError::Xsd(format!(
            "{what} '{name}' is defined twice; is the same document passed more than once?"
        )));
    }
    Ok(())
}

/// One named `complexType`: a sequence, a choice, or an extension.
fn complex_type(node: Node<'_, '_>) -> Result<ComplexType, UciError> {
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
fn simple_type(node: Node<'_, '_>) -> Result<(String, SimpleType), UciError> {
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

/// Child elements that carry structure, with documentation skipped.
///
/// Two thirds of the published schema is `xs:annotation`, so dropping it here
/// keeps every caller from having to.
fn definitions<'a, 'i>(node: Node<'a, 'i>) -> Result<impl Iterator<Item = Node<'a, 'i>>, UciError> {
    for child in node.children().filter(Node::is_element) {
        if !is_xs(child) {
            return Err(UciError::Xsd(format!(
                "unexpected element <{}> outside the XML Schema namespace",
                child.tag_name().name()
            )));
        }
    }
    Ok(node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() != "annotation"))
}

/// Whether `node` is in the XML Schema namespace.
fn is_xs(node: Node<'_, '_>) -> bool {
    node.tag_name().namespace() == Some(XS)
}

/// Required attribute, or an XSD error naming the element.
fn required<'a>(node: Node<'a, '_>, attr: &str) -> Result<&'a str, UciError> {
    node.attribute(attr).ok_or_else(|| {
        UciError::Xsd(format!(
            "<xs:{}> is missing the required {attr}= attribute",
            node.tag_name().name()
        ))
    })
}

/// Normalize a QName into the form the rest of the crate expects.
///
/// XML Schema primitives become `xs:name` regardless of the prefix the document
/// binds them to, and names in any other namespace reduce to their local part.
/// Reducing is safe because UCI defines everything in a single target namespace;
/// the prefix cannot be stripped blindly, since that would turn `xs:string` into
/// `string` and defeat every leaf coercion.
fn type_ref(node: Node<'_, '_>, qname: &str) -> String {
    let (prefix, local) = match qname.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, qname),
    };
    if node.lookup_namespace_uri(prefix) == Some(XS) {
        format!("xs:{local}")
    } else {
        local.to_owned()
    }
}

/// Confirm every type reference lands on something the schema defines.
///
/// This is what turns a forgotten document into a precise error instead of an
/// `UnknownType` surfacing later against live traffic.
fn check_references(schema: &Schema) -> Result<(), UciError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    fn wrap(body: &str) -> String {
        format!(
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
                          xmlns:uci="urn:example"
                          targetNamespace="urn:example">{body}</xs:schema>"#
        )
    }

    fn compile_one(body: &str) -> Result<Schema, UciError> {
        compile(&[&wrap(body)])
    }

    #[test]
    fn sequence_carries_names_types_and_occurrences() {
        let schema = compile_one(
            r#"<xs:complexType name="T">
                 <xs:sequence>
                   <xs:element name="a" type="xs:string"/>
                   <xs:element name="b" type="xs:int" minOccurs="0"/>
                   <xs:element name="c" type="xs:int" maxOccurs="unbounded"/>
                   <xs:element name="d" type="xs:int" maxOccurs="8"/>
                 </xs:sequence>
               </xs:complexType>"#,
        )
        .unwrap();

        let decls = schema.flatten("T").unwrap();
        let names: Vec<&str> = decls.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert_eq!(decls[0].min_occurs, 1);
        assert_eq!(decls[1].min_occurs, 0);
        assert_eq!(decls[0].type_name, "xs:string");
        assert_eq!(decls[2].max_occurs, MaxOccurs::Unbounded);
        assert_eq!(decls[3].max_occurs, MaxOccurs::Bounded(8));
        assert!(decls[3].max_occurs.is_array());
        assert!(!decls[0].max_occurs.is_array());
    }

    #[test]
    fn named_simple_types_resolve_through_their_chain() {
        let schema = compile_one(
            r#"<xs:simpleType name="Outer">
                 <xs:restriction base="uci:Inner"/>
               </xs:simpleType>
               <xs:simpleType name="Inner">
                 <xs:restriction base="xs:double">
                   <xs:minInclusive value="0"/>
                 </xs:restriction>
               </xs:simpleType>"#,
        )
        .unwrap();

        assert!(schema.is_simple("Outer"));
        assert!(schema.is_simple("Inner"));
        assert_eq!(schema.primitive("Outer"), "xs:double");
        assert_eq!(schema.primitive("Inner"), "xs:double");
        assert_eq!(schema.primitive("xs:int"), "xs:int");
    }

    #[test]
    fn enumerations_reduce_to_their_base_primitive() {
        let schema = compile_one(
            r#"<xs:simpleType name="ClassificationEnum">
                 <xs:restriction base="xs:string">
                   <xs:enumeration value="U"/>
                   <xs:enumeration value="S"/>
                 </xs:restriction>
               </xs:simpleType>"#,
        )
        .unwrap();

        assert_eq!(schema.primitive("ClassificationEnum"), "xs:string");
    }

    #[test]
    fn extension_flattens_base_before_extra() {
        let schema = compile_one(
            r#"<xs:complexType name="Base" abstract="true">
                 <xs:sequence><xs:element name="kind" type="xs:string"/></xs:sequence>
               </xs:complexType>
               <xs:complexType name="Derived">
                 <xs:complexContent>
                   <xs:extension base="uci:Base">
                     <xs:sequence><xs:element name="extra" type="xs:int"/></xs:sequence>
                   </xs:extension>
                 </xs:complexContent>
               </xs:complexType>"#,
        )
        .unwrap();

        assert!(schema.complex_types["Base"].abstract_);
        assert!(!schema.complex_types["Derived"].abstract_);
        let names: Vec<&str> = schema
            .flatten("Derived")
            .unwrap()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, ["kind", "extra"]);
    }

    #[test]
    fn extension_without_a_compositor_inherits_only() {
        let schema = compile_one(
            r#"<xs:complexType name="Base">
                 <xs:sequence><xs:element name="kind" type="xs:string"/></xs:sequence>
               </xs:complexType>
               <xs:complexType name="Derived">
                 <xs:complexContent><xs:extension base="uci:Base"/></xs:complexContent>
               </xs:complexType>"#,
        )
        .unwrap();

        assert_eq!(schema.flatten("Derived").unwrap().len(), 1);
    }

    #[test]
    fn choice_and_empty_content_are_represented() {
        let schema = compile_one(
            r#"<xs:complexType name="Pick">
                 <xs:choice>
                   <xs:element name="a" type="xs:string"/>
                   <xs:element name="b" type="xs:string"/>
                 </xs:choice>
               </xs:complexType>
               <xs:complexType name="Nothing"/>"#,
        )
        .unwrap();

        assert_eq!(
            schema.groups("Pick").unwrap()[0].kind,
            crate::schema::GroupKind::Choice,
            "an alternation must not compile to a sequence"
        );
        assert!(schema.groups("Nothing").unwrap().is_empty());
        assert!(schema.flatten("Nothing").unwrap().is_empty());
    }

    #[test]
    fn primitives_are_canonical_whatever_prefix_the_document_uses() {
        let schema = compile(
            &[r#"<xsd:schema xmlns:xsd="http://www.w3.org/2001/XMLSchema"
                                              xmlns:t="urn:example"
                                              targetNamespace="urn:example">
                                   <xsd:element name="E" type="t:T"/>
                                   <xsd:complexType name="T">
                                     <xsd:sequence>
                                       <xsd:element name="n" type="xsd:int"/>
                                     </xsd:sequence>
                                   </xsd:complexType>
                                 </xsd:schema>"#],
        )
        .unwrap();

        assert_eq!(schema.global_type("E"), Some("T"));
        assert_eq!(schema.flatten("T").unwrap()[0].type_name, "xs:int");
    }

    #[test]
    fn documents_compose_across_an_include_boundary() {
        let a = wrap(
            r#"<xs:include schemaLocation="b.xsd"/>
               <xs:element name="Root" type="uci:RootType"/>
               <xs:complexType name="RootType">
                 <xs:sequence><xs:element name="marking" type="uci:MarkingType"/></xs:sequence>
               </xs:complexType>"#,
        );
        let b = wrap(
            r#"<xs:complexType name="MarkingType">
                 <xs:sequence><xs:element name="Classification" type="xs:string"/></xs:sequence>
               </xs:complexType>"#,
        );

        let schema = compile(&[&a, &b]).unwrap();
        assert!(schema.is_complex("MarkingType"));

        // The same reference is a hard error when the second document is absent.
        let err = compile(&[&a]).unwrap_err();
        assert!(
            matches!(&err, UciError::Xsd(m) if m.contains("MarkingType") && m.contains("missing")),
            "{err}"
        );
    }

    #[test]
    fn unsupported_constructs_are_rejected_rather_than_ignored() {
        let cases = [
            (
                r#"<xs:complexType name="T">
                     <xs:sequence><xs:element name="a" type="xs:string"/></xs:sequence>
                     <xs:attribute name="id" type="xs:string"/>
                   </xs:complexType>"#,
                "more than one content model",
            ),
            (
                r#"<xs:complexType name="T">
                     <xs:sequence>
                       <xs:choice><xs:element name="a" type="xs:string"/></xs:choice>
                     </xs:sequence>
                   </xs:complexType>"#,
                "nests 'xs:choice'",
            ),
            (
                r#"<xs:complexType name="T">
                     <xs:complexContent>
                       <xs:restriction base="xs:anyType"/>
                     </xs:complexContent>
                   </xs:complexType>"#,
                "only extension is supported",
            ),
            (
                r#"<xs:complexType name="T">
                     <xs:sequence><xs:element name="a"><xs:complexType/></xs:element></xs:sequence>
                   </xs:complexType>"#,
                "anonymous inline types are not supported",
            ),
            (
                r#"<xs:simpleType name="T">
                     <xs:union memberTypes="xs:int xs:string"/>
                   </xs:simpleType>"#,
                "only restriction is supported",
            ),
            (
                r#"<xs:complexType name="T">
                     <xs:sequence maxOccurs="unbounded">
                       <xs:element name="a" type="xs:string"/>
                     </xs:sequence>
                   </xs:complexType>"#,
                "occurrence constraints on a compositor",
            ),
            (r#"<xs:group name="G"/>"#, "top-level 'xs:group'"),
        ];

        for (body, expected) in cases {
            let err = compile_one(body).unwrap_err();
            assert!(
                matches!(&err, UciError::Xsd(m) if m.contains(expected)),
                "expected {expected:?}, got {err}"
            );
        }
    }

    #[test]
    fn redefinition_is_an_error() {
        let doc = wrap(r#"<xs:complexType name="T"/>"#);
        let err = compile(&[&doc, &doc]).unwrap_err();
        assert!(
            matches!(&err, UciError::Xsd(m) if m.contains("defined twice")),
            "{err}"
        );
    }

    #[test]
    fn malformed_xml_is_reported_with_its_position() {
        let err = compile(&["<xs:schema>"]).unwrap_err();
        assert!(
            matches!(&err, UciError::Xsd(m) if m.contains("well-formed")),
            "{err}"
        );
    }

    #[test]
    fn a_compiled_schema_drives_conversion_including_named_simple_types() {
        let schema = compile_one(
            r#"<xs:element name="Report" type="uci:ReportType"/>
               <xs:complexType name="ReportType">
                 <xs:sequence>
                   <xs:element name="Latitude" type="uci:LatitudeType"/>
                   <xs:element name="Label" type="uci:LabelType" minOccurs="0"/>
                   <xs:element name="Tag" type="xs:string" maxOccurs="unbounded"/>
                 </xs:sequence>
               </xs:complexType>
               <xs:simpleType name="LatitudeType">
                 <xs:restriction base="xs:double"/>
               </xs:simpleType>
               <xs:simpleType name="LabelType">
                 <xs:restriction base="xs:string">
                   <xs:enumeration value="alpha"/>
                 </xs:restriction>
               </xs:simpleType>"#,
        )
        .unwrap();

        let src = r#"{"Report":{"Latitude":12.5,"Label":"alpha","Tag":["x","y"]}}"#;
        let xml = Message::from_json(src, &schema)
            .unwrap()
            .to_xml(&schema)
            .unwrap();
        assert!(xml.contains("<Latitude>12.5</Latitude>"), "{xml}");
        assert_eq!(xml.matches("<Tag>").count(), 2, "{xml}");

        let back = Message::from_xml(&xml, &schema).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&back.to_json(&schema).unwrap()).unwrap();

        // The point of the simple-type map: a named type restricting xs:double
        // has to come back as a JSON number, and one restricting xs:string as a
        // string, even though neither carries an xs: prefix in the schema.
        assert_eq!(
            value
                .pointer("/Report/Latitude")
                .and_then(serde_json::Value::as_f64),
            Some(12.5)
        );
        assert_eq!(
            value
                .pointer("/Report/Label")
                .and_then(serde_json::Value::as_str),
            Some("alpha")
        );
        assert_eq!(
            value
                .pointer("/Report/Tag")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(2)
        );
    }
}
