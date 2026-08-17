//! Provider shim. The adapter talks only to these traits.

use std::path::Path;

use crate::types::DdsSample;

mod rustdds;

pub use rustdds::RustddsProvider;

/// Joins a domain and returns a session.
pub trait DdsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// # Errors
    ///
    /// Returns [`DdsError`] if the QoS file cannot be used or the
    /// participant cannot join.
    fn join(&self, domain_id: u16, qos_path: &Path) -> Result<Box<dyn DdsSession>, DdsError>;
}

/// One participant: topics, writes, and inbound samples.
pub trait DdsSession: Send {
    /// # Errors
    ///
    /// Returns [`DdsError`] if the topic, reader, or writer cannot be
    /// created.
    fn create_topic(&mut self, name: &str) -> Result<(), DdsError>;

    /// # Errors
    ///
    /// Returns [`DdsError`] if the topic was not created or the write
    /// fails.
    fn write(&self, topic: &str, sample: DdsSample) -> Result<(), DdsError>;

    /// Samples from other participants. Local writes are omitted.
    ///
    /// # Errors
    ///
    /// Returns [`DdsError`] if a reader fails.
    fn poll_inbound(&mut self) -> Result<Vec<(String, DdsSample)>, DdsError>;
}

/// Failure inside a provider. The adapter maps this onto
/// [`oa_gateway_adapter::AdapterError`].
#[derive(Debug, thiserror::Error)]
pub enum DdsError {
    #[error("{0}")]
    Message(String),
}

impl DdsError {
    pub(crate) fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

/// Builds the provider named by `kind`.
#[must_use]
pub fn provider_for(kind: crate::DdsProviderKind) -> Box<dyn DdsProvider> {
    match kind {
        crate::DdsProviderKind::Rustdds => Box::new(RustddsProvider),
    }
}
