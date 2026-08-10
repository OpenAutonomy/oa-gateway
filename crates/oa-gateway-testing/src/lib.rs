//! Shared fixtures and test scaffolding.
//!
//! Adapter crates must **not** depend on the `harness` feature — that would
//! cycle (`oa-gateway-testing` → adapter → `oa-gateway-testing`). Unit tests stay in each
//! crate; cross-adapter tests live here. `oa-gateway-uci` uses
//! `default-features = false` for fixtures only.

pub mod fixtures;

#[cfg(feature = "harness")]
pub mod owp;
#[cfg(feature = "harness")]
pub mod stomp;
#[cfg(feature = "harness")]
pub mod util;
