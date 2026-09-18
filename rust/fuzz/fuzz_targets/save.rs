#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = source_save::SaveContainer::parse(data);
    let _ = source_save::MapState::parse(data);
});
