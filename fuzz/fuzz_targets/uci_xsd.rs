//! `oa_gateway_uci::xsd::compile` on arbitrary text. A schema is
//! operator-supplied rather than peer-supplied, but a malformed or hostile
//! document still has to fail as an error instead of exhausting the stack
//! or aborting the process at startup.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oa_gateway_uci::xsd;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = xsd::compile(&[text]);
    }
});
