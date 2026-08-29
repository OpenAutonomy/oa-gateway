//! OMS JSON / UCI XML parsing and conversion against the fixture schema:
//! the largest peer-facing parser in the tree. A malformed document must
//! fail cleanly, and a document that parses must convert and validate
//! without panicking or looping.

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
        let _ = msg.violations(schema);
    }
    if let Ok(msg) = Message::from_xml(text, schema) {
        let _ = msg.to_json(schema);
        let _ = msg.violations(schema);
    }
});
