use roxmltree::Node;

use crate::UciError;

/// The XML Schema namespace. Documents are free to bind it to any prefix.
pub(super) const XS: &str = "http://www.w3.org/2001/XMLSchema";

/// Child elements that carry structure, with documentation skipped.
///
/// Two thirds of the published schema is `xs:annotation`, so dropping it here
/// keeps every caller from having to.
pub(super) fn definitions<'a, 'i>(
    node: Node<'a, 'i>,
) -> Result<impl Iterator<Item = Node<'a, 'i>>, UciError> {
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
pub(super) fn is_xs(node: Node<'_, '_>) -> bool {
    node.tag_name().namespace() == Some(XS)
}

/// Required attribute, or an XSD error naming the element.
pub(super) fn required<'a>(node: Node<'a, '_>, attr: &str) -> Result<&'a str, UciError> {
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
pub(super) fn type_ref(node: Node<'_, '_>, qname: &str) -> String {
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
