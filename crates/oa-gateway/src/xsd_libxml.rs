//! Compiles the libxml2 validator from `[uci].schema` before any adapter
//! listens — the same timing as [`crate::schema::load`], for the same
//! reason: a bad schema should fail startup, not surface against live
//! traffic.
//!
//! Unlike the hand-rolled compiler (`oa_gateway_uci::xsd::compile`, which
//! takes every spanning document explicitly and does not follow
//! `xs:include`/`xs:import`), libxml2 wants one entry point and resolves
//! includes itself, relative to that file's own path on disk. `uci.schema`'s
//! first entry is treated as that entry point.

use std::sync::Arc;

use oa_gateway_xsd_libxml::XsdLibxmlValidator;
use tracing::warn;

use crate::config::Config;

/// Compiles the libxml2 validator named in `config`, if any.
///
/// Returns `None` when `uci.schema` is empty — the same condition under
/// which [`crate::schema::load`] returns `None`, so the two always agree
/// on whether there is anything to check.
///
/// # Errors
///
/// Returns an error if the first `uci.schema` entry cannot be read or
/// does not compile.
pub(crate) fn load(config: &Config) -> Result<Option<Arc<XsdLibxmlValidator>>, String> {
    let Some(entry) = config.uci.schema.first() else {
        return Ok(None);
    };
    if config.uci.schema.len() > 2 {
        warn!(
            files = config.uci.schema.len(),
            entry = %entry.display(),
            "libxml2 only follows xs:include/xs:import from the first uci.schema entry; \
             additional files will not be covered unless the first one includes them"
        );
    }
    let validator = XsdLibxmlValidator::compile(entry)
        .map_err(|err| format!("cannot compile the UCI schema via libxml2: {err}"))?;
    Ok(Some(Arc::new(validator)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_schema_is_fine_when_nothing_validates() {
        let config: Config = toml::from_str("").unwrap();
        assert!(load(&config).unwrap().is_none());
    }

    #[test]
    fn an_unreadable_schema_path_names_the_file() {
        let config: Config =
            toml::from_str("[uci]\nschema = [\"definitely/not/here.xsd\"]\n").unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("definitely/not/here.xsd"), "{err}");
    }
}
