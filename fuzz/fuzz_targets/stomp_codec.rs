//! `oa_gateway_stomp::decode_one_with_limit` on arbitrary bytes: it must
//! never panic, hang, or over-allocate on a hostile frame. This is the
//! decoder that had the `content-length` panic bug.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oa_gateway_stomp::decode_one_with_limit;

fuzz_target!(|data: &[u8]| {
    // A small cap keeps the size-limit paths cheap to explore, and drains
    // the buffer frame by frame the way the client's read loop does.
    let mut buf = data.to_vec();
    while let Ok(Some(_frame)) = decode_one_with_limit(&mut buf, 64 * 1024) {
        if buf.is_empty() {
            break;
        }
    }
});
