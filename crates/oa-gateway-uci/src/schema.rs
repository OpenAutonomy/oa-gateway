//! In-memory model of a UCI schema. Enough for the OMS JSON rules; not a full
//! XSD processor.
//!
//! Build one by hand with the builder methods below, or compile the published
//! XSD into one with [`crate::xsd::compile`].

use std::cmp::Ordering;
use std::collections::HashMap;

/// How many links of a named-simple-type chain [`Schema::primitive`] will follow
/// before giving up. The published schema nests three deep at most; the limit
/// exists so a cyclic hand-built schema cannot hang the caller.
const MAX_SIMPLE_DEPTH: usize = 16;

#[derive(Debug, Clone)]
pub struct Schema {
    pub global_elements: HashMap<String, GlobalElement>,
    pub complex_types: HashMap<String, ComplexType>,
    /// Named simple types: what each one restricts, and how. The target is
    /// usually an `xs:` primitive but may be another named simple type, so read
    /// it through [`Schema::primitive`] rather than directly, and read the
    /// constraints through [`Schema::effective_facets`].
    pub simple_types: HashMap<String, SimpleType>,
}

/// A named simple type: the type it restricts, and the facets it adds.
#[derive(Debug, Clone)]
pub struct SimpleType {
    pub base: String,
    pub facets: Facets,
}

/// Constraints a simple type places on a value, as written.
///
/// Read [`Schema::effective_facets`] instead of a single type's facets: a
/// restriction chain spreads them over several links.
#[derive(Debug, Clone, Default)]
pub struct Facets {
    /// Permitted values. Empty means unconstrained rather than "nothing allowed".
    pub enumeration: Vec<String>,
    /// Patterns as the XSD wrote them. Several at one link are alternatives.
    pub patterns: Vec<String>,
    pub length: Option<usize>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    /// Numeric bounds, held as `f64`. Every bound in the published catalog is
    /// small enough to be exact; a bound past 2^53 on an `xs:long` would not be,
    /// and is worth revisiting if a program's message set carries one.
    pub min_inclusive: Option<f64>,
    pub max_inclusive: Option<f64>,
    pub min_exclusive: Option<f64>,
    pub max_exclusive: Option<f64>,
}

/// The facets in force for a type, gathered along its restriction chain.
///
/// A derived type's own enumeration is the operative one, since XSD requires it
/// to be a subset of its base's. Patterns at different links all have to hold.
/// For a length or a bound, the tightest wins.
#[derive(Debug, Default)]
pub struct Effective<'a> {
    pub enumeration: Option<&'a [String]>,
    pub patterns: Vec<&'a str>,
    pub length: Option<usize>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub min_inclusive: Option<f64>,
    pub max_inclusive: Option<f64>,
    pub min_exclusive: Option<f64>,
    pub max_exclusive: Option<f64>,
}

impl Effective<'_> {
    /// Whether anything here can be violated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.enumeration.is_none()
            && self.patterns.is_empty()
            && self.length.is_none()
            && self.min_length.is_none()
            && self.max_length.is_none()
            && self.min_inclusive.is_none()
            && self.max_inclusive.is_none()
            && self.min_exclusive.is_none()
            && self.max_exclusive.is_none()
    }
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
    /// The compositors declared directly on the type. A type with no content
    /// model has none.
    Groups(Vec<Group>),
    Extension {
        base: String,
        extra: Vec<Group>,
    },
}

/// A run of element declarations under one compositor.
///
/// Kept apart from the flat list of declarations because the compositor is the
/// difference between siblings and alternatives, and only one of those can be
/// checked by counting.
#[derive(Debug, Clone)]
pub struct Group {
    pub kind: GroupKind,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    /// Members stand on their own, each governed by its own occurrence range.
    Sequence,
    /// Members are alternatives to one another.
    Choice,
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

    /// Declare a named simple type that restricts `base` without narrowing it.
    pub fn simple(&mut self, name: impl Into<String>, base: impl Into<String>) -> &mut Self {
        self.simple_with(name, base, Facets::default())
    }

    /// Declare a named simple type that restricts `base` with `facets`.
    pub fn simple_with(
        &mut self,
        name: impl Into<String>,
        base: impl Into<String>,
        facets: Facets,
    ) -> &mut Self {
        self.simple_types.insert(
            name.into(),
            SimpleType {
                base: base.into(),
                facets,
            },
        );
        self
    }

    pub fn complex(&mut self, name: impl Into<String>, elements: Vec<Element>) -> &mut Self {
        self.complex_groups(name, vec![sequence(elements)])
    }

    /// Declare a type from explicit compositors, for a choice or a mix of both.
    pub fn complex_groups(&mut self, name: impl Into<String>, groups: Vec<Group>) -> &mut Self {
        let name = name.into();
        self.complex_types.insert(
            name.clone(),
            ComplexType {
                name,
                abstract_: false,
                content: ComplexContent::Groups(groups),
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
                content: ComplexContent::Groups(vec![sequence(elements)]),
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
                    extra: vec![sequence(extra)],
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

    /// Every element declaration a type contributes, base types included.
    ///
    /// Errors on a cyclic extension chain rather than following it. Nothing in
    /// the published schema is cyclic, but a schema is an input like any other:
    /// it can come from a program-specific Message Set, and a chain that closes
    /// on itself would otherwise recurse until the stack ran out, at startup or
    /// on the first message that touched the type.
    pub fn flatten<'a>(&'a self, type_name: &str) -> Result<Vec<&'a Element>, super::UciError> {
        Ok(self
            .groups(type_name)?
            .into_iter()
            .flat_map(|g| g.elements.iter())
            .collect())
    }

    /// The compositors a type is built from, base types first.
    ///
    /// [`Self::flatten`] answers which elements may appear; this also answers
    /// under what compositor, which is what tells a set of optional siblings
    /// apart from a set of alternatives.
    pub fn groups<'a>(&'a self, type_name: &str) -> Result<Vec<&'a Group>, super::UciError> {
        self.groups_chain(type_name, &mut Vec::new())
    }

    fn groups_chain<'a>(
        &'a self,
        type_name: &str,
        chain: &mut Vec<String>,
    ) -> Result<Vec<&'a Group>, super::UciError> {
        if chain.iter().any(|seen| seen == type_name) {
            chain.push(type_name.to_owned());
            return Err(super::UciError::Xsd(format!(
                "cyclic extension chain: {}",
                chain.join(" -> ")
            )));
        }
        let ct = self
            .complex_types
            .get(type_name)
            .ok_or_else(|| super::UciError::UnknownType(type_name.to_owned()))?;
        match &ct.content {
            ComplexContent::Groups(groups) => Ok(groups.iter().collect()),
            ComplexContent::Extension { base, extra } => {
                chain.push(type_name.to_owned());
                let mut out = self.groups_chain(base, chain)?;
                chain.pop();
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
                Some(simple) => current = simple.base.as_str(),
                None => return current,
            }
        }
        current
    }

    /// Every constraint a value of `type_name` has to satisfy.
    ///
    /// Walks the restriction chain, so a type that narrows another inherits what
    /// the other already required. An `xs:` primitive, or a type the schema does
    /// not define, constrains nothing.
    #[must_use]
    pub fn effective_facets<'a>(&'a self, type_name: &str) -> Effective<'a> {
        let mut out = Effective::default();
        let mut current = type_name;
        for _ in 0..MAX_SIMPLE_DEPTH {
            let Some(simple) = self.simple_types.get(current) else {
                break;
            };
            let facets = &simple.facets;
            if out.enumeration.is_none() && !facets.enumeration.is_empty() {
                out.enumeration = Some(&facets.enumeration);
            }
            out.patterns
                .extend(facets.patterns.iter().map(String::as_str));
            out.length = out.length.or(facets.length);
            out.min_length = stricter(out.min_length, facets.min_length, Ordering::Greater);
            out.max_length = stricter(out.max_length, facets.max_length, Ordering::Less);
            out.min_inclusive = stricter_f64(out.min_inclusive, facets.min_inclusive, f64::max);
            out.max_inclusive = stricter_f64(out.max_inclusive, facets.max_inclusive, f64::min);
            out.min_exclusive = stricter_f64(out.min_exclusive, facets.min_exclusive, f64::max);
            out.max_exclusive = stricter_f64(out.max_exclusive, facets.max_exclusive, f64::min);
            current = simple.base.as_str();
        }
        out
    }
}

/// Keep whichever bound is harder to satisfy.
fn stricter<T: Ord>(a: Option<T>, b: Option<T>, keep: Ordering) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.cmp(&b) == keep { a } else { b }),
        (some, None) | (None, some) => some,
    }
}

fn stricter_f64(a: Option<f64>, b: Option<f64>, keep: fn(f64, f64) -> f64) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(keep(a, b)),
        (some, None) | (None, some) => some,
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

/// One compositor whose members stand on their own.
#[must_use]
pub fn sequence(elements: Vec<Element>) -> Group {
    Group {
        kind: GroupKind::Sequence,
        elements,
    }
}

/// One compositor whose members are alternatives.
#[must_use]
pub fn choice(elements: Vec<Element>) -> Group {
    Group {
        kind: GroupKind::Choice,
        elements,
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
