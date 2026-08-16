//! Schema-annotated instance tree. Adapters convert; the engine never
//! sees this.
//!
//! [`Message::name`] is the global element. [`Complex::type_name`] is
//! set when `$type` / `xsi:type` overrides the declared type.
//! [`Field::Many`] is a repeating element, not a JSON array invented
//! by the codec.

use serde_json::Number;

/// One UCI / OMS message: a global element and its body.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub name: String,
    pub body: Node,
}

/// A leaf or a complex element.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Simple(Simple),
    Complex(Complex),
}

/// A scalar as JSON would carry it. XML text that does not fit the
/// declared primitive stays a [`Self::String`] rather than being
/// coerced.
#[derive(Debug, Clone, PartialEq)]
pub enum Simple {
    String(String),
    Bool(bool),
    Number(Number),
}

/// Child fields of a complex type, in document order.
#[derive(Debug, Clone, PartialEq)]
pub struct Complex {
    /// Set when `$type` / `xsi:type` overrides the declared type.
    pub type_name: Option<String>,
    pub fields: Vec<(String, Field)>,
}

/// One child name: a single occurrence or a repeating group.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    One(Node),
    Many(Vec<Node>),
}

impl Complex {
    /// First field whose name equals `name`. Case-sensitive.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
}

impl Simple {
    /// Lexical form for XML text and for validation.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::Bool(true) => "true".into(),
            Self::Bool(false) => "false".into(),
            Self::Number(n) => n.to_string(),
        }
    }
}
