//! Isolated cost layers. Each scenario calls public APIs only.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) mod engine;
pub(crate) mod loopback;
pub(crate) mod owp;
pub(crate) mod ping;
pub(crate) mod uci;

/// Waits until `received` stops increasing, or two seconds elapse.
pub(crate) async fn drain_until_quiet(received: &AtomicU64) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut last = received.load(Ordering::Relaxed);
    let mut quiet = 0u8;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let now = received.load(Ordering::Relaxed);
        if now == last {
            quiet += 1;
            if quiet >= 3 {
                break;
            }
        } else {
            quiet = 0;
            last = now;
        }
    }
}
