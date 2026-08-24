//! Shared panic and retry supervision for an adapter whose `run` is a loop
//! of independent sessions (connect, bridge, disconnect; repeat).
//!
//! An adapter with this shape spawns each session on its own child task, so
//! a panic inside one is a [`tokio::task::JoinError`] rather than an unwind
//! that would otherwise take the retry loop down with it. [`after_join`]
//! is the pure decision at the center of that loop: given how a session
//! ended and what this adapter's config says to do about it, it says
//! whether to stop, fail, or try again.

use tracing::error;

use oa_gateway_core::AdapterId;

use crate::AdapterError;

/// What an adapter does when its session task panics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnPanic {
    /// End the adapter. Default so a bug stays visible.
    #[default]
    Abort,
    /// Treat the panic as a failed session, then follow the adapter's own
    /// reconnect setting.
    Reconnect,
}

impl std::fmt::Display for OnPanic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Abort => "abort",
            Self::Reconnect => "reconnect",
        })
    }
}

impl std::str::FromStr for OnPanic {
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

/// What a session retry loop does after one session task ends.
#[derive(Debug)]
pub enum AfterSession {
    /// Shut down cleanly.
    ReturnOk,
    /// Shut down with this fatal error.
    ReturnErr(AdapterError),
    /// Sleep, then start another session. Carries a message worth logging.
    Retry { message: String },
}

/// Maps a joined session result onto abort, return, or retry.
///
/// `joined` is the outcome of `tokio::spawn(session).await`: `Ok(Ok(()))`
/// on a clean session end, `Ok(Err(_))` on a session that failed without
/// panicking, and `Err(_)` when the session task panicked or was
/// cancelled.
#[must_use]
pub fn after_join(
    joined: Result<Result<(), AdapterError>, tokio::task::JoinError>,
    reconnect: bool,
    on_panic: OnPanic,
    adapter: &AdapterId,
) -> AfterSession {
    match joined {
        Ok(Ok(())) => {
            if reconnect {
                AfterSession::Retry {
                    message: "session ended, reconnecting".into(),
                }
            } else {
                AfterSession::ReturnOk
            }
        }
        Ok(Err(err)) => {
            if reconnect {
                AfterSession::Retry {
                    message: format!("session failed, retrying: {err}"),
                }
            } else {
                AfterSession::ReturnErr(err)
            }
        }
        Err(join) if join.is_panic() => {
            error!(adapter = %adapter, "adapter session panicked");
            match (on_panic, reconnect) {
                (OnPanic::Abort, _) | (OnPanic::Reconnect, false) => {
                    AfterSession::ReturnErr(AdapterError::failed(adapter, "session panicked"))
                }
                (OnPanic::Reconnect, true) => AfterSession::Retry {
                    message: "session panicked, retrying".into(),
                },
            }
        }
        Err(_) => AfterSession::ReturnOk,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn panic_aborts_even_when_reconnect_is_on() {
        let joined = tokio::spawn(async { panic!("session boom") }).await;
        match after_join(joined, true, OnPanic::Abort, &AdapterId::new("test")) {
            AfterSession::ReturnErr(err) => {
                assert!(err.to_string().contains("session panicked"), "{err}");
            }
            other => panic!("expected abort, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn panic_retries_when_on_panic_is_reconnect() {
        let joined = tokio::spawn(async { panic!("session boom") }).await;
        match after_join(joined, true, OnPanic::Reconnect, &AdapterId::new("test")) {
            AfterSession::Retry { message } => {
                assert!(message.contains("panicked"), "{message}");
            }
            other => panic!("expected retry, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn panic_reconnect_still_stops_when_reconnect_is_off() {
        let joined = tokio::spawn(async { panic!("session boom") }).await;
        match after_join(joined, false, OnPanic::Reconnect, &AdapterId::new("test")) {
            AfterSession::ReturnErr(_) => {}
            other => panic!("expected err, got {other:?}"),
        }
    }

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
        assert!("die".parse::<OnPanic>().is_err());
    }
}
