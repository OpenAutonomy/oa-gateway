use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use crate::codec::DEFAULT_MAX_FRAME_SIZE;

/// What to do when a STOMP session task panics.
///
/// The session always runs on a child task so a panic is a join
/// error, not an unwind of the retry loop.
/// [`Self::Abort`] ends `run`. [`Self::Reconnect`] treats the panic as
/// a failed session and then follows [`StompConfig::reconnect`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnPanic {
    /// End the adapter. Default so a bug stays visible.
    #[default]
    Abort,
    /// Retry if [`StompConfig::reconnect`] is on.
    Reconnect,
}

impl fmt::Display for OnPanic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Abort => "abort",
            Self::Reconnect => "reconnect",
        })
    }
}

impl FromStr for OnPanic {
    type Err = String;

    /// `abort` or `reconnect`.
    ///
    /// # Errors
    ///
    /// Returns a message if `s` is neither.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "abort" => Ok(Self::Abort),
            "reconnect" => Ok(Self::Reconnect),
            other => Err(format!(
                "unknown on_panic '{other}'; expected abort or reconnect"
            )),
        }
    }
}

/// Runtime settings for a STOMP client session.
///
/// This is not the host TOML section. `broker` is already a
/// [`SocketAddr`]; the host resolves the hostname before constructing
/// this. There is no `enabled` flag here — spawning is the host's
/// decision.
///
/// [`Self::login`] and [`Self::passcode`] are `None` to omit those
/// CONNECT headers, not to send blanks. [`Self::passcode`] is sent only
/// when [`Self::login`] is set.
#[derive(Debug, Clone)]
pub struct StompConfig {
    pub broker: SocketAddr,
    /// STOMP `host` header. ActiveMQ Classic typically wants `/`.
    pub host: String,
    /// CONNECT `login`. `None` omits the header.
    pub login: Option<String>,
    /// CONNECT `passcode`. Sent only when [`Self::login`] is `Some`.
    pub passcode: Option<String>,
    /// Prepended to each topic to form a STOMP destination. A missing
    /// trailing slash is added by [`crate::DestinationMap`].
    pub destination_prefix: String,
    /// Engine topics (and STOMP dest suffixes) to bridge both ways.
    pub topics: Vec<String>,
    /// Peel A-GRA Rx/Tx hex wrappers on inbound MESSAGE frames and
    /// publish wrapper plus inner as two envelopes.
    pub unwrap_ma_payloads: bool,
    /// Retry the broker after a dropped session or a session `Err`.
    pub reconnect: bool,
    /// Sleep between reconnect attempts.
    pub reconnect_delay: Duration,
    /// Budget for TCP connect and for the CONNECTED wait, each.
    pub connect_timeout: Duration,
    /// Skip outbound SEND when the envelope is an echo of this adapter.
    pub suppress_echo: bool,
    /// Panic in a session task: abort `run`, or treat as a failed
    /// session.
    pub on_panic: OnPanic,
    /// Largest frame accepted from the broker. Bounds the read buffer
    /// and the `content-length` a peer can claim.
    pub max_frame_size: usize,
}

impl Default for StompConfig {
    /// Local ActiveMQ defaults: `127.0.0.1:61613`, `host` `/`,
    /// `/topic/` + `demo`, unwrap and reconnect on, echo suppressed,
    /// panic aborts, one-second reconnect delay, five-second
    /// timeouts, [`DEFAULT_MAX_FRAME_SIZE`].
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
            reconnect_delay: Duration::from_secs(1),
            connect_timeout: Duration::from_secs(5),
            suppress_echo: true,
            on_panic: OnPanic::Abort,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_panic_reads_back() {
        assert_eq!(
            OnPanic::Abort.to_string().parse::<OnPanic>().unwrap(),
            OnPanic::Abort
        );
        assert_eq!(
            OnPanic::Reconnect.to_string().parse::<OnPanic>().unwrap(),
            OnPanic::Reconnect
        );
        assert!(OnPanic::from_str("die").is_err());
    }
}
