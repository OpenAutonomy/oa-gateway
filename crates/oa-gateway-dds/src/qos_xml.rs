//! Documented DDS-XML subset used by the rustdds provider.
//!
//! The adapter does not interpret this file. A later FFI provider may
//! pass the same path to its own loader and ignore this parser.

use std::path::Path;

use roxmltree::{Document, Node};

/// Reliability, durability, and history taken from one QoS profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QosSpec {
    pub reliability: Reliability,
    pub durability: Durability,
    pub history: History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reliability {
    BestEffort,
    Reliable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    Volatile,
    TransientLocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum History {
    KeepLast { depth: i32 },
    KeepAll,
}

impl Default for QosSpec {
    fn default() -> Self {
        Self {
            reliability: Reliability::Reliable,
            durability: Durability::Volatile,
            history: History::KeepLast { depth: 16 },
        }
    }
}

/// Reads [`QosSpec`] from `path`.
///
/// The first `<datawriter_qos>` or `<datareader_qos>` supplies the
/// policies. Omitted policies keep [`QosSpec::default`].
///
/// # Errors
///
/// Returns a message if the file cannot be read, is not well-formed
/// XML, or names an element this subset does not accept.
pub fn load(path: &Path) -> Result<QosSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    parse(&text).map_err(|err| format!("in {}: {err}", path.display()))
}

/// Parses the subset from `xml`.
///
/// # Errors
///
/// Returns a message if the document is not well-formed or names an
/// unknown element.
pub fn parse(xml: &str) -> Result<QosSpec, String> {
    let doc = Document::parse(xml).map_err(|err| err.to_string())?;
    check_unknown(doc.root_element())?;
    let mut spec = QosSpec::default();
    if let Some(qos) = first_entity_qos(doc.root_element()) {
        apply_entity(&mut spec, qos)?;
    }
    Ok(spec)
}

const ALLOWED: &[&str] = &[
    "dds",
    "qos_library",
    "qos_profile",
    "datawriter_qos",
    "datareader_qos",
    "reliability",
    "durability",
    "history",
    "kind",
    "depth",
];

fn check_unknown(node: Node<'_, '_>) -> Result<(), String> {
    if node.is_element() {
        let name = node.tag_name().name();
        if !ALLOWED.contains(&name) {
            return Err(format!("unknown element <{name}>"));
        }
        for child in node.children() {
            check_unknown(child)?;
        }
    }
    Ok(())
}

fn first_entity_qos<'a, 'input>(node: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    if node.is_element() {
        let name = node.tag_name().name();
        if name == "datawriter_qos" || name == "datareader_qos" {
            return Some(node);
        }
        for child in node.children() {
            if let Some(found) = first_entity_qos(child) {
                return Some(found);
            }
        }
    }
    None
}

fn apply_entity(spec: &mut QosSpec, qos: Node<'_, '_>) -> Result<(), String> {
    for child in qos.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "reliability" => spec.reliability = parse_reliability(child)?,
            "durability" => spec.durability = parse_durability(child)?,
            "history" => spec.history = parse_history(child)?,
            other => return Err(format!("unknown element <{other}>")),
        }
    }
    Ok(())
}

fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .and_then(|n| n.text())
        .map(str::trim)
        .map(str::to_owned)
}

fn parse_reliability(node: Node<'_, '_>) -> Result<Reliability, String> {
    match child_text(node, "kind").as_deref() {
        Some("RELIABLE") => Ok(Reliability::Reliable),
        Some("BEST_EFFORT") => Ok(Reliability::BestEffort),
        Some(other) => Err(format!("unknown reliability kind {other}")),
        None => Ok(Reliability::Reliable),
    }
}

fn parse_durability(node: Node<'_, '_>) -> Result<Durability, String> {
    match child_text(node, "kind").as_deref() {
        Some("VOLATILE") => Ok(Durability::Volatile),
        Some("TRANSIENT_LOCAL") => Ok(Durability::TransientLocal),
        Some(other) => Err(format!("unknown durability kind {other}")),
        None => Ok(Durability::Volatile),
    }
}

fn parse_history(node: Node<'_, '_>) -> Result<History, String> {
    let kind = child_text(node, "kind");
    match kind.as_deref() {
        Some("KEEP_ALL") => Ok(History::KeepAll),
        Some("KEEP_LAST") | None => {
            let depth = child_text(node, "depth")
                .as_deref()
                .unwrap_or("16")
                .parse::<i32>()
                .map_err(|err| format!("history depth: {err}"))?;
            Ok(History::KeepLast { depth })
        }
        Some(other) => Err(format!("unknown history kind {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_profile_is_reliable_volatile() {
        let spec = parse(include_str!("../../../config/dds-qos.xml")).unwrap();
        assert_eq!(spec.reliability, Reliability::Reliable);
        assert_eq!(spec.durability, Durability::Volatile);
        assert_eq!(spec.history, History::KeepLast { depth: 16 });
    }

    #[test]
    fn unknown_element_is_refused() {
        let err = parse("<dds><deadline/></dds>").unwrap_err();
        assert!(err.contains("deadline"), "{err}");
    }
}
