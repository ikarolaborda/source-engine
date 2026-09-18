#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = source_studio::Mdl::parse(data);
    let _ = source_studio::Vvd::parse(data);
    let _ = source_studio::Vtx::parse(data);
    let _ = source_studio::Phy::parse(data);
});
