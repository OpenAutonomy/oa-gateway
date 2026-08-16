//! `[uci]` section: schema documents and what to do when a payload is not one.
//!
//! The standard is not redistributed here. Conversion and validation both
//! need the files named explicitly, and both documents the catalog spans.

use std::path::PathBuf;

use oa_gateway_uci::ValidateMode;
use serde::Deserialize;

/// Where to find the UCI schema that drives JSON ↔ XML conversion.
///
/// The standard is not redistributed here, so the documents have to be
/// named explicitly. List every file the schema spans:
/// `UCI_MessageDefinitions` alone leaves the security-marking types
/// dangling, which is reported as an error rather than discovered later
/// against live traffic.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UciSection {
    /// XSD paths, relative to the process working directory.
    ///
    /// Empty (the default) means no conversion and no validation. That is
    /// fine for routing and refused when `owp.xml_baseline` is on.
    #[serde(default)]
    pub(crate) schema: Vec<PathBuf>,
    /// What to do about a payload that does not follow the schema: `"off"`,
    /// `"warn"`, or `"reject"`.
    ///
    /// Stored as a string so a typo is refused with `uci.validate: …`
    /// rather than a generic serde error. Ignored when no schema is
    /// loaded. Defaults to `"warn"`.
    #[serde(default = "default_validate")]
    pub(crate) validate: String,
}

// Written out rather than derived: a derived Default would leave `validate`
// empty, so a config with no [uci] section at all would be refused for naming a
// mode it never named.
impl Default for UciSection {
    fn default() -> Self {
        Self {
            schema: Vec::new(),
            validate: default_validate(),
        }
    }
}

impl UciSection {
    /// Parses [`Self::validate`] into a [`ValidateMode`].
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not `off`, `warn`, or `reject`.
    pub(crate) fn validate_mode(&self) -> Result<ValidateMode, String> {
        self.validate
            .parse()
            .map_err(|err| format!("uci.validate: {err}"))
    }
}

fn default_validate() -> String {
    ValidateMode::default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn a_validation_mode_is_read_from_the_config_or_refused_by_name() {
        let config: Config = toml::from_str("[uci]\nvalidate = \"reject\"\n").unwrap();
        assert_eq!(config.uci.validate_mode().unwrap(), ValidateMode::Reject);

        // Loading a schema is an opt-in to the UCI layer; reporting on it is the
        // default once that choice is made.
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.uci.validate_mode().unwrap(), ValidateMode::Warn);

        let config: Config = toml::from_str("[uci]\nvalidate = \"strict\"\n").unwrap();
        let err = config.uci.validate_mode().unwrap_err();
        assert!(err.contains("uci.validate"), "{err}");
        assert!(err.contains("off, warn, or reject"), "{err}");
    }
}
