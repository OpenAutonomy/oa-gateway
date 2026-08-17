//! On-wire sample for rustdds: one type that carries either Rx or Tx.
//!
//! DDS gives each topic a single type. The A-GRA pair is therefore one
//! struct with a kind discriminator. `encoded` is the inner UCI payload
//! as octets, not hex text.

use bytes::Bytes;
use oa_gateway_agra::{WrapperKind, WrapperMeta};
use serde::{Deserialize, Serialize};

/// Provider-neutral sample the adapter reads and writes.
///
/// `encoded` is the inner UCI payload as octets. The provider maps this
/// onto its on-wire type; the adapter never sees CDR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsSample {
    /// Wrapper kind and A-GRA metadata lifted off Rx/Tx.
    pub meta: WrapperMeta,
    /// Inner UCI bytes, not hex text.
    pub encoded: Bytes,
}

/// rustdds / CDR representation of [`DdsSample`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaDataPayload {
    pub kind: String,
    pub message_type: String,
    pub encoded: Vec<u8>,
    pub originator_uuid: String,
    pub rx_payload_id: String,
    pub command_id: String,
    pub destination_routing: String,
}

/// Type name passed to `create_topic`.
pub const TYPE_NAME: &str = "MaDataPayload";

impl MaDataPayload {
    #[must_use]
    pub fn from_sample(sample: &DdsSample) -> Self {
        Self {
            kind: match sample.meta.kind {
                WrapperKind::Rx => "rx".into(),
                WrapperKind::Tx => "tx".into(),
            },
            message_type: sample.meta.message_type_enum.clone(),
            encoded: sample.encoded.to_vec(),
            originator_uuid: sample.meta.originator_uuid.clone().unwrap_or_default(),
            rx_payload_id: sample.meta.rx_payload_id.clone().unwrap_or_default(),
            command_id: sample.meta.command_id.clone().unwrap_or_default(),
            destination_routing: sample.meta.destination_routing.clone().unwrap_or_default(),
        }
    }

    /// # Errors
    ///
    /// Returns a message if `kind` is not `rx` or `tx`.
    pub fn into_sample(self) -> Result<DdsSample, String> {
        let kind = match self.kind.as_str() {
            "rx" => WrapperKind::Rx,
            "tx" => WrapperKind::Tx,
            other => return Err(format!("unknown wrapper kind {other}")),
        };
        Ok(DdsSample {
            meta: WrapperMeta {
                kind,
                message_type_enum: self.message_type,
                originator_uuid: nonempty(self.originator_uuid),
                rx_payload_id: nonempty(self.rx_payload_id),
                command_id: nonempty(self.command_id),
                destination_routing: nonempty(self.destination_routing),
            },
            encoded: Bytes::from(self.encoded),
        })
    }
}

fn nonempty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
