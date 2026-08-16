//! Compiles the UCI schema named in the config before any adapter listens.
//!
//! Conversion and validation both need a compiled schema. Doing that here,
//! once, keeps a bad or incomplete catalog from being discovered against
//! live traffic.

use std::sync::Arc;

use oa_gateway_uci::Schema;
use tracing::{info, warn};

use crate::config::Config;

/// Reads and compiles the UCI schema listed in `config`, if any.
///
/// Returns `None` when no schema is listed, which is fine for routing:
/// adapters forward payloads untouched and use the topic as the type hint.
/// It is not fine for `owp.xml_baseline`, which exists only to convert, so
/// that combination is refused here rather than failing per message once
/// traffic is flowing.
///
/// After a successful compile, types this build cannot check are logged as
/// warnings. A constraint that cannot be read enforces nothing, and nothing
/// downstream can tell that from a value that passed.
///
/// # Errors
///
/// Returns an error if `owp.xml_baseline` is set with no schema files, a
/// listed file cannot be read, or the documents do not compile.
pub(crate) fn load(config: &Config) -> Result<Option<Arc<Schema>>, String> {
    if config.uci.schema.is_empty() {
        if config.owp.enabled && config.owp.xml_baseline {
            return Err(
                "owp.xml_baseline needs a UCI schema, but uci.schema lists no files. \
                 Point it at the schema documents (UCI_MessageDefinitions and \
                 UCI_SecurityMarkings), or set owp.xml_baseline = false."
                    .into(),
            );
        }
        return Ok(None);
    }

    let mut texts = Vec::with_capacity(config.uci.schema.len());
    for path in &config.uci.schema {
        texts
            .push(std::fs::read_to_string(path).map_err(|err| {
                format!("cannot read uci.schema entry {}: {err}", path.display())
            })?);
    }
    let documents: Vec<&str> = texts.iter().map(String::as_str).collect();

    let schema = oa_gateway_uci::xsd::compile(&documents)
        .map_err(|err| format!("cannot compile the UCI schema: {err}"))?;
    info!(
        files = documents.len(),
        messages = schema.global_elements.len(),
        complex_types = schema.complex_types.len(),
        simple_types = schema.simple_types.len(),
        "uci schema compiled"
    );

    // A constraint this build cannot read enforces nothing. Nothing downstream
    // can tell that from a value that passed, so the one place to say so is here,
    // where an operator is still reading startup output.
    let primitives = schema.unchecked_primitives();
    if !primitives.is_empty() {
        warn!(
            primitives = primitives.join(", "),
            "values of these types will not be checked beyond the facets on them"
        );
    }
    let unchecked = schema.unchecked_patterns();
    if !unchecked.is_empty() {
        warn!(
            count = unchecked.len(),
            types = unchecked
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", "),
            "some schema patterns cannot be checked and will not be enforced"
        );
    }

    Ok(Some(Arc::new(schema)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_baseline_without_a_schema_is_refused_at_startup() {
        let config: Config =
            toml::from_str("[owp]\nenabled = true\nxml_baseline = true\n").unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("uci.schema"), "{err}");
        assert!(err.contains("xml_baseline"), "{err}");
    }

    /// Routing does not need a schema, so its absence must not block startup.
    #[test]
    fn no_schema_is_fine_when_nothing_converts() {
        let config: Config =
            toml::from_str("[owp]\nenabled = true\nxml_baseline = false\n").unwrap();
        assert!(load(&config).unwrap().is_none());
    }

    #[test]
    fn an_unreadable_schema_path_names_the_file() {
        let config: Config =
            toml::from_str("[uci]\nschema = [\"definitely/not/here.xsd\"]\n").unwrap();
        let err = load(&config).unwrap_err();
        assert!(err.contains("definitely/not/here.xsd"), "{err}");
    }

    #[test]
    fn a_configured_schema_is_compiled() {
        let dir = std::env::temp_dir().join("oa-gateway-schema-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mini.xsd");
        std::fs::write(
            &path,
            r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"
                          xmlns:uci="urn:example" targetNamespace="urn:example">
                 <xs:element name="Ping" type="uci:PingType"/>
                 <xs:complexType name="PingType">
                   <xs:sequence><xs:element name="n" type="xs:int"/></xs:sequence>
                 </xs:complexType>
               </xs:schema>"#,
        )
        .unwrap();

        let config: Config = toml::from_str(&format!(
            "[uci]\nschema = [{:?}]\n",
            path.display().to_string()
        ))
        .unwrap();
        let schema = load(&config).unwrap().expect("a schema was configured");
        assert_eq!(schema.global_type("Ping"), Some("PingType"));

        std::fs::remove_file(&path).ok();
    }
}
