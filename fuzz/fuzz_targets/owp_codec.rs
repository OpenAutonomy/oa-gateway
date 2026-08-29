//! `oa_gateway_owp::parse_client` on arbitrary text: every client frame
//! (INIT / PUB / SUB / UNSUB) goes through here, and a malformed one must
//! come back as an error rather than a panic or a wrong parse.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oa_gateway_owp::parse_client;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = parse_client(text);
    }
});
