//! The LZSS codec Source uses for saves, demos and compressed buffers.
//!
//! The layout matches `tier1/lzss.cpp`: an eight-byte header holding the
//! `LZSS` tag and the uncompressed length, then groups of one command byte
//! followed by up to eight tokens. A command bit set means a back-reference,
//! clear means a literal byte, and the bits are consumed from the least
//! significant end.
//!
//! Compression here reproduces the native encoder byte for byte, including
//! its greedy match choice and its refusal to emit a buffer that did not get
//! smaller. That matters because the encoder is the side a save file is
//! written with, and a save the native engine cannot read is worse than no
//! save at all.

use std::fmt;

/// `LZSS_ID`, which is the tag `LZSS` as it appears in the first four bytes.
pub const LZSS_TAG: [u8; 4] = *b"LZSS";
/// `SNAPPY_ID`, recognised only well enough to report that a buffer is
/// Snappy rather than LZSS.
pub const SNAPPY_TAG: [u8; 4] = *b"SNAP";
/// The tag and the uncompressed length.
pub const HEADER_BYTES: usize = 8;
/// `DEFAULT_LZSS_WINDOW_SIZE`, how far back a reference may reach.
pub const DEFAULT_WINDOW_SIZE: usize = 4096;
/// `LZSS_LOOKSHIFT`, which splits a reference's two bytes into a twelve-bit
/// distance and a four-bit length.
const LOOK_SHIFT: u32 = 4;
/// `LZSS_LOOKAHEAD`, the longest run a single reference can name.
const LOOKAHEAD: usize = 1 << LOOK_SHIFT;
/// Below this a reference costs more than the literals it replaces.
const MIN_MATCH: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressError {
    /// The buffer is too short for compression to be able to pay for its own
    /// header and end marker, so the native encoder declines it outright.
    TooSmall(usize),
    /// Compression made the buffer larger. The native encoder abandons rather
    /// than emit it, and the caller stores the original instead.
    WouldGrow,
    /// The input is longer than the header's length field can describe.
    TooLarge(usize),
    /// A window that is not a power of two, which the encoder's wrapping node
    /// table depends on.
    WindowNotPowerOfTwo(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecompressError {
    /// The buffer does not begin with the `LZSS` tag.
    NotCompressed,
    /// The buffer carries a tag, but is too short to hold anything after it.
    Truncated(usize),
    /// A token ran off the end of the input.
    UnexpectedEnd,
    /// A reference points before the start of what has been decoded, so there
    /// is nothing there to copy.
    ReferenceBeforeStart { distance: usize, produced: usize },
    /// The stream decoded to a different length than its header declared,
    /// which means it is not the stream that header belongs to.
    LengthMismatch { declared: usize, produced: usize },
    /// Decoding would exceed the length the header declared.
    LongerThanDeclared(usize),
    /// The header declares more output than this much input could possibly
    /// produce, so the buffer is not a truncated stream but a wrong one.
    DeclaredMoreThanPossible { declared: usize, possible: usize },
}

impl fmt::Display for CompressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall(length) => write!(
                formatter,
                "{length} bytes is too short to compress, needing more than {}",
                HEADER_BYTES + 8
            ),
            Self::WouldGrow => write!(formatter, "compression did not make the buffer smaller"),
            Self::TooLarge(length) => {
                write!(formatter, "{length} bytes cannot be described in a u32")
            }
            Self::WindowNotPowerOfTwo(window) => {
                write!(formatter, "window {window} is not a power of two")
            }
        }
    }
}

impl fmt::Display for DecompressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCompressed => write!(formatter, "the buffer does not carry the LZSS tag"),
            Self::Truncated(length) => {
                write!(formatter, "{length} bytes is not a complete LZSS buffer")
            }
            Self::UnexpectedEnd => write!(formatter, "a token ran past the end of the input"),
            Self::ReferenceBeforeStart {
                distance,
                produced,
            } => write!(
                formatter,
                "a reference {distance} back reaches before the {produced} bytes decoded so far"
            ),
            Self::LengthMismatch {
                declared,
                produced,
            } => write!(
                formatter,
                "the stream decoded to {produced} bytes where its header declared {declared}"
            ),
            Self::LongerThanDeclared(declared) => write!(
                formatter,
                "the stream decodes to more than the {declared} bytes its header declared"
            ),
            Self::DeclaredMoreThanPossible {
                declared,
                possible,
            } => write!(
                formatter,
                "the header declares {declared} bytes where this input could produce at most {possible}"
            ),
        }
    }
}

impl std::error::Error for CompressError {}
impl std::error::Error for DecompressError {}

/// Whether a buffer carries the `LZSS` tag.
pub fn is_compressed(buffer: &[u8]) -> bool {
    buffer.len() >= 4 && buffer[..4] == LZSS_TAG
}

/// Whether a buffer carries the `SNAP` tag, which this codec does not read.
pub fn is_snappy(buffer: &[u8]) -> bool {
    buffer.len() >= 4 && buffer[..4] == SNAPPY_TAG
}

/// The uncompressed length a tagged buffer declares.
pub fn actual_size(buffer: &[u8]) -> Option<usize> {
    if !is_compressed(buffer) || buffer.len() < HEADER_BYTES {
        return None;
    }
    Some(u32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]) as usize)
}

/// Compresses `input` at the default window size.
pub fn compress(input: &[u8]) -> Result<Vec<u8>, CompressError> {
    compress_with_window(input, DEFAULT_WINDOW_SIZE)
}

/// Compresses `input`, reaching back at most `window` bytes for a match.
///
/// `window` must be a power of two: the encoder's match index is a table of
/// that many slots indexed by position, so a slot is reused exactly `window`
/// positions later, and that wrap is what bounds how far back a reference can
/// point.
pub fn compress_with_window(input: &[u8], window: usize) -> Result<Vec<u8>, CompressError> {
    if !window.is_power_of_two() {
        return Err(CompressError::WindowNotPowerOfTwo(window));
    }
    // The native encoder declines anything this short before allocating, on
    // the grounds that the header and end marker alone would eat the gain.
    if input.len() <= HEADER_BYTES + 8 {
        return Err(CompressError::TooSmall(input.len()));
    }
    if u32::try_from(input.len()).is_err() {
        return Err(CompressError::TooLarge(input.len()));
    }

    let mut output = Vec::with_capacity(input.len());
    output.extend_from_slice(&LZSS_TAG);
    output.extend_from_slice(&(input.len() as u32).to_le_bytes());

    // Compression is abandoned once the output reaches this, which is where
    // the native encoder gives up rather than inflate the buffer.
    let limit = input.len() - HEADER_BYTES - 8;

    let mut index = MatchIndex::new(window);
    let mut command_slot = 0usize;
    let mut tokens_in_group = 0usize;
    let mut position = 0usize;

    while position < input.len() {
        if tokens_in_group == 0 {
            command_slot = output.len();
            output.push(0);
        }
        tokens_in_group = (tokens_in_group + 1) & 0x07;

        let lookahead = (input.len() - position).min(LOOKAHEAD);
        let found = index.longest_match(input, position, lookahead);

        let consumed = match found {
            Some(found) if found.length >= MIN_MATCH => {
                // A set bit shifted in at the top; the group's final shift
                // brings the first token's bit down to bit zero.
                output[command_slot] = (output[command_slot] >> 1) | 0x80;
                let distance = found.distance - 1;
                output.push((distance >> LOOK_SHIFT) as u8);
                output.push((((distance << LOOK_SHIFT) & 0xf0) | (found.length - 1)) as u8);
                found.length
            }
            _ => {
                output[command_slot] >>= 1;
                output.push(input[position]);
                1
            }
        };

        for offset in position..position + consumed {
            index.insert(input, offset);
        }
        position += consumed;

        if output.len() >= limit {
            return Err(CompressError::WouldGrow);
        }
    }

    // The end marker is a reference whose length field decodes to one, which
    // no real reference ever is, so it needs a command bit of its own.
    if tokens_in_group == 0 {
        output.push(0x01);
    } else {
        output[command_slot] = ((output[command_slot] >> 1) | 0x80) >> (7 - tokens_in_group);
    }
    output.push(0);
    output.push(0);

    Ok(output)
}

/// Decompresses a tagged buffer.
///
/// Every reference is checked against what has actually been decoded, so a
/// corrupt or hostile stream is refused rather than read out of bounds. The
/// native `Uncompress` does neither and writes wherever the stream points;
/// `SafeUncompress` checks, and this follows that.
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, DecompressError> {
    if !is_compressed(input) {
        return Err(DecompressError::NotCompressed);
    }
    if input.len() <= HEADER_BYTES {
        return Err(DecompressError::Truncated(input.len()));
    }
    let declared = actual_size(input).ok_or(DecompressError::Truncated(input.len()))?;

    // The declared length comes straight off the wire, so it is checked
    // against what this much input could actually produce before a buffer is
    // reserved for it. A group is one command byte and at most eight
    // references of two bytes, so seventeen input bytes yield at most eight
    // runs of the sixteen-byte lookahead. Without this, an eight-byte header
    // claiming four gigabytes would reserve four gigabytes.
    let possible = (input.len() - HEADER_BYTES)
        .saturating_mul(LOOKAHEAD * 8)
        .div_ceil(1 + 8 * 2);
    if declared > possible {
        return Err(DecompressError::DeclaredMoreThanPossible { declared, possible });
    }

    let mut output = Vec::with_capacity(declared);
    let mut cursor = HEADER_BYTES;
    let mut command = 0u8;
    let mut tokens_in_group = 0usize;

    loop {
        if tokens_in_group == 0 {
            command = *input.get(cursor).ok_or(DecompressError::UnexpectedEnd)?;
            cursor += 1;
        }
        tokens_in_group = (tokens_in_group + 1) & 0x07;

        if command & 0x01 != 0 {
            let high = *input.get(cursor).ok_or(DecompressError::UnexpectedEnd)?;
            let low = *input
                .get(cursor + 1)
                .ok_or(DecompressError::UnexpectedEnd)?;
            cursor += 2;

            let length = usize::from(low & 0x0f) + 1;
            if length == 1 {
                break;
            }
            let distance = ((usize::from(high) << LOOK_SHIFT) | usize::from(low >> LOOK_SHIFT)) + 1;
            if distance > output.len() {
                return Err(DecompressError::ReferenceBeforeStart {
                    distance,
                    produced: output.len(),
                });
            }
            if output.len() + length > declared {
                return Err(DecompressError::LongerThanDeclared(declared));
            }
            // Copied one byte at a time because a run may overlap itself,
            // which is how the encoder expresses a repeat.
            let start = output.len() - distance;
            for offset in 0..length {
                let byte = output[start + offset];
                output.push(byte);
            }
        } else {
            let byte = *input.get(cursor).ok_or(DecompressError::UnexpectedEnd)?;
            cursor += 1;
            if output.len() + 1 > declared {
                return Err(DecompressError::LongerThanDeclared(declared));
            }
            output.push(byte);
        }
        command >>= 1;
    }

    if output.len() != declared {
        return Err(DecompressError::LengthMismatch {
            declared,
            produced: output.len(),
        });
    }
    Ok(output)
}

/// A match the index found.
struct Found {
    distance: usize,
    length: usize,
}

/// The encoder's match index.
///
/// The native encoder keeps, per leading byte value, a list of recent
/// positions starting with that byte, most recent first, and a table of
/// `window` nodes indexed by position so that inserting a position evicts the
/// one exactly `window` earlier. Reproducing both is what makes the chosen
/// match, and therefore the compressed bytes, identical: the search takes the
/// first strictly longest match walking from the most recent, and stops as
/// soon as one fills the lookahead.
struct MatchIndex {
    mask: usize,
    /// Head of the list for each leading byte value, as a slot index.
    heads: [Option<usize>; 256],
    /// Tail of each list, which is the slot eviction takes from.
    tails: [Option<usize>; 256],
    /// Per slot: the position it holds, and its neighbours in its list.
    slots: Vec<Slot>,
}

#[derive(Clone, Copy, Default)]
struct Slot {
    position: Option<usize>,
    previous: Option<usize>,
    next: Option<usize>,
}

impl MatchIndex {
    fn new(window: usize) -> Self {
        Self {
            mask: window - 1,
            heads: [None; 256],
            tails: [None; 256],
            slots: vec![Slot::default(); window],
        }
    }

    fn insert(&mut self, input: &[u8], position: usize) {
        let slot = position & self.mask;

        // The slot's previous occupant is exactly `window` positions back, so
        // evicting it is what keeps every reference inside the window.
        if let Some(evicted) = self.slots[slot].position {
            let value = usize::from(input[evicted]);
            match self.slots[slot].previous {
                Some(previous) => {
                    self.tails[value] = Some(previous);
                    self.slots[previous].next = None;
                }
                None => {
                    self.heads[value] = None;
                    self.tails[value] = None;
                }
            }
        }

        let value = usize::from(input[position]);
        let head = self.heads[value];
        self.slots[slot] = Slot {
            position: Some(position),
            previous: None,
            next: head,
        };
        match head {
            Some(head) => self.slots[head].previous = Some(slot),
            None => self.tails[value] = Some(slot),
        }
        self.heads[value] = Some(slot);
    }

    fn longest_match(&self, input: &[u8], position: usize, lookahead: usize) -> Option<Found> {
        let mut best: Option<Found> = None;
        let mut slot = self.heads[usize::from(input[position])];

        while let Some(current) = slot {
            let candidate = self.slots[current].position?;
            let mut length = 0;
            // A candidate may reach past `position` into bytes this match is
            // itself producing, which is how a repeating run is expressed.
            while length < lookahead && input[candidate + length] == input[position + length] {
                length += 1;
            }
            // Strictly greater, so the most recent of equally long matches
            // wins, which is the shortest distance and what the native
            // encoder picks.
            if best.as_ref().is_none_or(|best| length > best.length) {
                best = Some(Found {
                    distance: position - candidate,
                    length,
                });
            }
            if length == lookahead {
                break;
            }
            slot = self.slots[current].next;
        }

        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text compresses, because it repeats; this is long enough to clear the
    /// encoder's minimum and to contain runs worth referencing.
    const PROSE: &[u8] = b"the quick brown fox jumps over the lazy dog; \
the quick brown fox jumps over the lazy dog; \
the quick brown fox jumps over the lazy dog.";

    #[test]
    fn decodes_the_native_golden_vector() {
        // These exact bytes are checked into `unittests/tier1test/lzsstest.cpp`
        // as a stream the native codec produced, so decoding them proves the
        // group and bit ordering match rather than merely being self
        // consistent. The payload is a C string, so its terminator is part of
        // the 26 bytes the header declares.
        let native: [u8; 36] = [
            0x4c, 0x5a, 0x53, 0x53, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x44, 0x6f, 0x20, 0x79, 0x6f,
            0x75, 0x20, 0x6c, 0x00, 0x69, 0x6b, 0x65, 0x20, 0x77, 0x68, 0x61, 0x74, 0x41, 0x00,
            0xd4, 0x73, 0x65, 0x65, 0x3f, 0x00, 0x00, 0x00,
        ];
        assert_eq!(actual_size(&native), Some(26));
        assert_eq!(
            decompress(&native).as_deref(),
            Ok(b"Do you like what you see?\0".as_slice())
        );
    }

    #[test]
    fn round_trips_a_buffer_that_compresses() {
        let compressed = compress(PROSE).expect("compressed");
        assert!(is_compressed(&compressed));
        assert_eq!(actual_size(&compressed), Some(PROSE.len()));
        // Worth compressing at all only if it actually got smaller.
        assert!(compressed.len() < PROSE.len(), "{}", compressed.len());
        assert_eq!(decompress(&compressed).as_deref(), Ok(PROSE));
    }

    #[test]
    fn round_trips_a_long_repeating_run() {
        // A single repeated byte is the case that exercises overlapping
        // references, where a match reads bytes it is itself producing.
        let input = vec![0x5au8; 8192];
        let compressed = compress(&input).expect("compressed");
        assert!(compressed.len() < input.len() / 4, "{}", compressed.len());
        assert_eq!(decompress(&compressed).as_deref(), Ok(input.as_slice()));
    }

    #[test]
    fn round_trips_across_the_window_boundary() {
        // Two copies of enough data that the second copy's matches have to
        // reach back through a full turn of the node table.
        let mut input = Vec::new();
        for index in 0..DEFAULT_WINDOW_SIZE * 2 {
            input.push((index % 251) as u8);
        }
        let compressed = compress(&input).expect("compressed");
        assert_eq!(decompress(&compressed).as_deref(), Ok(input.as_slice()));
    }

    #[test]
    fn round_trips_every_length_around_the_minimum() {
        // The group and end-marker bookkeeping differs depending on how many
        // tokens land in the final group, so every remainder mod eight has to
        // be covered rather than a convenient length.
        for length in 17..200usize {
            let input: Vec<u8> = (0..length).map(|index| (index % 7) as u8).collect();
            match compress(&input) {
                Ok(compressed) => assert_eq!(
                    decompress(&compressed).as_deref(),
                    Ok(input.as_slice()),
                    "length {length}"
                ),
                // Refusing to grow is a legitimate outcome for short input.
                Err(CompressError::WouldGrow) => {}
                Err(error) => panic!("length {length}: {error}"),
            }
        }
    }

    #[test]
    fn refuses_input_it_cannot_help() {
        for length in 0..=HEADER_BYTES + 8 {
            assert_eq!(
                compress(&vec![0u8; length]),
                Err(CompressError::TooSmall(length)),
                "length {length}"
            );
        }
        // Random bytes have nothing to reference, so the encoder gives up
        // rather than emit something larger than it was given.
        let incompressible: Vec<u8> = (0..512u32)
            .map(|index| {
                let mixed = index
                    .wrapping_mul(2_654_435_761)
                    .rotate_left(13)
                    .wrapping_mul(0x9e37_79b9);
                (mixed >> 24) as u8
            })
            .collect();
        assert_eq!(compress(&incompressible), Err(CompressError::WouldGrow));
        assert_eq!(
            compress_with_window(PROSE, 3000),
            Err(CompressError::WindowNotPowerOfTwo(3000))
        );
    }

    #[test]
    fn refuses_streams_that_do_not_decode() {
        assert_eq!(decompress(b"NOPE1234"), Err(DecompressError::NotCompressed));
        assert_eq!(decompress(b"LZSS"), Err(DecompressError::Truncated(4)));
        assert_eq!(
            decompress(b"LZSS\x10\x00\x00\x00"),
            Err(DecompressError::Truncated(8))
        );
        // A command byte promising eight literals that are not there.
        assert_eq!(
            decompress(b"LZSS\x08\x00\x00\x00\x00"),
            Err(DecompressError::UnexpectedEnd)
        );
        // A reference as the very first token has nothing behind it.
        assert_eq!(
            decompress(b"LZSS\x03\x00\x00\x00\x01\x00\x05"),
            Err(DecompressError::ReferenceBeforeStart {
                distance: 1,
                produced: 0
            })
        );

        // An eight-byte header claiming four gigabytes must be refused
        // before anything is reserved for it, not merely fail afterwards.
        let mut absurd = b"LZSS\x00\x00\x00\x00\x00".to_vec();
        absurd[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decompress(&absurd),
            Err(DecompressError::DeclaredMoreThanPossible {
                declared: u32::MAX as usize,
                possible: 8
            })
        );

        let compressed = compress(PROSE).expect("compressed");

        // A header claiming a length the stream does not decode to.
        let mut wrong_length = compressed.clone();
        wrong_length[4..8].copy_from_slice(&(PROSE.len() as u32 + 1).to_le_bytes());
        assert_eq!(
            decompress(&wrong_length),
            Err(DecompressError::LengthMismatch {
                declared: PROSE.len() + 1,
                produced: PROSE.len()
            })
        );

        // A header claiming less than the stream decodes to must be caught
        // while decoding rather than by writing past the buffer.
        let mut short_length = compressed.clone();
        short_length[4..8].copy_from_slice(&16u32.to_le_bytes());
        assert_eq!(
            decompress(&short_length),
            Err(DecompressError::LongerThanDeclared(16))
        );

        // Truncating anywhere inside the stream has to be refused, never
        // read past.
        for cut in HEADER_BYTES..compressed.len() {
            let error = decompress(&compressed[..cut]).expect_err("truncated");
            assert!(
                matches!(
                    error,
                    DecompressError::Truncated(_)
                        | DecompressError::UnexpectedEnd
                        | DecompressError::LengthMismatch { .. }
                        | DecompressError::ReferenceBeforeStart { .. }
                        | DecompressError::LongerThanDeclared(_)
                        | DecompressError::DeclaredMoreThanPossible { .. }
                ),
                "cut {cut}: {error}"
            );
        }
    }

    #[test]
    fn distinguishes_snappy_from_lzss() {
        // The engine tags Snappy buffers the same way, and reading one as
        // LZSS would produce garbage rather than an error.
        assert!(is_snappy(b"SNAP\x00\x00\x00\x00"));
        assert!(!is_compressed(b"SNAP\x00\x00\x00\x00"));
        assert!(!is_snappy(&compress(PROSE).expect("compressed")));
    }

    #[test]
    fn describes_every_failure() {
        let compress_errors: [CompressError; 4] = [
            CompressError::TooSmall(4),
            CompressError::WouldGrow,
            CompressError::TooLarge(usize::MAX),
            CompressError::WindowNotPowerOfTwo(3000),
        ];
        for error in compress_errors {
            assert!(!error.to_string().is_empty());
        }
        let decompress_errors: [DecompressError; 7] = [
            DecompressError::DeclaredMoreThanPossible {
                declared: 4096,
                possible: 8,
            },
            DecompressError::NotCompressed,
            DecompressError::Truncated(4),
            DecompressError::UnexpectedEnd,
            DecompressError::ReferenceBeforeStart {
                distance: 1,
                produced: 0,
            },
            DecompressError::LengthMismatch {
                declared: 2,
                produced: 1,
            },
            DecompressError::LongerThanDeclared(16),
        ];
        for error in decompress_errors {
            assert!(!error.to_string().is_empty());
        }
    }
}
