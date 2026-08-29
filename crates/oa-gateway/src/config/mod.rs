//! Host TOML: one section per adapter, plus the UCI schema the host compiles.
//!
//! Unknown keys are refused so a typo fails at startup instead of being
//! ignored. A missing section is not an error: each one deserializes as
//! its [`Default`].

use std::path::Path;

use serde::Deserialize;
use tracing::info;

mod dds;
mod engine;
mod loopback;
mod owp;
mod stomp;
mod uci;

pub(crate) use dds::DdsSection;
pub(crate) use engine::EngineSection;
pub(crate) use loopback::LoopbackSection;
pub(crate) use owp::OwpSection;
pub(crate) use stomp::StompSection;
pub(crate) use uci::UciSection;

/// Root document of a host config file.
///
/// Every section is optional in the file. An adapter table that is
/// present is on; one that is omitted is off. `enabled = false` keeps
/// the keys without spawning.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    /// How often the host logs engine counters. Omitting the section
    /// uses a 30-second interval.
    #[serde(default)]
    pub(crate) engine: EngineSection,
    /// Schema files and validation mode compiled before any adapter listens.
    #[serde(default)]
    pub(crate) uci: UciSection,
    /// In-process adapter. On when the `[loopback]` table is present.
    #[serde(default)]
    pub(crate) loopback: LoopbackSection,
    /// OWP/WebSocket server. On when the `[owp]` table is present.
    #[serde(default)]
    pub(crate) owp: OwpSection,
    /// STOMP client. On when the `[stomp]` table is present.
    #[serde(default)]
    pub(crate) stomp: StompSection,
    /// DDS participant. On when the `[dds]` table is present.
    #[serde(default)]
    pub(crate) dds: DdsSection,
}

/// Serde default for fields that must be true when omitted.
///
/// `#[serde(default)]` on a `bool` uses `false`. These fields cannot use that.
pub(crate) fn default_true() -> bool {
    true
}

/// Reads and parses the TOML file at `path`.
///
/// A path the user named is a mistake when missing, not a cue to quietly
/// run some other configuration.
///
/// # Errors
///
/// Returns an error if the file does not exist, cannot be read, or is not
/// a [`Config`] — including when it names a key no section declares.
pub(crate) fn load(path: &Path) -> Result<Config, String> {
    // A path the user named is a mistake when missing, not a cue to quietly run
    // some other configuration.
    if !path.exists() {
        return Err(format!("config file {} not found", path.display()));
    }

    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let config = toml::from_str(&text).map_err(|err| format!("in {}: {err}", path.display()))?;
    info!(path = %path.display(), "config loaded");
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn misspelled_config_key_is_rejected() {
        let err = toml::from_str::<Config>("[stomp]\ntopic = [\"PositionReport\"]\n").unwrap_err();
        assert!(err.to_string().contains("topic"), "{err}");
    }

    #[test]
    fn known_config_keys_still_parse() {
        let config: Config = toml::from_str("[stomp]\ntopics = [\"PositionReport\"]\n").unwrap();
        assert!(config.stomp.enabled);
        assert_eq!(config.stomp.topics, ["PositionReport"]);
    }

    #[test]
    fn a_present_adapter_section_is_on() {
        let empty: Config = toml::from_str("").unwrap();
        assert!(!empty.loopback.enabled);
        assert!(!empty.owp.enabled);
        assert!(!empty.stomp.enabled);
        assert!(!empty.dds.enabled);

        let named: Config =
            toml::from_str("[owp]\n[stomp]\n[dds]\nqos = \"config/dds-qos.xml\"\n").unwrap();
        assert!(!named.loopback.enabled);
        assert!(named.owp.enabled);
        assert!(named.stomp.enabled);
        assert!(named.dds.enabled);

        let held: Config = toml::from_str("[owp]\nenabled = false\n").unwrap();
        assert!(!held.owp.enabled);
    }

    /// Guards the shipped configs against drifting from the structs above, which
    /// `deny_unknown_fields` would otherwise turn into a startup failure.
    #[test]
    fn shipped_configs_parse() {
        for name in ["default.toml", "asb.toml", "compose.toml", "dds.toml"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../config")
                .join(name);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            toml::from_str::<Config>(&text).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }

    #[test]
    fn engine_and_stomp_runtime_knobs_parse() {
        let config: Config = toml::from_str(
            "[engine]\nstats_interval_secs = 0\n[stomp]\non_panic = \"reconnect\"\nsuppress_echo = false\n",
        )
        .unwrap();
        assert_eq!(config.engine.stats_interval_secs, 0);
        assert!(!config.stomp.suppress_echo);
        assert_eq!(
            config.stomp.on_panic_mode().unwrap(),
            oa_gateway_stomp::OnPanic::Reconnect
        );

        let config: Config = toml::from_str("[stomp]\non_panic = \"die\"\n").unwrap();
        let err = config.stomp.on_panic_mode().unwrap_err();
        assert!(err.contains("stomp.on_panic"), "{err}");
    }

    #[test]
    fn a_debug_of_the_config_does_not_leak_the_stomp_passcode() {
        let config: Config =
            toml::from_str("[stomp]\nlogin = \"user\"\npasscode = \"s3cr3t-passphrase\"\n")
                .unwrap();
        assert_eq!(config.stomp.passcode.expose(), "s3cr3t-passphrase");
        let shown = format!("{config:?}");
        assert!(!shown.contains("s3cr3t-passphrase"), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
    }

    #[test]
    fn dds_provider_and_qos_are_checked() {
        let config: Config =
            toml::from_str("[dds]\nprovider = \"rustdds\"\nqos = \"config/dds-qos.xml\"\n")
                .unwrap();
        assert!(config.dds.enabled);
        assert_eq!(
            config.dds.provider_kind().unwrap(),
            oa_gateway_dds::DdsProviderKind::Rustdds
        );

        let config: Config = toml::from_str("[dds]\nprovider = \"cyclone\"\n").unwrap();
        let err = config.dds.provider_kind().unwrap_err();
        assert!(err.contains("dds.provider"), "{err}");
    }

    #[test]
    fn a_missing_config_is_an_error() {
        let err = load(Path::new("definitely/not/here.toml")).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }
}
