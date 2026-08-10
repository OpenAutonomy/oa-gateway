//! In-memory XSD slice. Enough OMS JSON rules; not a full XSD processor.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Schema {
    pub global_elements: HashMap<String, GlobalElement>,
    pub complex_types: HashMap<String, ComplexType>,
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

    #[must_use]
    pub fn is_simple(type_name: &str) -> bool {
        type_name.starts_with("xs:")
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
