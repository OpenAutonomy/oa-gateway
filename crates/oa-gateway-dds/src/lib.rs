//! DDS adapter. Samples are A-GRA Rx/Tx; streams are DDS topic names.
//!
//! The adapter talks to a [`DdsProvider`]. This build ships
//! [`RustddsProvider`]. A later vendor stack is another type behind
//! the same trait. QoS comes from a file the provider interprets.
//!
//! Engine topic equals DDS topic. The engine sees envelopes only.

mod adapter;
mod config;
mod convert;
mod provider;
mod qos_xml;
mod types;

pub use adapter::DdsAdapter;
pub use config::{DdsConfig, DdsProviderKind, DEFAULT_MAX_SAMPLE_SIZE};
pub use provider::{provider_for, DdsError, DdsProvider, DdsSession, RustddsProvider};
pub use qos_xml::{load as load_qos, parse as parse_qos, QosSpec};
pub use types::{DdsSample, MaDataPayload, TYPE_NAME};
