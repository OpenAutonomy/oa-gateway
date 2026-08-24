use roxmltree::Document;

use crate::schema::{GlobalElement, Schema};
use crate::UciError;

use super::node_util::{definitions, is_xs, required, type_ref};
use super::parse::{complex_type, simple_type};
use super::references::check_references;

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
