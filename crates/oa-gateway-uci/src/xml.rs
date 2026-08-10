use std::collections::BTreeMap;

use roxmltree::{Document, Node as XmlNode};
use serde_json::Number;

use crate::instance::{Complex, Field, Message, Node, Simple};
use crate::schema::Schema;
use crate::{UciError, MAX_DEPTH};

const NS: &str = "https://www.vdl.afrl.af.mil/programs/oam";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub fn from_xml(text: &str, schema: &Schema) -> Result<Message, UciError> {
    // Before the parser sees it: roxmltree descends recursively and offers no
    // depth limit of its own, so a document a few thousand elements deep ends
    // the process rather than the message, and every check further down —
    // including the one in read_element — is never reached.
    if nesting_exceeds(text, MAX_DEPTH) {
        return Err(UciError::too_deep("document"));
    }
    let doc = Document::parse(text).map_err(|e| UciError::Xml(e.to_string()))?;
    let root = doc.root_element();
    let name = root.tag_name().name().to_string();
    let declared = schema
        .global_type(&name)
        .ok_or_else(|| UciError::UnknownElement(name.clone()))?;
    let xsi = root
        .attribute((XSI, "type"))
        .map(local_name)
        .map(str::to_owned);
    let actual = xsi.as_deref().unwrap_or(declared);
    let body = read_element(root, schema, actual, &name, 0)?;
    Ok(Message { name, body })
}

pub fn to_xml(message: &Message, schema: &Schema) -> Result<String, UciError> {
    let declared = schema
        .global_type(&message.name)
        .unwrap_or(message.name.as_str());
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    write_element(
        &mut out,
        &message.name,
        &message.body,
        schema,
        declared,
        true,
        0,
    )?;
    out.push('\n');
    Ok(out)
}

/// Whether element nesting in `text` could exceed `limit`, without building a
/// tree.
///
/// Deliberately an over-estimate. Rejecting an odd document costs a message;
/// under-counting costs the process, so anything ambiguous counts as nesting.
/// Comments, processing instructions, DOCTYPEs, and CDATA are skipped, since a
/// `<` inside them is text rather than an element. Quoting inside a start tag is
/// tracked for the opposite reason: `>` and `/>` are legal in an attribute
/// value, and treating one as the end of a tag is how a deep document would
/// pass for a shallow one.
pub(crate) fn nesting_exceeds(text: &str, limit: usize) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    let mut depth: usize = 0;

    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i + 1..];

        if let Some(end) = skip_past(rest, b"!--", b"-->") {
            i += 1 + end;
        } else if let Some(end) = skip_past(rest, b"![CDATA[", b"]]>") {
            i += 1 + end;
        } else if let Some(end) = skip_past(rest, b"?", b"?>") {
            i += 1 + end;
        } else if rest.first() == Some(&b'!') {
            // DOCTYPE and anything else declaration-shaped. An internal subset
            // would end at its own '>', which only ends the scan early.
            i += 1 + rest
                .iter()
                .position(|&c| c == b'>')
                .map_or(rest.len(), |p| p + 1);
        } else if rest.first() == Some(&b'/') {
            depth = depth.saturating_sub(1);
            i += 1 + rest
                .iter()
                .position(|&c| c == b'>')
                .map_or(rest.len(), |p| p + 1);
        } else {
            depth += 1;
            if depth > limit {
                return true;
            }
            let (consumed, self_closing) = scan_start_tag(rest);
            if self_closing {
                depth -= 1;
            }
            i += 1 + consumed;
        }
    }
    false
}

/// Bytes consumed if `rest` opens with `open`, through the matching `close`.
///
/// An unterminated construct consumes the remainder, which ends the scan; the
/// parser is the one that reports it as malformed.
fn skip_past(rest: &[u8], open: &[u8], close: &[u8]) -> Option<usize> {
    if !rest.starts_with(open) {
        return None;
    }
    let body = &rest[open.len()..];
    let end = body
        .windows(close.len())
        .position(|w| w == close)
        .map_or(body.len(), |p| p + close.len());
    Some(open.len() + end)
}

/// Bytes consumed by a start tag, and whether it closed itself.
fn scan_start_tag(rest: &[u8]) -> (usize, bool) {
    let mut i = 0;
    let mut quote: Option<u8> = None;
    let mut prev = 0u8;
    while i < rest.len() {
        let c = rest[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if c == b'>' => return (i + 1, prev == b'/'),
            None => {}
        }
        prev = c;
        i += 1;
    }
    (rest.len(), false)
}

fn read_element(
    node: XmlNode<'_, '_>,
    schema: &Schema,
    type_name: &str,
    path: &str,
    depth: usize,
) -> Result<Node, UciError> {
    if depth > MAX_DEPTH {
        return Err(UciError::too_deep(path));
    }
    if schema.is_simple(type_name) || !schema.is_complex(type_name) {
        let text = node.text().unwrap_or("").trim();
        return Ok(Node::Simple(parse_text(schema.primitive(type_name), text)));
    }

    let xsi = node
        .attribute((XSI, "type"))
        .map(local_name)
        .unwrap_or(type_name);
    let actual = if schema.is_complex(xsi) {
        xsi
    } else {
        type_name
    };
    let decls = schema.flatten(actual)?;

    let mut groups: BTreeMap<String, Vec<XmlNode<'_, '_>>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for child in node.children().filter(|c| c.is_element()) {
        let n = child.tag_name().name().to_string();
        if !groups.contains_key(&n) {
            order.push(n.clone());
        }
        groups.entry(n).or_default().push(child);
    }

    let mut fields = Vec::new();
    for name in order {
        let kids = groups.remove(&name).unwrap_or_default();
        let decl = decls.iter().copied().find(|e| e.name == name);
        let child_type = decl.map(|e| e.type_name.as_str()).unwrap_or("xs:string");
        let array = decl.is_some_and(|e| e.max_occurs.is_array()) || kids.len() > 1;
        let child_path = format!("{path}.{name}");
        let nodes = kids
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                read_element(
                    c,
                    schema,
                    child_type,
                    &format!("{child_path}[{i}]"),
                    depth + 1,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if array {
            fields.push((name, Field::Many(nodes)));
        } else {
            fields.push((
                name,
                Field::One(nodes.into_iter().next().expect("non-empty group")),
            ));
        }
    }

    let type_name = if actual != type_name {
        Some(actual.to_owned())
    } else {
        None
    };
    Ok(Node::Complex(Complex { type_name, fields }))
}

fn parse_text(type_name: &str, text: &str) -> Simple {
    match type_name {
        "xs:boolean" => Simple::Bool(text == "true" || text == "1"),
        "xs:int" | "xs:integer" | "xs:long" | "xs:short" | "xs:byte" => text
            .parse::<i64>()
            .map(|n| Simple::Number(n.into()))
            .unwrap_or_else(|_| Simple::String(text.to_owned())),
        "xs:double" | "xs:float" | "xs:decimal" => text
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map(Simple::Number)
            .unwrap_or_else(|| Simple::String(text.to_owned())),
        _ => Simple::String(text.to_owned()),
    }
}

fn write_element(
    out: &mut String,
    name: &str,
    node: &Node,
    schema: &Schema,
    type_name: &str,
    is_root: bool,
    depth: usize,
) -> Result<(), UciError> {
    if depth > MAX_DEPTH {
        return Err(UciError::too_deep(name));
    }
    let pad = "  ".repeat(depth);
    match node {
        Node::Simple(s) => {
            out.push_str(&pad);
            out.push('<');
            out.push_str(name);
            if is_root {
                write_root_ns(out, false);
            }
            out.push('>');
            out.push_str(&escape(s.as_text()));
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
        Node::Complex(c) => {
            let actual = c.type_name.as_deref().unwrap_or(type_name);
            out.push_str(&pad);
            out.push('<');
            out.push_str(name);
            if is_root {
                write_root_ns(out, c.type_name.is_some());
            } else if c.type_name.is_some() {
                out.push_str(" xmlns:xsi=\"");
                out.push_str(XSI);
                out.push('"');
            }
            if let Some(tn) = &c.type_name {
                out.push_str(" xsi:type=\"");
                out.push_str(tn);
                out.push('"');
            }
            if c.fields.is_empty() {
                out.push_str("/>");
                return Ok(());
            }
            out.push('>');
            out.push('\n');
            let decls = if schema.is_complex(actual) {
                schema.flatten(actual)?
            } else {
                Vec::new()
            };
            for (fname, field) in &c.fields {
                let decl = decls.iter().copied().find(|e| e.name == *fname);
                let child_type = decl.map(|e| e.type_name.as_str()).unwrap_or("xs:string");
                match field {
                    Field::One(n) => {
                        write_element(out, fname, n, schema, child_type, false, depth + 1)?;
                        out.push('\n');
                    }
                    Field::Many(items) => {
                        for n in items {
                            write_element(out, fname, n, schema, child_type, false, depth + 1)?;
                            out.push('\n');
                        }
                    }
                }
            }
            out.push_str(&pad);
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
    }
    Ok(())
}

fn write_root_ns(out: &mut String, need_xsi: bool) {
    out.push_str(" xmlns=\"");
    out.push_str(NS);
    out.push('"');
    if need_xsi {
        out.push_str(" xmlns:xsi=\"");
        out.push_str(XSI);
        out.push('"');
    }
}

fn local_name(qname: &str) -> &str {
    qname.rsplit(':').next().unwrap_or(qname)
}

fn escape(s: String) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth_of(text: &str) -> usize {
        // The smallest limit the text does not exceed is its depth.
        (0..64)
            .find(|&limit| !nesting_exceeds(text, limit))
            .expect("test inputs nest shallowly")
    }

    #[test]
    fn ordinary_documents_count_as_they_read() {
        assert_eq!(depth_of("<a/>"), 1);
        assert_eq!(depth_of("<a></a>"), 1);
        assert_eq!(depth_of("<a><b><c>x</c></b></a>"), 3);
        // Siblings are not depth.
        assert_eq!(depth_of("<a><b/><b/><b/></a>"), 2);
        assert_eq!(
            depth_of(r#"<?xml version="1.0"?><!-- note --><a><b/></a>"#),
            2
        );
    }

    #[test]
    fn markup_that_is_really_text_is_not_counted() {
        // A `<` inside a comment or CDATA opens nothing, and counting it would
        // reject documents that are perfectly valid.
        assert_eq!(depth_of("<a><!-- <b><c><d> --></a>"), 1);
        assert_eq!(depth_of("<a><![CDATA[<b><c><d>]]></a>"), 1);
        assert_eq!(depth_of("<a><?pi <b><c> ?></a>"), 1);
        assert_eq!(depth_of("<!DOCTYPE a><a><b/></a>"), 2);
    }

    #[test]
    fn a_tag_cannot_hide_depth_in_an_attribute_value() {
        // `>` and `/>` are legal in an attribute value. Read naively, each of
        // these looks like a tag that ended and closed itself, so a document
        // could nest without ever appearing to.
        assert_eq!(depth_of(r#"<a x="/>"><b x="/>"><c/></b></a>"#), 3);
        assert_eq!(depth_of(r#"<a x=">"><b x=">"><c/></b></a>"#), 3);
        assert_eq!(depth_of(r#"<a x='/>'><b/></a>"#), 2);

        let hidden: String = (0..500)
            .map(|_| r#"<n x="/>">"#)
            .collect::<Vec<_>>()
            .join("");
        assert!(
            nesting_exceeds(&hidden, MAX_DEPTH),
            "nesting behind quoted attribute values must still count"
        );
    }

    #[test]
    fn unterminated_markup_ends_the_scan_rather_than_looping() {
        assert!(!nesting_exceeds("<a", 64));
        assert!(!nesting_exceeds("<!-- open", 64));
        assert!(!nesting_exceeds("<![CDATA[ open", 64));
        assert!(!nesting_exceeds("<?pi open", 64));
        assert!(!nesting_exceeds(r#"<a x="open"#, 64));
        assert!(!nesting_exceeds("", 64));
        assert!(!nesting_exceeds("no markup at all", 64));
    }

    #[test]
    fn a_deep_document_is_refused_before_the_parser_recurses() {
        // Without this the parser exhausts the stack and takes the process with
        // it, which is why the check runs on the text rather than the tree.
        let mut deep = String::from("<leaf/>");
        for _ in 0..50_000 {
            deep = format!("<n>{deep}</n>");
        }
        assert!(nesting_exceeds(&deep, MAX_DEPTH));

        let err = from_xml(&deep, crate::slice::v25()).unwrap_err();
        assert!(matches!(err, UciError::TooDeep { .. }), "{err}");
    }
}
