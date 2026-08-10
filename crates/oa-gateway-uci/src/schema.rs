//! In-memory model of a UCI schema. Enough for the OMS JSON rules; not a full
//! XSD processor.
//!
//! Build one by hand with the builder methods below, or compile the published
//! XSD into one with [`crate::xsd::compile`].

use std::collections::HashMap;

/// How many links of a named-simple-type chain [`Schema::primitive`] will follow
/// before giving up. The published schema nests three deep at most; the limit
/// exists so a cyclic hand-built schema cannot hang the caller.
const MAX_SIMPLE_DEPTH: usize = 16;

#[derive(Debug, Clone)]
pub struct Schema {
    pub global_elements: HashMap<String, GlobalElement>,
    pub complex_types: HashMap<String, ComplexType>,
    /// Named simple types, mapped to the type each one restricts. The target is
    /// usually an `xs:` primitive but may be another named simple type, so read
    /// this through [`Schema::primitive`] rather than directly.
    pub simple_types: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct GlobalElement {
    pub type_name: String,
}

#[derive(Debug, Clone)]
pub struct ComplexType {
    pub name: String,
    pub abstract_: bool,
    pub content: ComplexContent,
}

#[derive(Debug, Clone)]
pub enum ComplexContent {
    Empty,
    Sequence { elements: Vec<Element> },
    Choice { elements: Vec<Element> },
    Extension { base: String, extra: Vec<Element> },
}

#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub type_name: String,
    pub min_occurs: u32,
    pub max_occurs: MaxOccurs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxOccurs {
    Bounded(u32),
    Unbounded,
}

impl MaxOccurs {
    #[must_use]
    pub fn is_array(self) -> bool {
        match self {
            Self::Unbounded => true,
            Self::Bounded(n) => n > 1,
        }
    }
}

impl Schema {
    #[must_use]
    pub fn new() -> Self {
        Self {
            global_elements: HashMap::new(),
            complex_types: HashMap::new(),
            simple_types: HashMap::new(),
        }
    }

    pub fn element(&mut self, name: impl Into<String>, type_name: impl Into<String>) -> &mut Self {
        self.global_elements.insert(
            name.into(),
            GlobalElement {
                type_name: type_name.into(),
            },
        );
        self
    }

    /// Declare a named simple type that restricts `base`.
    pub fn simple(&mut self, name: impl Into<String>, base: impl Into<String>) -> &mut Self {
        self.simple_types.insert(name.into(), base.into());
        self
    }

    pub fn complex(&mut self, name: impl Into<String>, elements: Vec<Element>) -> &mut Self {
        let name = name.into();
        self.complex_types.insert(
            name.clone(),
            ComplexType {
                name,
                abstract_: false,
                content: ComplexContent::Sequence { elements },
            },
        );
        self
    }

    pub fn complex_abstract(
        &mut self,
        name: impl Into<String>,
        elements: Vec<Element>,
    ) -> &mut Self {
        let name = name.into();
        self.complex_types.insert(
            name.clone(),
            ComplexType {
                name,
                abstract_: true,
                content: ComplexContent::Sequence { elements },
            },
        );
        self
    }

    pub fn extend(
        &mut self,
        name: impl Into<String>,
        base: impl Into<String>,
        extra: Vec<Element>,
    ) -> &mut Self {
        let name = name.into();
        self.complex_types.insert(
            name.clone(),
            ComplexType {
                name,
                abstract_: false,
                content: ComplexContent::Extension {
                    base: base.into(),
                    extra,
                },
            },
        );
        self
    }

    #[must_use]
    pub fn global_type(&self, element: &str) -> Option<&str> {
        self.global_elements
            .get(element)
            .map(|g| g.type_name.as_str())
    }

    pub fn flatten<'a>(&'a self, type_name: &str) -> Result<Vec<&'a Element>, super::UciError> {
        let ct = self
            .complex_types
            .get(type_name)
            .ok_or_else(|| super::UciError::UnknownType(type_name.to_owned()))?;
        match &ct.content {
            ComplexContent::Empty => Ok(Vec::new()),
            ComplexContent::Sequence { elements } | ComplexContent::Choice { elements } => {
                Ok(elements.iter().collect())
            }
            ComplexContent::Extension { base, extra } => {
                let mut out = self.flatten(base)?;
                out.extend(extra.iter());
                Ok(out)
            }
        }
    }

    #[must_use]
    pub fn is_complex(&self, type_name: &str) -> bool {
        self.complex_types.contains_key(type_name)
    }

    /// Whether `type_name` holds a scalar value rather than child elements.
    ///
    /// Covers both `xs:` primitives and the schema's own named simple types —
    /// the published catalog defines over nine hundred of the latter, so a
    /// prefix test alone would misread them as complex.
    #[must_use]
    pub fn is_simple(&self, type_name: &str) -> bool {
        type_name.starts_with("xs:") || self.simple_types.contains_key(type_name)
    }

    /// Reduce `type_name` to the `xs:` primitive it ultimately restricts.
    ///
    /// Leaf coercion matches on primitive names to decide whether a value is a
    /// JSON number, boolean, or string, so every named simple type has to be
    /// resolved through its restriction chain first. Returns `type_name`
    /// unchanged when it is already a primitive or is not a known simple type.
    #[must_use]
    pub fn primitive<'a>(&'a self, type_name: &'a str) -> &'a str {
        let mut current = type_name;
        for _ in 0..MAX_SIMPLE_DEPTH {
            if current.starts_with("xs:") {
                return current;
            }
            match self.simple_types.get(current) {
                Some(base) => current = base.as_str(),
                None => return current,
            }
        }
        current
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn el(name: &str, type_name: &str) -> Element {
    Element {
        name: name.into(),
        type_name: type_name.into(),
        min_occurs: 1,
        max_occurs: MaxOccurs::Bounded(1),
    }
}

#[must_use]
pub fn el_opt(name: &str, type_name: &str) -> Element {
    Element {
        name: name.into(),
        type_name: type_name.into(),
        min_occurs: 0,
        max_occurs: MaxOccurs::Bounded(1),
    }
}

#[must_use]
pub fn el_many(name: &str, type_name: &str) -> Element {
    Element {
        name: name.into(),
        type_name: type_name.into(),
        min_occurs: 0,
        max_occurs: MaxOccurs::Unbounded,
    }
}
