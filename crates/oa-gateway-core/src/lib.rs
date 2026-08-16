//! Protocol-agnostic routing engine.
//!
//! The core never parses a payload and never names a protocol. Adapters
//! map native frames onto [`Envelope`] values and subscribe through
//! [`Engine`]. Matching is a [`RouteKey`] (topic plus optional type
//! hint). Fan-out is `try_send`: a slow subscriber loses the message
//! rather than blocking the publisher.
//!
//! Adding a protocol means adding an adapter crate, not changing this
//! one. The plugin contract is `oa_gateway_adapter::Adapter`; the
//! walkthrough is `docs/writing-an-adapter.md`.

mod engine;
mod envelope;
mod headers;
mod ids;
mod route;

pub use engine::{
    Delivery, Engine, EngineError, EngineStats, PublishOutcome, SubscriberKey,
    DEFAULT_CHANNEL_CAPACITY,
};
pub use envelope::{ContentType, Envelope};
pub use headers::{HDR_ID, HDR_ORIGIN, HDR_TOPIC, HDR_TYPE_HINT};
pub use ids::{AdapterId, MessageId, SubId};
pub use route::RouteKey;
