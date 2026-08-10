//! Host process: load config, start adapters, wait for Ctrl-C.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mpg_adapter::Adapter;
use mpg_core::Engine;
use mpg_loopback::Loopback;
use mpg_owp::{OwpAdapter, OwpConfig};
use mpg_stomp::{StompAdapter, StompConfig};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

const USAGE: &str = "\
mpg — multi-protocol gateway (prototype)

Usage:
  mpg [CONFIG]

Arguments:
  CONFIG           Path to a TOML config file. When omitted, mpg looks for
                   config/default.toml in the current directory and its two
                   parents, and falls back to built-in defaults.

Options:
  -h, --help       Print this help and exit
  -V, --version    Print the version and exit

Examples:
  mpg config/default.toml   OWP on ws://127.0.0.1:9000/ (no broker needed)
  mpg config/asb.toml       adds a STOMP client for ActiveMQ on :61613

Set RUST_LOG to change log filtering (default: info).
";

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run(Option<PathBuf>),
    Help,
    Version,
}

fn parse_args<I>(args: I) -> Result<Cli, String>
where
    I: IntoIterator<Item = String>,
{
    let mut config = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}`. Try `mpg --help`."))
            }
            path if config.is_none() => config = Some(PathBuf::from(path)),
            extra => {
                return Err(format!(
                    "unexpected second argument `{extra}`. mpg takes one config path."
                ))
            }
        }
    }
    Ok(Cli::Run(config))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    loopback: LoopbackSection,
    #[serde(default)]
    owp: OwpSection,
    #[serde(default)]
    stomp: StompSection,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoopbackSection {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_loopback_id")]
    id: String,
}

impl Default for LoopbackSection {
    fn default() -> Self {
        Self {
            enabled: true,
            id: default_loopback_id(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwpSection {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_owp_id")]
    id: String,
    #[serde(default = "default_bind")]
    bind: String,
    #[serde(default = "default_server_id")]
    server_id: String,
    #[serde(default = "default_label")]
    system_label: String,
    #[serde(default = "default_schema")]
    schema: String,
    #[serde(default = "default_true")]
    unwrap_ma_payloads: bool,
    #[serde(default)]
    xml_baseline: bool,
}

impl Default for OwpSection {
    fn default() -> Self {
        Self {
            enabled: true,
            id: default_owp_id(),
            bind: default_bind(),
            server_id: default_server_id(),
            system_label: default_label(),
            schema: default_schema(),
            unwrap_ma_payloads: true,
            xml_baseline: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StompSection {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_stomp_id")]
    id: String,
    #[serde(default = "default_stomp_broker")]
    broker: String,
    #[serde(default = "default_stomp_host")]
    host: String,
    #[serde(default)]
    login: String,
    #[serde(default)]
    passcode: String,
    #[serde(default = "default_stomp_prefix")]
    destination_prefix: String,
    #[serde(default = "default_stomp_topics")]
    topics: Vec<String>,
    #[serde(default = "default_true")]
    unwrap_ma_payloads: bool,
    #[serde(default = "default_true")]
    reconnect: bool,
    #[serde(default = "default_stomp_max_frame_size")]
    max_frame_size: usize,
}

impl Default for StompSection {
    fn default() -> Self {
        Self {
            enabled: false,
            id: default_stomp_id(),
            broker: default_stomp_broker(),
            host: default_stomp_host(),
            login: String::new(),
            passcode: String::new(),
            destination_prefix: default_stomp_prefix(),
            topics: default_stomp_topics(),
            unwrap_ma_payloads: true,
            reconnect: true,
            max_frame_size: default_stomp_max_frame_size(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_loopback_id() -> String {
    "loopback".into()
}
fn default_owp_id() -> String {
    "owp".into()
}
fn default_bind() -> String {
    "127.0.0.1:9000".into()
}
fn default_server_id() -> String {
    "mpg-0".into()
}
fn default_label() -> String {
    "MPG Prototype".into()
}
fn default_schema() -> String {
    "002.5.0".into()
}
fn default_stomp_id() -> String {
    "stomp".into()
}
fn default_stomp_broker() -> String {
    "127.0.0.1:61613".into()
}
fn default_stomp_host() -> String {
    "/".into()
}
fn default_stomp_prefix() -> String {
    "/topic/".into()
}
fn default_stomp_topics() -> Vec<String> {
    vec!["demo".into()]
}
fn default_stomp_max_frame_size() -> usize {
    mpg_stomp::DEFAULT_MAX_FRAME_SIZE
}

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("mpg: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    match parse_args(std::env::args().skip(1))? {
        Cli::Help => {
            print!("{USAGE}");
            Ok(())
        }
        Cli::Version => {
            println!("mpg {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Cli::Run(path) => serve(path.as_deref()).await,
    }
}

async fn serve(config_path: Option<&Path>) -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = load_config(config_path)?;

    // Resolve every address before starting anything, so a bad one fails cleanly
    // instead of leaving the earlier adapters already running.
    let owp_bind = if config.owp.enabled {
        Some(resolve_addr("owp.bind", &config.owp.bind).await?)
    } else {
        None
    };
    let stomp_broker = if config.stomp.enabled {
        Some(resolve_addr("stomp.broker", &config.stomp.broker).await?)
    } else {
        None
    };

    let engine = Arc::new(Engine::new());
    let shutdown = CancellationToken::new();

    let mut handles = Vec::new();

    if config.loopback.enabled {
        let adapter = Arc::new(Loopback::new(engine.clone(), config.loopback.id.clone()));
        info!(id = %adapter.id(), "starting loopback adapter");
        let engine = Arc::clone(&engine);
        let token = shutdown.clone();
        handles.push(tokio::spawn(async move {
            if let Err(err) = adapter.run(engine, token).await {
                error!(error = %err, "loopback adapter failed");
            }
        }));
    }

    if let Some(bind) = owp_bind {
        let adapter = Arc::new(OwpAdapter::new(
            config.owp.id.clone(),
            OwpConfig {
                bind,
                server_id: config.owp.server_id.clone(),
                system_label: config.owp.system_label.clone(),
                schema: if config.owp.schema.is_empty() {
                    None
                } else {
                    Some(config.owp.schema.clone())
                },
                system_uuid: uuid::Uuid::new_v4().to_string(),
                unwrap_ma_payloads: config.owp.unwrap_ma_payloads,
                xml_baseline: config.owp.xml_baseline,
            },
        ));
        info!(id = %adapter.id(), bind = %bind, "starting owp adapter");
        let engine = Arc::clone(&engine);
        let token = shutdown.clone();
        handles.push(tokio::spawn(async move {
            if let Err(err) = adapter.run(engine, token).await {
                error!(error = %err, "owp adapter failed");
            }
        }));
    }

    if let Some(broker) = stomp_broker {
        let login = if config.stomp.login.is_empty() {
            None
        } else {
            Some(config.stomp.login.clone())
        };
        let passcode = if config.stomp.passcode.is_empty() {
            None
        } else {
            Some(config.stomp.passcode.clone())
        };
        let adapter = Arc::new(StompAdapter::new(
            config.stomp.id.clone(),
            StompConfig {
                broker,
                host: config.stomp.host.clone(),
                login,
                passcode,
                destination_prefix: config.stomp.destination_prefix.clone(),
                topics: config.stomp.topics.clone(),
                unwrap_ma_payloads: config.stomp.unwrap_ma_payloads,
                reconnect: config.stomp.reconnect,
                connect_timeout: std::time::Duration::from_secs(5),
                max_frame_size: config.stomp.max_frame_size,
            },
        ));
        info!(id = %adapter.id(), %broker, "starting stomp adapter");
        let engine = Arc::clone(&engine);
        let token = shutdown.clone();
        handles.push(tokio::spawn(async move {
            if let Err(err) = adapter.run(engine, token).await {
                error!(error = %err, "stomp adapter failed");
            }
        }));
    }

    if handles.is_empty() {
        return Err(
            "no adapters enabled. Set enabled = true under [loopback], [owp], or [stomp].".into(),
        );
    }

    info!("mpg running — Ctrl-C to stop");
    tokio::signal::ctrl_c()
        .await
        .map_err(|err| format!("cannot listen for Ctrl-C: {err}"))?;
    info!("shutdown requested");
    shutdown.cancel();
    for handle in handles {
        let _ = handle.await;
    }
    Ok(())
}

/// Accept a literal socket address or a `host:port` needing a lookup, so
/// `localhost:9000` works as naturally as `127.0.0.1:9000`.
///
/// IPv4 wins when a name offers both, because `localhost` resolves to `::1` first
/// on some hosts and binding there alone would refuse the documented
/// `127.0.0.1:9000` clients.
async fn resolve_addr(key: &str, value: &str) -> Result<SocketAddr, String> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let candidates: Vec<SocketAddr> = tokio::net::lookup_host(value)
        .await
        .map_err(|err| {
            format!("{key} = \"{value}\" is not a usable address ({err}). Expected host:port, such as 127.0.0.1:9000.")
        })?
        .collect();
    let addr = candidates
        .iter()
        .find(|addr| addr.is_ipv4())
        .or_else(|| candidates.first())
        .copied()
        .ok_or_else(|| format!("{key} = \"{value}\" resolved to no addresses"))?;
    info!(%key, value, %addr, "resolved address");
    Ok(addr)
}

fn load_config(path: Option<&Path>) -> Result<Config, String> {
    let path = match path {
        // A file the user named explicitly is a mistake when missing, not a cue
        // to quietly run some other configuration.
        Some(named) if !named.exists() => {
            return Err(format!("config file {} not found", named.display()))
        }
        Some(named) => named.to_path_buf(),
        None => match find_default_config() {
            Some(found) => found,
            None => {
                info!("no config/default.toml found, using built-in defaults");
                return Ok(Config::default());
            }
        },
    };

    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let config = toml::from_str(&text).map_err(|err| format!("in {}: {err}", path.display()))?;
    info!(path = %path.display(), "config loaded");
    Ok(config)
}

fn find_default_config() -> Option<PathBuf> {
    [
        "config/default.toml",
        "../config/default.toml",
        "../../config/default.toml",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|candidate| candidate.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn help_and_version_flags_are_recognized() {
        assert_eq!(parse_args(args(&["--help"])).unwrap(), Cli::Help);
        assert_eq!(parse_args(args(&["-h"])).unwrap(), Cli::Help);
        assert_eq!(parse_args(args(&["--version"])).unwrap(), Cli::Version);
        assert_eq!(parse_args(args(&["-V"])).unwrap(), Cli::Version);
    }

    #[test]
    fn config_path_is_optional() {
        assert_eq!(parse_args(args(&[])).unwrap(), Cli::Run(None));
        assert_eq!(
            parse_args(args(&["config/asb.toml"])).unwrap(),
            Cli::Run(Some(PathBuf::from("config/asb.toml")))
        );
    }

    #[test]
    fn unknown_flag_is_not_taken_as_a_config_path() {
        let err = parse_args(args(&["--verbose"])).unwrap_err();
        assert!(err.contains("--verbose"), "{err}");
    }

    #[test]
    fn second_positional_argument_is_rejected() {
        let err = parse_args(args(&["a.toml", "b.toml"])).unwrap_err();
        assert!(err.contains("b.toml"), "{err}");
    }

    #[test]
    fn misspelled_config_key_is_rejected() {
        let err = toml::from_str::<Config>("[stomp]\ntopic = [\"PositionReport\"]\n").unwrap_err();
        assert!(err.to_string().contains("topic"), "{err}");
    }

    #[test]
    fn known_config_keys_still_parse() {
        let config: Config =
            toml::from_str("[stomp]\nenabled = true\ntopics = [\"PositionReport\"]\n").unwrap();
        assert!(config.stomp.enabled);
        assert_eq!(config.stomp.topics, ["PositionReport"]);
    }

    /// Guards the shipped configs against drifting from the structs above, which
    /// `deny_unknown_fields` would otherwise turn into a startup failure.
    #[test]
    fn shipped_configs_parse() {
        for name in ["default.toml", "asb.toml"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../config")
                .join(name);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            toml::from_str::<Config>(&text).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
    }

    #[test]
    fn explicitly_named_missing_config_is_an_error() {
        let err = load_config(Some(Path::new("definitely/not/here.toml"))).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }
}
