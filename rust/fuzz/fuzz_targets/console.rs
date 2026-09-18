#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(script) = std::str::from_utf8(data) {
        let mut console = source_console::Console::new();
        let _ = console.enqueue(script);
        while console.pop().is_some() {}
    }
});
