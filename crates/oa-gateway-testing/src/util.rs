//! Timeouts and markers for cross-adapter tests.
//!
//! [`recv_envelope`] is the loopback counterpart of the OWP/STOMP
//! recv helpers. [`xml_marked`] and [`unique_token`] exist for live
//! ActiveMQ runs, where leftover messages on a shared topic would
//! otherwise match the wrong test.

use std::time::Duration;

use oa_gateway_core::Envelope;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Next envelope, or panic after two seconds / a closed channel.
pub async fn recv_envelope(rx: &mut mpsc::Receiver<Envelope>) -> Envelope {
    timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for envelope")
        .expect("channel closed")
}

/// Rewrites the fixture's `<n>1</n>` so a live broker test can assert
/// it saw *this* send, not a leftover from another run.
#[must_use]
pub fn xml_marked(template: &str, token: &str) -> String {
    template.replace("<n>1</n>", &format!("<n>{token}</n>"))
}

/// `prefix` plus wall-clock nanos. Unique enough on one machine for
/// concurrent live tests; not a UUID.
#[must_use]
pub fn unique_token(prefix: &str) -> String {
    format!(
        "{prefix}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}
