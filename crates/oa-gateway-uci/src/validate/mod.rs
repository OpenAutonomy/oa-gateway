//! Whether an adapter checks a converted message against its schema at all.
//!
//! The actual check moved to `oa-gateway-xsd-libxml`: a hand-rolled walk of
//! the converted instance tree checked presence, cardinality, and facets,
//! but not element order within an `xs:sequence` (JSON-origin messages
//! come through `serde_json`'s `BTreeMap`-backed `Value::Object`, i.e.
//! alphabetical order, not declaration order) — a real XSD validator
//! rejects that, so libxml2 replaced it outright rather than running
//! alongside it. [`Mode`] is the one piece every adapter config still
//! holds directly: whether validation runs at all, and whether a
//! violation refuses the message or just gets logged.

mod mode;

pub use mode::Mode;
