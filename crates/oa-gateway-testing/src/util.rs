use std::time::Duration;

use oa_gateway_core::Envelope;
use tokio::sync::mpsc;
use tokio::time::timeout;

pub async fn recv_envelope(rx: &mut mpsc::Receiver<Envelope>) -> Envelope {
    timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for envelope")
        .expect("channel closed")
}

#[must_use]
pub fn xml_marked(template: &str, token: &str) -> String {
    template.replace("<n>1</n>", &format!("<n>{token}</n>"))
}

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
