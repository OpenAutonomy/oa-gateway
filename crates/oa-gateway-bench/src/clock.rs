//! Sequence → send-time map shared by a publisher and its subscribers.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Wall origin plus per-sequence offsets in nanoseconds.
pub(crate) struct SeqClock {
    origin: Instant,
    sent_ns: Mutex<HashMap<u64, u64>>,
}

impl SeqClock {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
            sent_ns: Mutex::new(HashMap::new()),
        }
    }

    /// Records `seq` as sent now. Returns the offset used.
    pub(crate) fn stamp(&self, seq: u64) -> u64 {
        let ns = self.origin.elapsed().as_nanos() as u64;
        self.sent_ns.lock().expect("seq clock").insert(seq, ns);
        ns
    }

    /// Nanoseconds from stamp to now, if `seq` was recorded.
    #[must_use]
    pub(crate) fn latency_ns(&self, seq: u64) -> Option<u64> {
        let sent = *self.sent_ns.lock().expect("seq clock").get(&seq)?;
        Some(self.origin.elapsed().as_nanos() as u64 - sent)
    }

    /// Offset of `seq`, if recorded.
    #[must_use]
    pub(crate) fn sent_ns(&self, seq: u64) -> Option<u64> {
        self.sent_ns.lock().expect("seq clock").get(&seq).copied()
    }
}
