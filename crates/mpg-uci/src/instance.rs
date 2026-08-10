//! Schema-annotated instance tree. Adapters convert; the engine never sees this.

use serde_json::Number;

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub name: String,
    pub body: Node,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Simple(Simple),
    Complex(Complex),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Simple {
    String(String),
    Bool(bool),
    Number(Number),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Complex {
    /// Set when `$type` / `xsi:type` overrides the declared type.
    pub type_name: Option<String>,
    pub fields: Vec<(String, Field)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    One(Node),
    Many(Vec<Node>),
}

impl Complex {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
}

impl Simple {
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
