//! Bench payloads. Sequence numbers live in the JSON, not in engine headers.

use oa_gateway_testing::fixtures::POSITION_REPORT_JSON;
use serde_json::{json, Value};

/// Which document to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PayloadKind {
    /// Small typed message the fixture schema and the engine both understand.
    Ping,
    /// Realistic UCI document used when `xml_baseline` conversion is on.
    PositionReport,
}

impl PayloadKind {
    #[must_use]
    pub(crate) fn topic(self) -> &'static str {
        match self {
            Self::Ping => "demo",
            Self::PositionReport => "PositionReport",
        }
    }

    #[must_use]
    pub(crate) fn type_hint(self) -> &'static str {
        match self {
            Self::Ping => "Ping",
            Self::PositionReport => "PositionReport",
        }
    }
}

/// Builds a JSON payload with `seq` in `n` and optional padding.
#[must_use]
pub(crate) fn render(kind: PayloadKind, seq: u64, min_bytes: usize) -> String {
    match kind {
        PayloadKind::Ping => {
            let mut from = String::new();
            let mut text = ping_json(seq, &from);
            if text.len() < min_bytes {
                from = "x".repeat(min_bytes.saturating_sub(text.len()));
                text = ping_json(seq, &from);
            }
            text
        }
        PayloadKind::PositionReport => {
            let mut value: Value =
                serde_json::from_str(POSITION_REPORT_JSON).expect("PositionReport fixture is JSON");
            value["PositionReport"]["MessageData"]["n"] = json!(seq);
            let mut text = serde_json::to_string(&value).expect("value is serializable");
            if text.len() < min_bytes {
                let pad = "x".repeat(min_bytes.saturating_sub(text.len()));
                value["PositionReport"]["MessageData"]["pad"] = json!(pad);
                text = serde_json::to_string(&value).expect("value is serializable");
            }
            text
        }
    }
}

fn ping_json(seq: u64, from: &str) -> String {
    if from.is_empty() {
        format!(r#"{{"Ping":{{"n":{seq}}}}}"#)
    } else {
        format!(r#"{{"Ping":{{"n":{seq},"from":"{from}"}}}}"#)
    }
}

/// Reads the sequence number stamped by [`render`].
#[must_use]
pub(crate) fn parse_seq(payload: &[u8]) -> Option<u64> {
    let value: Value = serde_json::from_slice(payload).ok()?;
    value
        .pointer("/Ping/n")
        .or_else(|| value.pointer("/PositionReport/MessageData/n"))
        .and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_round_trips_seq_and_padding() {
        let text = render(PayloadKind::Ping, 42, 64);
        assert!(text.len() >= 64, "{text}");
        assert_eq!(parse_seq(text.as_bytes()), Some(42));
    }

    #[test]
    fn position_report_carries_seq() {
        let text = render(PayloadKind::PositionReport, 7, 1);
        assert_eq!(parse_seq(text.as_bytes()), Some(7));
        assert!(text.contains("PositionReport"));
    }
}
