//! Checking a converted message against the schema it claims to follow.
//!
//! Conversion and validation answer different questions. Conversion asks
//! whether a payload can be mapped between OMS JSON and UCI XML, and it
//! is deliberately forgiving: an element it cannot place is carried as a
//! string, and an alternation is mapped as though its branches were
//! siblings. That is what makes the gateway useful before a program's
//! message set is fully understood, and it is also why a payload can
//! convert cleanly and still not be a valid instance of the standard.
//!
//! What is checked here is what the compiled schema actually states:
//! every element is declared, required elements are present, occurrence
//! ranges hold, exactly one branch of an alternation is taken, no
//! abstract type is instantiated without naming a concrete one, leaves
//! fit their `xs:` primitive, and facets (enumerations, patterns,
//! lengths, ranges) hold. A primitive this build does not check
//! (`xs:base64Binary`, `xs:anyURI`, `xs:QName`) and a pattern that
//! does not translate are reported on the schema, not on each message.
//!
//! Every violation is reported rather than the first, up to
//! [`MAX_VIOLATIONS`], because an operator comparing a producer against
//! the standard wants the list, not a bisection.
//!
//! Split into `mode` (the [`Mode`] enum), `violation` (the report
//! types), and `rules` (the walk that produces them) — this module
//! re-exports the public surface of all three so callers keep using
//! `oa_gateway_uci::validate::{...}` as one flat path.

mod mode;
mod rules;
mod violation;

#[cfg(test)]
mod tests;

pub use mode::Mode;
pub use rules::validate;
pub use violation::{summarize, Violation, ViolationKind, MAX_VIOLATIONS};
