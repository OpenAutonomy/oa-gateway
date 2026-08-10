//! Protocol-agnostic routing engine.
//!
//! The core never parses a payload and never names a protocol. Adapters map
//! native frames onto [`Envelope`] values and subscribe through [`Engine`].

mod engine;
mod envelope;
mod ids;
mod route;

pub use engine::{
    Delivery, Engine, EngineError, EngineStats, PublishOutcome, SubscriberKey,
    DEFAULT_CHANNEL_CAPACITY,
};
pub use envelope::{ContentType, Envelope};
pub use ids::{AdapterId, MessageId, SubId};
pub use route::RouteKey;
