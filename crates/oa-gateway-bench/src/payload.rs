//! Bench payloads. Sequence numbers live in `<n>`, not in engine headers.

use oa_gateway_testing::fixtures::POSITION_REPORT_XML;
use oa_gateway_testing::util::xml_marked;

/// Which document to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PayloadKind {
    /// Small typed message the fixture validator and the engine both understand.
    Ping,
    /// Larger, more realistic UCI document.
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

/// Builds an XML payload with `seq` in `<n>` and optional padding.
#[must_use]
pub(crate) fn render(kind: PayloadKind, seq: u64, min_bytes: usize) -> String {
    match kind {
        PayloadKind::Ping => {
            let mut text = ping_xml(seq, "");
            if text.len() < min_bytes {
                let pad = "x".repeat(min_bytes.saturating_sub(text.len()));
                text = ping_xml(seq, &pad);
            }
            text
        }
        PayloadKind::PositionReport => {
            let mut text = xml_marked(POSITION_REPORT_XML, &seq.to_string());
            if text.len() < min_bytes {
                let pad = "x".repeat(min_bytes.saturating_sub(text.len()));
                text = text.replace("</MessageData>", &format!("<pad>{pad}</pad></MessageData>"));
            }
            text
        }
    }
}

fn ping_xml(seq: u64, from: &str) -> String {
    const NS: &str = "https://www.vdl.afrl.af.mil/programs/oam";
    if from.is_empty() {
        format!(r#"<Ping xmlns="{NS}"><n>{seq}</n></Ping>"#)
    } else {
        format!(r#"<Ping xmlns="{NS}"><n>{seq}</n><from>{from}</from></Ping>"#)
    }
}

/// Reads the sequence number stamped by [`render`]: the first `<n>`
/// element's text, parsed as `u64`.
#[must_use]
pub(crate) fn parse_seq(payload: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(payload).ok()?;
    let doc = roxmltree::Document::parse(text).ok()?;
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "n")
        .and_then(|n| n.text())
        .and_then(|t| t.parse().ok())
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

    #[test]
    fn position_report_padding_still_parses() {
        let text = render(PayloadKind::PositionReport, 9, 4096);
        assert!(text.len() >= 4096, "{text}");
        assert_eq!(parse_seq(text.as_bytes()), Some(9));
    }
}
