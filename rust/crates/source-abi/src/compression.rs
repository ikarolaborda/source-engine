//! Allocation-neutral entry points for the engine's compressed-buffer path.

use super::*;

/// Worst-case storage for a Source-tagged Snappy stream.
///
/// # Safety
/// `out_size` must point to writable u64 storage.
#[no_mangle]
pub unsafe extern "C" fn source_compress_snappy_max_size(
    input_length: u64,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_size.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_size, 0) };
        let Some(size) = usize::try_from(input_length)
            .ok()
            .and_then(source_compress::snappy::max_compressed_size)
        else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        unsafe { ptr::write(out_size, size as u64) };
        SOURCE_ABI_OK
    })
}

/// Compresses with the Source SNAP tag, even when compression grows the data.
/// BUFFER_TOO_SMALL reports required length without writing any output bytes.
///
/// # Safety
/// Input is readable, output writable for capacity bytes and non-overlapping
/// with input. out_length is writable u64 storage. Null output is allowed only
/// with zero capacity (including a size query).
#[no_mangle]
pub unsafe extern "C" fn source_compress_snappy_compress(
    input: SourceAbiSlice,
    out_bytes: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_length.is_null() || (capacity != 0 && out_bytes.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_length, 0) };
        if checked_len(capacity).is_err() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let Ok(input) = (unsafe { read_slice(input) }) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        match source_compress::snappy::compress(input) {
            Ok(output) => unsafe { write_bytes_out(&output, out_bytes, capacity, out_length) },
            Err(_) => SOURCE_ABI_INVALID_ARGUMENT,
        }
    })
}

fn compressed_size(input: &[u8]) -> Result<Option<usize>, SourceAbiStatus> {
    if input.starts_with(b"SNAP") {
        source_compress::snappy::actual_size(input)
            .map(Some)
            .map_err(|_| SOURCE_ABI_FORMAT_ERROR)
    } else if input.starts_with(b"LZSS") {
        source_compress::actual_size(input)
            .map(Some)
            .ok_or(SOURCE_ABI_FORMAT_ERROR)
    } else {
        Ok(None)
    }
}

/// Reads a SNAP/LZSS length prefix, without validating payload tokens. DECLINED
/// means untagged data; a present but malformed tag is FORMAT_ERROR.
///
/// # Safety
/// Input must be readable and out_size writable u64 storage.
#[no_mangle]
pub unsafe extern "C" fn source_compress_buffer_actual_size(
    input: SourceAbiSlice,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_size.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_size, 0) };
        let Ok(input) = (unsafe { read_slice(input) }) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        match compressed_size(input) {
            Ok(Some(length)) => {
                unsafe { ptr::write(out_size, length as u64) };
                SOURCE_ABI_OK
            }
            Ok(None) => SOURCE_ABI_DECLINED,
            Err(status) => status,
        }
    })
}

/// Decodes SNAP/LZSS or copies untagged bytes. Tagged corruption never falls
/// back to raw copying. Capacity is checked before any decompression allocation;
/// failed calls do not modify output bytes. With valid output arguments,
/// out_length is zero on errors other than BUFFER_TOO_SMALL, when it reports
/// the header's required size.
///
/// # Safety
/// Same memory contract as source_compress_snappy_compress.
#[no_mangle]
pub unsafe extern "C" fn source_compress_buffer_decompress(
    input: SourceAbiSlice,
    out_bytes: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_length.is_null() || (capacity != 0 && out_bytes.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_length, 0) };
        let Ok(capacity) = checked_len(capacity) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        let Ok(input) = (unsafe { read_slice(input) }) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        let size = match compressed_size(input) {
            Ok(size) => size,
            Err(status) => return status,
        };
        let length = size.unwrap_or(input.len());
        if length > capacity {
            unsafe { ptr::write(out_length, length as u64) };
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        if size.is_none() {
            return unsafe { write_bytes_out(input, out_bytes, capacity as u64, out_length) };
        }
        let output = if input.starts_with(b"SNAP") {
            source_compress::snappy::decompress(input, capacity)
                .map_err(|_| SOURCE_ABI_FORMAT_ERROR)
        } else {
            source_compress::decompress(input).map_err(|_| SOURCE_ABI_FORMAT_ERROR)
        };
        match output {
            Ok(output) => unsafe {
                write_bytes_out(&output, out_bytes, capacity as u64, out_length)
            },
            Err(status) => status,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(bytes: &[u8]) -> SourceAbiSlice {
        SourceAbiSlice {
            data: bytes.as_ptr(),
            length: bytes.len() as u64,
        }
    }

    #[test]
    fn buffer_dispatch_capacity_and_corruption_are_transactional() {
        let input = vec![b'a'; 4096];
        let snappy = source_compress::snappy::compress(&input).unwrap();
        let lzss = source_compress::compress(&input).unwrap();
        unsafe {
            for bytes in [&input, &snappy, &lzss] {
                let mut length = 999;
                let mut output = vec![0x5a; input.len() + 1];
                assert_eq!(
                    source_compress_buffer_decompress(
                        slice(bytes),
                        output.as_mut_ptr(),
                        3,
                        &mut length
                    ),
                    SOURCE_ABI_BUFFER_TOO_SMALL
                );
                assert_eq!(length, input.len() as u64);
                assert!(output.iter().all(|byte| *byte == 0x5a));
                assert_eq!(
                    source_compress_buffer_decompress(
                        slice(bytes),
                        output.as_mut_ptr(),
                        input.len() as u64,
                        &mut length
                    ),
                    SOURCE_ABI_OK
                );
                assert_eq!(&output[..input.len()], input);
                assert_eq!(output[input.len()], 0x5a);
            }
            for bytes in [
                b"SNAP".as_slice(),
                b"LZSS",
                b"SNAP\x04\x01\x00",
                b"LZSS\x04\0\0\0",
            ] {
                let mut output = [0x5a; 16];
                let mut length = 999;
                assert_eq!(
                    source_compress_buffer_decompress(
                        slice(bytes),
                        output.as_mut_ptr(),
                        16,
                        &mut length
                    ),
                    SOURCE_ABI_FORMAT_ERROR
                );
                assert_eq!(length, 0);
                assert_eq!(output, [0x5a; 16]);
            }
        }
    }

    #[test]
    fn abi_empty_inputs_queries_and_argument_validation() {
        unsafe {
            let empty = SourceAbiSlice {
                data: ptr::null(),
                length: 0,
            };
            let mut length = 999;
            assert_eq!(
                source_compress_buffer_decompress(empty, ptr::null_mut(), 0, &mut length),
                SOURCE_ABI_OK
            );
            assert_eq!(length, 0);
            assert_eq!(
                source_compress_buffer_actual_size(empty, &mut length),
                SOURCE_ABI_DECLINED
            );
            assert_eq!(
                source_compress_snappy_compress(empty, ptr::null_mut(), 0, &mut length),
                SOURCE_ABI_BUFFER_TOO_SMALL
            );
            assert_eq!(length, 5);
            assert_eq!(
                source_compress_buffer_decompress(
                    slice(b"SNAP\0"),
                    ptr::null_mut(),
                    0,
                    &mut length
                ),
                SOURCE_ABI_OK
            );
            assert_eq!(length, 0);
            assert_eq!(
                source_compress_snappy_max_size(u64::MAX, &mut length),
                SOURCE_ABI_INVALID_ARGUMENT
            );
            assert_eq!(length, 0);
            assert_eq!(
                source_compress_snappy_max_size(0, &mut length),
                SOURCE_ABI_OK
            );
            assert_eq!(length, 36);
            assert_eq!(
                source_compress_snappy_compress(empty, ptr::null_mut(), 1, &mut length),
                SOURCE_ABI_INVALID_ARGUMENT
            );
            assert_eq!(
                source_compress_buffer_decompress(empty, ptr::null_mut(), 0, ptr::null_mut()),
                SOURCE_ABI_INVALID_ARGUMENT
            );
            let invalid = SourceAbiSlice {
                data: ptr::null(),
                length: 1,
            };
            assert_eq!(
                source_compress_buffer_actual_size(invalid, &mut length),
                SOURCE_ABI_INVALID_ARGUMENT
            );
            assert_eq!(
                source_compress_buffer_decompress(
                    slice(b"SNAP\xff\xff\xff\xff\x0f"),
                    ptr::null_mut(),
                    0,
                    &mut length
                ),
                SOURCE_ABI_BUFFER_TOO_SMALL
            );
            assert_eq!(length, u32::MAX as u64);
        }
    }
}
