//! Valve's LZMA envelope, the buffer format behind `tier1/lzmaDecoder.cpp`'s
//! `CLZMA`: a packed seventeen-byte `lzma_header_t` holding the `LZMA` tag,
//! the uncompressed and compressed lengths and the five coder properties,
//! followed by a raw LZMA stream that the lengths terminate rather than an
//! end marker.

use std::fmt;
use std::io::{self, Write};

/// `LZMA_ID`, the tag as it appears in the first four bytes.
pub const LZMA_TAG: [u8; 4] = *b"LZMA";
/// `sizeof( lzma_header_t )`, which is packed.
pub const HEADER_BYTES: usize = 17;
/// Dictionaries larger than this are refused rather than allocated.
pub const DEFAULT_MEMORY_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The buffer does not begin with a whole header carrying the tag.
    NotCompressed,
    /// The header names more compressed bytes than the buffer holds.
    Truncated { expected: usize, available: usize },
    /// The stream is corrupt or does not produce the length it declares.
    InvalidStream,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCompressed => f.write_str("buffer has no LZMA header"),
            Self::Truncated {
                expected,
                available,
            } => write!(
                f,
                "LZMA stream declares {expected} bytes but {available} remain"
            ),
            Self::InvalidStream => f.write_str("invalid LZMA stream"),
        }
    }
}

impl std::error::Error for Error {}

/// `CLZMA::IsCompressed`.
pub fn is_compressed(buffer: &[u8]) -> bool {
    buffer.len() >= HEADER_BYTES && buffer[..4] == LZMA_TAG
}

/// `CLZMA::GetActualSize`, which is zero for a buffer that is not compressed.
pub fn actual_size(buffer: &[u8]) -> usize {
    if is_compressed(buffer) {
        read_u32(buffer, 4) as usize
    } else {
        0
    }
}

/// `CLZMA::Uncompress`, except that the output is owned and a stream that
/// misstates its length is an error instead of a write past the destination.
pub fn decompress(buffer: &[u8]) -> Result<Vec<u8>, Error> {
    decompress_with_memory_limit(buffer, DEFAULT_MEMORY_LIMIT)
}

pub fn decompress_with_memory_limit(buffer: &[u8], memory_limit: usize) -> Result<Vec<u8>, Error> {
    if !is_compressed(buffer) {
        return Err(Error::NotCompressed);
    }
    let actual = read_u32(buffer, 4) as usize;
    let compressed = read_u32(buffer, 8) as usize;
    let available = buffer.len() - HEADER_BYTES;
    if compressed > available {
        return Err(Error::Truncated {
            expected: compressed,
            available,
        });
    }
    // lzma-rs reads the five properties itself, and they sit directly before
    // the stream in this header, so one slice covers both.
    let stream = &buffer[12..HEADER_BYTES + compressed];
    // The native decoder stops at the declared length and then accepts either
    // the end of the stream or an end marker, and nothing else. Valve's own
    // encoder writes no marker; liblzma's always does.
    decode(stream, actual, Some(actual as u64), memory_limit)
        .or_else(|| decode(stream, actual, None, memory_limit))
        .ok_or(Error::InvalidStream)
}

/// One pass over the stream, ended by `unpacked_size` or, without one, by the
/// stream's end marker. Succeeds only if it consumed the whole stream and
/// produced exactly `actual` bytes.
fn decode(
    stream: &[u8],
    actual: usize,
    unpacked_size: Option<u64>,
    memory_limit: usize,
) -> Option<Vec<u8>> {
    let mut input = stream;
    let mut output = Bounded {
        bytes: Vec::new(),
        limit: actual,
    };
    let options = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(unpacked_size),
        memlimit: Some(memory_limit),
        allow_incomplete: false,
    };
    lzma_rs::lzma_decompress_with_options(&mut input, &mut output, &options).ok()?;
    (output.bytes.len() == actual && input.is_empty()).then_some(output.bytes)
}

fn read_u32(buffer: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buffer[offset],
        buffer[offset + 1],
        buffer[offset + 2],
        buffer[offset + 3],
    ])
}

/// Stops a stream that lies about its length before it exhausts memory.
struct Bounded {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for Bounded {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.len() > self.limit - self.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LZMA output exceeds its declared size",
            ));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(payload: &[u8]) -> Vec<u8> {
        let mut stream = Vec::new();
        lzma_rs::lzma_compress_with_options(
            &mut &payload[..],
            &mut stream,
            &lzma_rs::compress::Options {
                unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader,
            },
        )
        .unwrap();
        let (properties, data) = stream.split_at(5);
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&LZMA_TAG);
        buffer.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buffer.extend_from_slice(properties);
        buffer.extend_from_slice(data);
        buffer
    }

    #[test]
    fn round_trips_the_native_envelope() {
        let payload: Vec<u8> = (0..4096u32).map(|value| (value % 7) as u8).collect();
        let buffer = envelope(&payload);
        assert!(is_compressed(&buffer));
        assert_eq!(actual_size(&buffer), payload.len());
        assert_eq!(decompress(&buffer).unwrap(), payload);
    }

    #[test]
    fn rejects_what_is_not_a_whole_valid_envelope() {
        assert!(!is_compressed(b"LZMA"));
        assert_eq!(actual_size(b"bvcd0123456789abcdef"), 0);
        assert_eq!(
            decompress(b"bvcd0123456789abcdef"),
            Err(Error::NotCompressed)
        );

        let payload = vec![9u8; 512];
        let buffer = envelope(&payload);
        assert!(matches!(
            decompress(&buffer[..buffer.len() - 1]),
            Err(Error::Truncated { .. })
        ));

        let mut overstated = buffer.clone();
        overstated[4..8].copy_from_slice(&1024u32.to_le_bytes());
        assert_eq!(decompress(&overstated), Err(Error::InvalidStream));

        let mut understated = buffer;
        understated[4..8].copy_from_slice(&16u32.to_le_bytes());
        assert_eq!(decompress(&understated), Err(Error::InvalidStream));
    }
}
