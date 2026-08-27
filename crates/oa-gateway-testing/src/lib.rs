//! Shared fixtures and test scaffolding.
//!
//! This is not a runtime crate. Adapter crates must **not** depend on
//! the `harness` feature — that would cycle
//! (`oa-gateway-testing` → adapter → `oa-gateway-testing`). Unit tests
//! stay in each crate; anything that crosses the engine lives here.
//!
//! `harness` is on by default and pulls OWP, STOMP, DDS, and loopback.
//! [`fixtures`] is always available. `oa-gateway-uci` depends with
//! `default-features = false` so it can compile against the sample
//! payloads without taking the adapters.

pub mod fixtures;

#[cfg(feature = "harness")]
pub mod dds;
#[cfg(feature = "harness")]
pub mod owp;
#[cfg(feature = "harness")]
pub mod stomp;
#[cfg(feature = "harness")]
pub mod tls;
#[cfg(feature = "harness")]
pub mod util;
