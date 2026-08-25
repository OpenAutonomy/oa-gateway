//! Compile the published UCI XSD into a [`Schema`](crate::Schema).
//!
//! This is not a general XSD processor. UCI is written against its own Schema
//! Style & Design Specification, which confines it to a narrow subset of XML
//! Schema: every type is top-level and named, every element declaration refers
//! to a type by name, compositors never nest, and the only derivation is
//! extension. This module accepts that subset and rejects everything else
//! rather than guessing, so a schema revision that starts using a new construct
//! fails loudly instead of quietly losing data.
//!
//! Split into `assemble` (the `compile` entry point and per-document
//! merging), `parse` (turning one XSD node into a schema type),
//! `node_util` (small XML-node helpers shared by both), and
//! `references` (the post-parse check that every type name resolves).
//! Only [`compile`] is public; everything else is an implementation detail
//! of how this module builds a [`Schema`](crate::Schema).

mod assemble;
mod node_util;
mod parse;
mod references;

#[cfg(test)]
mod tests;

pub use assemble::compile;
