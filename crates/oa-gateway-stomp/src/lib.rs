//! STOMP 1.2 client adapter for ActiveMQ Classic (and any STOMP broker).
//!
//! This is a client, not a server, and it is not JMS. Java CAL peers
//! still speak OpenWire; ActiveMQ routes the same topic between
//! OpenWire and STOMP when destinations match. The engine sees only
//! [`oa_gateway_core::Envelope`] values. This crate maps
//! `/topic/{name}` onto [`oa_gateway_core::RouteKey::topic`] and stamps
//! `oag.*` headers so a MESSAGE is not SENDed back to the broker that
//! just delivered it.
//!
//! [`StompAdapter::serve`] (and [`oa_gateway_adapter::Adapter::run`])
//! connect, subscribe the configured topics both ways, and retry while
//! [`StompConfig::reconnect`] is set. A panic in a session is isolated
//! on a child task; [`StompConfig::on_panic`] chooses abort or
//! reconnect. The host does not restart a finished `run`.
//!
//! The codec ([`Frame`], [`decode_one`]) is public so tests can speak
//! STOMP without this adapter.

mod adapter;
mod client;
mod codec;
mod config;
mod dest;

pub use adapter::StompAdapter;
pub use codec::{decode_one, decode_one_with_limit, CodecError, Frame, DEFAULT_MAX_FRAME_SIZE};
pub use config::StompConfig;
pub use dest::{
    inbound_route, sniff_content_type, sniff_type_hint, DestinationMap, HDR_ID, HDR_ORIGIN,
    HDR_STOMP_DEST, HDR_TOPIC, HDR_TYPE_HINT,
};
pub use oa_gateway_adapter::OnPanic;
