use std::fmt;

use crate::MAX_DEPTH;

/// A report short enough for a log line or an error frame.
///
/// Names the first few violations and how many were left out, since the first
/// one is usually the one to act on and the count says whether to go looking.
#[must_use]
pub fn summarize(violations: &[Violation]) -> String {
    const SHOWN: usize = 3;
    let shown: Vec<String> = violations
        .iter()
        .take(SHOWN)
        .map(ToString::to_string)
        .collect();
    let mut out = shown.join("; ");
    if violations.len() > SHOWN {
        out.push_str(&format!(" (and {} more)", violations.len() - SHOWN));
    }
    out
}

/// Violations reported for one message before the rest are elided.
///
/// A payload built to produce one violation per element would otherwise be
/// answered with a report as large as itself, which is a poor thing to put in a
/// log line or an error frame.
pub const MAX_VIOLATIONS: usize = 32;

/// One way in which a message departs from the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Dotted path to the offending element, starting at the message name.
    pub path: String,
    pub kind: ViolationKind,
}

/// How a message departed from the schema. Display text is meant for a
/// log line, not a parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    /// An element the type does not declare.
    Undeclared { element: String },
    /// A required element that is absent.
    Missing { element: String },
    /// Present, but fewer times than `minOccurs`.
    TooFew {
        element: String,
        min: u32,
        found: usize,
    },
    /// Present more times than `maxOccurs`.
    TooMany {
        element: String,
        max: u32,
        found: usize,
    },
    /// An alternation with no branch taken.
    NoAlternative { alternatives: Vec<String> },
    /// An alternation with more than one branch taken.
    ManyAlternatives { taken: Vec<String> },
    /// An abstract type used without naming a concrete one.
    AbstractType { type_name: String },
    /// A declared type the schema does not define, or a cyclic chain.
    UnusableType { type_name: String, reason: String },
    /// Nesting past the depth conversion would have refused.
    TooDeep,
    /// A value outside the enumeration its type declares.
    ///
    /// `allowed` is a sample, since some enumerations run to hundreds of values.
    NotEnumerated {
        value: String,
        allowed: Vec<String>,
        total: usize,
    },
    /// A value of the wrong length.
    ///
    /// `unit` is `"characters"` for the string types and `"octets"` for
    /// `xs:hexBinary`, which is the unit XSD's length facet uses on each.
    Length {
        value: String,
        len: usize,
        unit: &'static str,
        requirement: String,
    },
    /// A number outside the bounds its type declares.
    Range { value: String, requirement: String },
    /// A value matching none of the patterns declared at one link of its type's
    /// restriction chain.
    PatternMismatch {
        value: String,
        patterns: Vec<String>,
    },
    /// A value that is not one of the primitive underneath its type at all.
    NotLexical {
        value: String,
        primitive: String,
        expected: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.kind)
    }
}

impl fmt::Display for ViolationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undeclared { element } => {
                write!(f, "'{element}' is not declared by this type")
            }
            Self::Missing { element } => write!(f, "'{element}' is required and absent"),
            Self::TooFew {
                element,
                min,
                found,
            } => write!(f, "'{element}' appears {found} times, minOccurs is {min}"),
            Self::TooMany {
                element,
                max,
                found,
            } => write!(f, "'{element}' appears {found} times, maxOccurs is {max}"),
            Self::NoAlternative { alternatives } => write!(
                f,
                "none of {} is present, and one of them must be",
                quoted(alternatives)
            ),
            Self::ManyAlternatives { taken } => write!(
                f,
                "{} are all present, and they are alternatives",
                quoted(taken)
            ),
            Self::AbstractType { type_name } => write!(
                f,
                "'{type_name}' is abstract, so a concrete type has to be named"
            ),
            Self::UnusableType { type_name, reason } => {
                write!(f, "declared type '{type_name}' cannot be used: {reason}")
            }
            Self::TooDeep => write!(f, "nests deeper than {MAX_DEPTH} elements"),
            Self::NotEnumerated {
                value,
                allowed,
                total,
            } => {
                let sample = quoted(allowed);
                let more = if *total > allowed.len() { ", …" } else { "" };
                write!(
                    f,
                    "'{}' is not one of the {total} values this type allows: {sample}{more}",
                    abbreviated(value)
                )
            }
            Self::Length {
                value,
                len,
                unit,
                requirement,
            } => write!(
                f,
                "'{}' is {len} {unit}, and has to be {requirement}",
                abbreviated(value)
            ),
            Self::Range { value, requirement } => {
                write!(
                    f,
                    "{} is out of range: it has to be {requirement}",
                    abbreviated(value)
                )
            }
            Self::NotLexical {
                value,
                primitive,
                expected,
            } => write!(
                f,
                "'{}' is not a valid {primitive}: expected {expected}",
                abbreviated(value)
            ),
            Self::PatternMismatch { value, patterns } => match patterns.as_slice() {
                [only] => write!(
                    f,
                    "'{}' does not match the pattern '{}'",
                    abbreviated(value),
                    abbreviated(only)
                ),
                many => write!(
                    f,
                    "'{}' does not match any of the {} patterns this type allows",
                    abbreviated(value),
                    many.len()
                ),
            },
        }
    }
}

/// Values arrive from the wire and leave through logs and error frames, so a
/// long one is cut rather than repeated whole.
fn abbreviated(text: &str) -> String {
    const MAX: usize = 60;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    format!("{}…", text.chars().take(MAX).collect::<String>())
}

fn quoted(names: &[String]) -> String {
    names
        .iter()
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(", ")
}
