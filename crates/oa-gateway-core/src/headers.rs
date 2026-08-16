//! Gateway-wide envelope header names (`oag.*`).
//!
//! Protocol-owned names stay in their crates (`stomp.*`, `agra.*`).
//! Bridging adapters stamp [`HDR_ORIGIN`] on the way in and call
//! [`crate::Envelope::is_echo_of`] on the way out. The engine never
//! reads these.

/// Which adapter last published this envelope onto the engine.
pub const HDR_ORIGIN: &str = "oag.origin_adapter";
/// Engine topic, copied onto a native frame when the protocol has a
/// place for it.
pub const HDR_TOPIC: &str = "oag.topic";
/// Engine type hint, copied onto a native frame when present.
pub const HDR_TYPE_HINT: &str = "oag.type_hint";
/// Envelope [`crate::MessageId`], written on outbound frames.
pub const HDR_ID: &str = "oag.id";
