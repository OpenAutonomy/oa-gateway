use std::net::SocketAddr;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct StompConfig {
    pub broker: SocketAddr,
    /// STOMP `host` header. ActiveMQ Classic typically wants `/`.
    pub host: String,
    pub login: Option<String>,
    pub passcode: Option<String>,
    pub destination_prefix: String,
    /// Engine topics (and STOMP dest suffixes) to bridge both ways.
    pub topics: Vec<String>,
    pub unwrap_ma_payloads: bool,
    pub reconnect: bool,
    pub connect_timeout: Duration,
}

impl Default for StompConfig {
    fn default() -> Self {
        Self {
            broker: "127.0.0.1:61613".parse().expect("static addr"),
            host: "/".into(),
            login: None,
            passcode: None,
            destination_prefix: "/topic/".into(),
            topics: vec!["demo".into()],
            unwrap_ma_payloads: true,
            reconnect: true,
            connect_timeout: Duration::from_secs(5),
        }
    }
}
