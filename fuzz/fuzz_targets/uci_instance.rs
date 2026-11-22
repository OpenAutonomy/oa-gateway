//! OMS JSON / UCI XML parsing and conversion against the fixture schema:
//! the largest peer-facing parser in the tree. A malformed document must
//! fail cleanly, and a document that parses must convert without
//! panicking or looping. Validation moved to `oa-gateway-xsd-libxml` and
//! is not exercised here — fuzzing a libxml2 FFI boundary through
//! `cargo-fuzz`'s nightly toolchain needs its own sanitizer-instrumented
//! build of libxml2, which this target does not attempt.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oa_gateway_uci::{slice, Message};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let schema = slice::v25();

    if let Ok(msg) = Message::from_json(text, schema) {
        let _ = msg.to_xml(schema);
    }
    if let Ok(msg) = Message::from_xml(text, schema) {
        let _ = msg.to_json(schema);
    }
});
