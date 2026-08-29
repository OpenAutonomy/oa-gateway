//! A-GRA Rx/Tx wrapper detection and unwrapping on arbitrary bytes. The
//! wrapper fields were once located by substring search, which could anchor
//! to the wrong place on adversarial input; this exercises the roxmltree
//! and JSON paths that replaced it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oa_gateway_agra::{unwrap, wrapper_kind, xml_root_local_name};

fuzz_target!(|data: &[u8]| {
    let _ = wrapper_kind(data);
    let _ = unwrap("demo", data);
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = xml_root_local_name(text);
    }
});
