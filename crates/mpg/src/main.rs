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

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default)]
    loopback: LoopbackSection,
    #[serde(default)]
    owp: OwpSection,
    #[serde(default)]
    stomp: StompSection,
}

#[derive(Debug, Deserialize)]
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

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let config = load_config(&config_path);

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

    if config.owp.enabled {
        let bind: SocketAddr = config
            .owp
            .bind
            .parse()
            .unwrap_or_else(|err| panic!("invalid owp.bind {}: {err}", config.owp.bind));
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

    if config.stomp.enabled {
        let broker: SocketAddr =
            config.stomp.broker.parse().unwrap_or_else(|err| {
                panic!("invalid stomp.broker {}: {err}", config.stomp.broker)
            });
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
        error!("no adapters enabled");
        return;
    }

    info!("mpg running — Ctrl-C to stop");
    tokio::signal::ctrl_c().await.expect("ctrl-c handler");
    info!("shutdown requested");
    shutdown.cancel();
    for handle in handles {
        let _ = handle.await;
    }
}

fn default_config_path() -> PathBuf {
    let candidates = [
        PathBuf::from("config/default.toml"),
        PathBuf::from("../config/default.toml"),
        PathBuf::from("../../config/default.toml"),
    ];
    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("config/default.toml"))
}

fn load_config(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
            panic!("failed to parse {}: {err}", path.display());
        }),
        Err(err) => {
            info!(
                path = %path.display(),
                error = %err,
                "config not found, using defaults"
            );
            Config {
                loopback: LoopbackSection::default(),
                owp: OwpSection::default(),
                stomp: StompSection::default(),
            }
        }
    }
}
