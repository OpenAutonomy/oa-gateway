use std::collections::BTreeMap;

use roxmltree::{Document, Node as XmlNode};
use serde_json::Number;

use crate::instance::{Complex, Field, Message, Node, Simple};
use crate::schema::Schema;
use crate::UciError;

const NS: &str = "https://www.vdl.afrl.af.mil/programs/oam";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

pub fn from_xml(text: &str, schema: &Schema) -> Result<Message, UciError> {
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
    let body = read_element(root, schema, actual, &name)?;
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

fn read_element(
    node: XmlNode<'_, '_>,
    schema: &Schema,
    type_name: &str,
    path: &str,
) -> Result<Node, UciError> {
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
            .map(|(i, c)| read_element(c, schema, child_type, &format!("{child_path}[{i}]")))
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
    indent: usize,
) -> Result<(), UciError> {
    let pad = "  ".repeat(indent);
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
                        write_element(out, fname, n, schema, child_type, false, indent + 1)?;
                        out.push('\n');
                    }
                    Field::Many(items) => {
                        for n in items {
                            write_element(out, fname, n, schema, child_type, false, indent + 1)?;
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
