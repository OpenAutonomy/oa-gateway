//! STOMP 1.2 client adapter for ActiveMQ Classic (and any STOMP broker).
//!
//! This is not JMS. Java CAL peers still speak OpenWire; ActiveMQ routes the
//! same topic between OpenWire and STOMP when destinations match.

mod adapter;
mod client;
mod codec;
mod config;
mod dest;

pub use adapter::StompAdapter;
pub use codec::{decode_one, CodecError, Frame};
pub use config::StompConfig;
pub use dest::{
    inbound_route, sniff_content_type, sniff_type_hint, DestinationMap, HDR_ID, HDR_ORIGIN,
    HDR_STOMP_DEST, HDR_TOPIC, HDR_TYPE_HINT,
};
