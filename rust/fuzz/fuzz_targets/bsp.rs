#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(header) = source_bsp::Header::parse(data) {
        let _ = header.pakfile_range(data.len() as u64);
        let _ = header.pakfile_range(u64::MAX);
    }
    let _ = source_bsp::Bsp::parse(data);
});
