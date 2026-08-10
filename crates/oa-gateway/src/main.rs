//! Host process: load config, start adapters, wait for Ctrl-C.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oa_gateway_adapter::Adapter;
use oa_gateway_core::Engine;
use oa_gateway_loopback::Loopback;
use oa_gateway_owp::{OwpAdapter, OwpConfig};
use oa_gateway_stomp::{StompAdapter, StompConfig};
use oa_gateway_uci::Schema;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

const USAGE: &str = "\
OA-Gateway — protocol-agnostic routing with pluggable adapters (prototype)

Usage:
  oa-gateway [CONFIG]

Arguments:
  CONFIG           Path to a TOML config file. When omitted, oa-gateway looks for
                   config/default.toml in the current directory and its two
                   parents, and falls back to built-in defaults.

Options:
  -h, --help       Print this help and exit
  -V, --version    Print the version and exit

Examples:
  oa-gateway config/default.toml   OWP on ws://127.0.0.1:9000/ (no broker needed)
  oa-gateway config/asb.toml       adds a STOMP client for ActiveMQ on :61613

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
                return Err(format!("unknown option `{flag}`. Try `oa-gateway --help`."))
            }
            path if config.is_none() => config = Some(PathBuf::from(path)),
            extra => {
                return Err(format!(
                    "unexpected second argument `{extra}`. oa-gateway takes one config path."
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
    uci: UciSection,
    #[serde(default)]
    loopback: LoopbackSection,
    #[serde(default)]
    owp: OwpSection,
    #[serde(default)]
    stomp: StompSection,
}

/// Where to find the UCI schema that drives JSON ↔ XML conversion.
///
/// The standard is not redistributed here, so the documents have to be named
/// explicitly. List every file the schema spans: `UCI_MessageDefinitions` alone
/// leaves the security-marking types dangling, which is reported as an error
/// rather than discovered later against live traffic.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UciSection {
    #[serde(default)]
    schema: Vec<PathBuf>,
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
    #[serde(default = "default_owp_max_frame_size")]
    max_frame_size: usize,
    #[serde(default = "default_owp_max_connections")]
    max_connections: usize,
    #[serde(default = "default_owp_max_subscriptions")]
    max_subscriptions: usize,
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
            max_frame_size: default_owp_max_frame_size(),
            max_connections: default_owp_max_connections(),
            max_subscriptions: default_owp_max_subscriptions(),
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
    "oa-gateway-0".into()
}
fn default_label() -> String {
    "OA-Gateway Prototype".into()
}
fn default_schema() -> String {
    "002.5.0".into()
}
fn default_owp_max_frame_size() -> usize {
    oa_gateway_owp::DEFAULT_MAX_FRAME_SIZE
}
fn default_owp_max_connections() -> usize {
    oa_gateway_owp::DEFAULT_MAX_CONNECTIONS
}
fn default_owp_max_subscriptions() -> usize {
    oa_gateway_owp::DEFAULT_MAX_SUBSCRIPTIONS
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
    oa_gateway_stomp::DEFAULT_MAX_FRAME_SIZE
}

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("oa-gateway: {message}");
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
            println!("oa-gateway {}", env!("CARGO_PKG_VERSION"));
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

    // Compile the schema before anything starts listening, for the same reason
    // addresses are resolved up front: a bad input should fail cleanly rather
    // than after adapters are already accepting traffic.
    let schema = load_schema(&config)?;

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
        let mut adapter = OwpAdapter::new(
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
                max_frame_size: config.owp.max_frame_size,
                max_connections: config.owp.max_connections,
                max_subscriptions: config.owp.max_subscriptions,
            },
        );
        if let Some(schema) = &schema {
            adapter = adapter.with_schema(Arc::clone(schema));
        }
        let adapter = Arc::new(adapter);
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

    info!("oa-gateway running — Ctrl-C to stop");
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

/// Read and compile the configured UCI schema, if one is configured.
///
/// Returns `None` when no schema is listed, which is fine for routing: adapters
/// forward payloads untouched and use the topic as the type hint. It is not fine
/// for `owp.xml_baseline`, which exists only to convert, so that combination is
/// refused here rather than failing per message once traffic is flowing.
fn load_schema(config: &Config) -> Result<Option<Arc<Schema>>, String> {
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
    Ok(Some(Arc::new(schema)))
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
    fn xml_baseline_without_a_schema_is_refused_at_startup() {
        let config: Config =
            toml::from_str("[owp]\nenabled = true\nxml_baseline = true\n").unwrap();
        let err = load_schema(&config).unwrap_err();
        assert!(err.contains("uci.schema"), "{err}");
        assert!(err.contains("xml_baseline"), "{err}");
    }

    /// Routing does not need a schema, so its absence must not block startup.
    #[test]
    fn no_schema_is_fine_when_nothing_converts() {
        let config: Config =
            toml::from_str("[owp]\nenabled = true\nxml_baseline = false\n").unwrap();
        assert!(load_schema(&config).unwrap().is_none());
    }

    #[test]
    fn an_unreadable_schema_path_names_the_file() {
        let config: Config =
            toml::from_str("[uci]\nschema = [\"definitely/not/here.xsd\"]\n").unwrap();
        let err = load_schema(&config).unwrap_err();
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
        let schema = load_schema(&config)
            .unwrap()
            .expect("a schema was configured");
        assert_eq!(schema.global_type("Ping"), Some("PingType"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn explicitly_named_missing_config_is_an_error() {
        let err = load_config(Some(Path::new("definitely/not/here.toml"))).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }
}
