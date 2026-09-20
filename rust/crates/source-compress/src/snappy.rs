//! Source's `SNAP` envelope around a raw Snappy block (not Snappy framing).
//!
//! Format reference: https://github.com/google/snappy/blob/main/format_description.txt
//! The encoder chooses its own matches; compatibility means cross-decoding,
//! not identical compressed bytes. The decoder accepts all three copy forms,
//! including overlapping copies and offsets larger than an encoder block.

use crate::SNAPPY_TAG;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    NotCompressed,
    Truncated,
    InvalidLength,
    InvalidOffset,
    OutputLimit,
    LengthMismatch,
    TrailingData,
    InputTooLarge,
}

/// Includes the four-byte Source tag. Compatible with native callers' buffer
/// sizing contract, and checked even on 32-bit hosts.
pub fn max_compressed_size(length: usize) -> Option<usize> {
    u32::try_from(length).ok()?;
    length.checked_add(length / 6)?.checked_add(36)
}

fn header(input: &[u8]) -> Result<(usize, usize), Error> {
    if !input.starts_with(&SNAPPY_TAG) {
        return Err(Error::NotCompressed);
    }
    let mut length = 0u32;
    for index in 0..5 {
        let byte = *input.get(4 + index).ok_or(Error::Truncated)?;
        if index == 4 && byte > 15 {
            return Err(Error::InvalidLength);
        }
        length |= u32::from(byte & 127) << (7 * index);
        if byte & 128 == 0 {
            return Ok((length as usize, 5 + index));
        }
    }
    Err(Error::InvalidLength)
}

/// Only reads the length prefix; it does not validate the token stream.
pub fn actual_size(input: &[u8]) -> Result<usize, Error> {
    header(input).map(|(length, _)| length)
}

fn take<'a>(input: &mut &'a [u8], length: usize) -> Result<&'a [u8], Error> {
    let bytes = input.get(..length).ok_or(Error::Truncated)?;
    *input = &input[length..];
    Ok(bytes)
}

fn little(input: &mut &[u8], count: usize) -> Result<u32, Error> {
    let mut value = 0;
    for (index, byte) in take(input, count)?.iter().enumerate() {
        value |= u32::from(*byte) << (8 * index);
    }
    Ok(value)
}

/// Rejects oversized headers before allocating, and never produces more than
/// `limit`. A tagged malformed stream is an error, never uncompressed data.
pub fn decompress(input: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let (length, prefix) = header(input)?;
    if length > limit {
        return Err(Error::OutputLimit);
    }
    let mut input = &input[prefix..];
    // Every token emits at most 64 bytes per encoded byte (a conservative
    // bound). Do not reserve gigabytes based on a tiny hostile length prefix.
    if length > input.len().saturating_mul(64) {
        return Err(Error::LengthMismatch);
    }
    let mut output = Vec::with_capacity(length);
    while output.len() < length {
        let tag = take(&mut input, 1)?[0];
        let (count, offset) = match tag & 3 {
            0 => {
                let encoded = u32::from(tag >> 2);
                let count = if encoded < 60 {
                    encoded + 1
                } else {
                    little(&mut input, (encoded - 59) as usize)?
                        .checked_add(1)
                        .ok_or(Error::InvalidLength)?
                };
                (count as usize, None)
            }
            1 => {
                let offset = (u32::from(tag & 0xe0) << 3) | little(&mut input, 1)?;
                (usize::from((tag >> 2) & 7) + 4, Some(offset as usize))
            }
            form => (
                usize::from(tag >> 2) + 1,
                Some(little(&mut input, if form == 2 { 2 } else { 4 })? as usize),
            ),
        };
        if count > length - output.len() {
            return Err(Error::LengthMismatch);
        }
        if let Some(offset) = offset {
            if offset == 0 || offset > output.len() {
                return Err(Error::InvalidOffset);
            }
            // Forward copying is intentional: a copy may repeat its own output.
            for _ in 0..count {
                output.push(output[output.len() - offset]);
            }
        } else {
            output.extend_from_slice(take(&mut input, count)?);
        }
    }
    if !input.is_empty() {
        return Err(Error::TrailingData);
    }
    Ok(output)
}

fn literal(input: &[u8], output: &mut Vec<u8>) {
    if input.is_empty() {
        return;
    }
    let length = (input.len() - 1) as u32;
    if length < 60 {
        output.push((length << 2) as u8);
    } else {
        let bytes = (32 - length.leading_zeros()).div_ceil(8) as usize;
        output.push(((59 + bytes) << 2) as u8);
        output.extend_from_slice(&length.to_le_bytes()[..bytes]);
    }
    output.extend_from_slice(input);
}

fn copy(offset: usize, mut length: usize, output: &mut Vec<u8>) {
    while length > 0 {
        let count = length.min(64);
        if offset < 2048 && (4..=11).contains(&count) {
            output.push(1 | (((count - 4) << 2) as u8) | (((offset >> 8) << 5) as u8));
            output.push(offset as u8);
        } else {
            output.push(2 | (((count - 1) << 2) as u8));
            output.extend_from_slice(&(offset as u16).to_le_bytes());
        }
        length -= count;
    }
}

fn hash(input: &[u8], index: usize) -> usize {
    let word = u32::from_le_bytes(input[index..index + 4].try_into().unwrap());
    (word.wrapping_mul(0x9e37_79b1) >> 18) as usize
}

pub fn compress(input: &[u8]) -> Result<Vec<u8>, Error> {
    let bound = max_compressed_size(input.len()).ok_or(Error::InputTooLarge)?;
    let mut output = Vec::with_capacity(bound);
    output.extend_from_slice(&SNAPPY_TAG);
    let mut length = input.len() as u32;
    while length >= 128 {
        output.push((length as u8) | 128);
        length >>= 7;
    }
    output.push(length as u8);
    let mut table = vec![usize::MAX; 1 << 14];
    for block in input.chunks(65536) {
        table.fill(usize::MAX);
        let mut cursor = 0;
        let mut pending = 0;
        while cursor + 4 <= block.len() {
            let slot = hash(block, cursor);
            let previous = table[slot];
            table[slot] = cursor;
            if previous == usize::MAX || block[previous..previous + 4] != block[cursor..cursor + 4]
            {
                cursor += 1;
                continue;
            }
            literal(&block[pending..cursor], &mut output);
            let mut count = 4;
            while cursor + count < block.len() && block[previous + count] == block[cursor + count] {
                count += 1;
            }
            copy(cursor - previous, count, &mut output);
            cursor += count;
            pending = cursor;
        }
        literal(&block[pending..], &mut output);
    }
    debug_assert!(output.len() <= bound);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagged(raw: &[u8]) -> Vec<u8> {
        [b"SNAP".as_slice(), raw].concat()
    }

    #[test]
    fn golden_copy_forms_and_overlap() {
        for raw in [
            vec![7, 8, b'x', b'a', b'b', 1, 2],
            vec![7, 8, b'x', b'a', b'b', 14, 2, 0],
            vec![7, 8, b'x', b'a', b'b', 15, 2, 0, 0, 0],
        ] {
            assert_eq!(decompress(&tagged(&raw), 7).unwrap(), b"xababab");
        }
        assert_eq!(decompress(&tagged(&[0]), 0).unwrap(), b"");
    }

    #[test]
    fn literal_sizes_and_roundtrips() {
        for length in [
            0, 1, 4, 59, 60, 61, 255, 256, 257, 65535, 65536, 65537, 200000,
        ] {
            let input: Vec<_> = (0..length).map(|i| (i % 251) as u8).collect();
            let compressed = compress(&input).unwrap();
            assert!(compressed.len() <= max_compressed_size(length).unwrap());
            assert_eq!(actual_size(&compressed), Ok(length));
            assert_eq!(decompress(&compressed, length).unwrap(), input);
            let mut encoded = tagged(&[]);
            let mut n = length;
            while n >= 128 {
                encoded.push((n as u8) | 128);
                n >>= 7;
            }
            encoded.push(n as u8);
            literal(&input, &mut encoded);
            assert_eq!(decompress(&encoded, length).unwrap(), input);
        }
    }

    #[test]
    fn long_offset_is_not_limited_to_encoder_block_size() {
        let plain = vec![b'z'; 70000];
        let mut encoded = tagged(&[0xf4, 0xa2, 4]); // 70004
        literal(&plain, &mut encoded);
        encoded.push(15); // four-byte copy, length 4, offset 70000
        encoded.extend_from_slice(&70000u32.to_le_bytes());
        assert_eq!(decompress(&encoded, 70004).unwrap(), vec![b'z'; 70004]);
    }

    #[test]
    fn malformed_streams_and_limits() {
        for raw in [
            vec![],
            vec![128],
            vec![255, 255, 255, 255, 16],
            vec![255, 255, 255, 255, 255, 0],
            vec![4, 1, 0],
            vec![4, 1, 1],
            vec![1, 0],
            vec![1, 4, b'a', b'b'],
            vec![0, 0],
            vec![1, 252, 255, 255, 255, 255],
        ] {
            assert!(decompress(&tagged(&raw), 1000).is_err(), "{raw:?}");
        }
        let compressed = compress(&vec![b'x'; 1024]).unwrap();
        assert_eq!(decompress(&compressed, 1023), Err(Error::OutputLimit));
        for cut in 0..compressed.len() {
            assert!(decompress(&compressed[..cut], 1024).is_err());
        }
        assert_eq!(
            decompress(&tagged(&[255, 255, 255, 255, 15]), usize::MAX),
            Err(Error::LengthMismatch)
        );
    }
}
