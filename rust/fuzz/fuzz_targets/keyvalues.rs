#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = source_keyvalues::parse_bytes(data, source_keyvalues::ParseOptions::default());
});
