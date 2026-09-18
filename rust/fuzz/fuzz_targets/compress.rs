#![no_main]

use libfuzzer_sys::fuzz_target;
use source_compress::{actual_size, compress, decompress, is_compressed, is_snappy};

fuzz_target!(|data: &[u8]| {
    // Arbitrary bytes as a compressed stream: the decoder must refuse them
    // rather than read outside what it has decoded so far.
    let _ = is_compressed(data);
    let _ = is_snappy(data);
    let _ = actual_size(data);
    let _ = decompress(data);

    // A stream carrying the tag gets much further into the decoder than one
    // that does not, so the tag is forced on rather than left to chance.
    if data.len() >= 4 {
        let mut tagged = data.to_vec();
        tagged[..4].copy_from_slice(b"LZSS");
        let _ = decompress(&tagged);
    }

    // Whatever the encoder accepts has to come back unchanged, and has to
    // describe its own length correctly.
    if let Ok(compressed) = compress(data) {
        assert_eq!(actual_size(&compressed), Some(data.len()));
        assert_eq!(decompress(&compressed).as_deref(), Ok(data));

        // Truncating a valid stream anywhere must be refused, never read
        // past the end.
        for cut in 0..compressed.len() {
            let _ = decompress(&compressed[..cut]);
        }
    }
});
