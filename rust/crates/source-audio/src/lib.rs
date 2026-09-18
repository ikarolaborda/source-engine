//! Bounded RIFF/WAVE container parser for Source audio assets.

use source_binary::Reader;
use std::fmt;
use std::ops::Range;

pub const WAVE_FORMAT_PCM: u16 = 0x0001;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_chunks: usize,
    pub max_channels: u16,
    pub max_sample_rate: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 1024 * 1024 * 1024,
            max_chunks: 65_536,
            max_channels: 32,
            max_sample_rate: 768_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub id: [u8; 4],
    pub data: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Format {
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub average_bytes_per_second: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    pub extension: Vec<u8>,
    /// False for Valve's known PCM quirk where `block_align` is 1 even for
    /// 16-bit mono data. `sample_frame_bytes` remains safe for decoding.
    pub pcm_layout_consistent: bool,
}

impl Format {
    pub fn sample_frame_bytes(&self) -> Option<usize> {
        if matches!(self.format_tag, WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT) {
            usize::from(self.channels).checked_mul(usize::from(self.bits_per_sample).div_ceil(8))
        } else {
            Some(usize::from(self.block_align))
        }
    }
}

#[derive(Debug, Clone)]
pub struct Wave<'a> {
    bytes: &'a [u8],
    format: Format,
    chunks: Vec<Chunk>,
    data_chunks: Vec<Range<usize>>,
    missing_final_padding: bool,
}

impl<'a> Wave<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        if bytes.len() > limits.max_file_size {
            return Err(Error::FileTooLarge {
                size: bytes.len(),
                limit: limits.max_file_size,
            });
        }
        let mut reader = Reader::new(bytes);
        let signature: [u8; 4] = reader.take(4)?.try_into().expect("four-byte slice");
        if signature != *b"RIFF" {
            return Err(Error::InvalidSignature(signature));
        }
        let riff_size = reader.read_u32_le()? as usize;
        let declared_size = riff_size.checked_add(8).ok_or(Error::SizeOverflow)?;
        let permits_missing_final_padding = declared_size == bytes.len().saturating_add(1);
        if declared_size != bytes.len() && !permits_missing_final_padding {
            return Err(Error::DeclaredSizeMismatch {
                declared: declared_size,
                actual: bytes.len(),
            });
        }
        let form: [u8; 4] = reader.take(4)?.try_into().expect("four-byte slice");
        if form != *b"WAVE" {
            return Err(Error::InvalidForm(form));
        }

        let mut chunks = Vec::new();
        let mut missing_final_padding = false;
        while reader.position() < bytes.len() {
            if chunks.len() >= limits.max_chunks {
                return Err(Error::ChunkLimitExceeded(limits.max_chunks));
            }
            let id: [u8; 4] = reader.take(4)?.try_into().expect("four-byte slice");
            let size = reader.read_u32_le()? as usize;
            let start = reader.position();
            let end = start.checked_add(size).ok_or(Error::SizeOverflow)?;
            if end > bytes.len() {
                return Err(Error::InvalidChunkRange {
                    id,
                    offset: start,
                    size,
                });
            }
            reader.seek(end)?;
            if size % 2 != 0 {
                if reader.position() == bytes.len() {
                    // Source assets frequently omit only the final physical
                    // RIFF pad byte. Some count it in RIFF size and some do
                    // not, so both declared-size conventions are accepted.
                    missing_final_padding = true;
                } else {
                    reader.skip(1)?;
                }
            }
            chunks.push(Chunk {
                id,
                data: start..end,
            });
        }
        if permits_missing_final_padding && !missing_final_padding {
            return Err(Error::DeclaredSizeMismatch {
                declared: declared_size,
                actual: bytes.len(),
            });
        }

        let format_chunk = unique_chunk(&chunks, *b"fmt ")?.ok_or(Error::MissingFormatChunk)?;
        let format = parse_format(&bytes[format_chunk.data.clone()], limits)?;
        let data_chunks: Vec<_> = chunks
            .iter()
            .filter(|chunk| chunk.id == *b"data")
            .map(|chunk| chunk.data.clone())
            .collect();
        if data_chunks.is_empty() {
            return Err(Error::MissingDataChunk);
        }
        if matches!(format.format_tag, WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT) {
            let frame_bytes = format.sample_frame_bytes().ok_or(Error::SizeOverflow)?;
            for range in &data_chunks {
                let size = range.end - range.start;
                if size % frame_bytes != 0 {
                    return Err(Error::MisalignedSampleData {
                        size,
                        block_align: u16::try_from(frame_bytes).unwrap_or(u16::MAX),
                    });
                }
            }
        }
        Ok(Self {
            bytes,
            format,
            chunks,
            data_chunks,
            missing_final_padding,
        })
    }

    pub fn format(&self) -> &Format {
        &self.format
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn data_chunks(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.data_chunks
            .iter()
            .filter_map(|range| self.bytes.get(range.clone()))
    }

    /// Valve-authored assets commonly omit the physical pad byte for an odd
    /// final chunk while retaining it in the RIFF size. The parser accepts and
    /// reports that precise EOF-only compatibility case.
    pub fn missing_final_padding(&self) -> bool {
        self.missing_final_padding
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    FileTooLarge {
        size: usize,
        limit: usize,
    },
    InvalidSignature([u8; 4]),
    InvalidForm([u8; 4]),
    DeclaredSizeMismatch {
        declared: usize,
        actual: usize,
    },
    ChunkLimitExceeded(usize),
    InvalidChunkRange {
        id: [u8; 4],
        offset: usize,
        size: usize,
    },
    DuplicateChunk([u8; 4]),
    MissingFormatChunk,
    MissingDataChunk,
    InvalidFormatChunk(usize),
    InvalidChannels(u16),
    InvalidSampleRate(u32),
    InvalidBlockAlign(u16),
    InvalidBitsPerSample(u16),
    InvalidPcmLayout,
    InvalidFormatExtension {
        declared: usize,
        available: usize,
    },
    MisalignedSampleData {
        size: usize,
        block_align: u16,
    },
    SizeOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::FileTooLarge { size, limit } => {
                write!(f, "WAVE file size {size} exceeds limit {limit}")
            }
            Self::InvalidSignature(value) => write!(f, "invalid RIFF signature {value:?}"),
            Self::InvalidForm(value) => write!(f, "invalid RIFF form {value:?}; expected WAVE"),
            Self::DeclaredSizeMismatch { declared, actual } => write!(
                f,
                "RIFF declared size {declared} does not match file size {actual}"
            ),
            Self::ChunkLimitExceeded(limit) => write!(f, "RIFF chunk count exceeds {limit}"),
            Self::InvalidChunkRange { id, offset, size } => {
                write!(f, "RIFF chunk {id:?} has invalid range {offset}+{size}")
            }
            Self::DuplicateChunk(id) => write!(f, "duplicate required RIFF chunk {id:?}"),
            Self::MissingFormatChunk => write!(f, "WAVE file has no fmt chunk"),
            Self::MissingDataChunk => write!(f, "WAVE file has no data chunk"),
            Self::InvalidFormatChunk(size) => write!(f, "WAVE fmt chunk is only {size} bytes"),
            Self::InvalidChannels(value) => write!(f, "invalid WAVE channel count {value}"),
            Self::InvalidSampleRate(value) => write!(f, "invalid WAVE sample rate {value}"),
            Self::InvalidBlockAlign(value) => write!(f, "invalid WAVE block alignment {value}"),
            Self::InvalidBitsPerSample(value) => {
                write!(f, "invalid WAVE bits per sample {value}")
            }
            Self::InvalidPcmLayout => write!(f, "inconsistent PCM WAVE layout"),
            Self::InvalidFormatExtension {
                declared,
                available,
            } => write!(
                f,
                "WAVE fmt extension declares {declared} bytes but only {available} are present"
            ),
            Self::MisalignedSampleData { size, block_align } => write!(
                f,
                "WAVE data size {size} is not aligned to block size {block_align}"
            ),
            Self::SizeOverflow => write!(f, "WAVE size arithmetic overflow"),
        }
    }
}

impl std::error::Error for Error {}

impl From<source_binary::Error> for Error {
    fn from(value: source_binary::Error) -> Self {
        Self::Binary(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn unique_chunk(chunks: &[Chunk], id: [u8; 4]) -> Result<Option<&Chunk>> {
    let mut matches = chunks.iter().filter(|chunk| chunk.id == id);
    let result = matches.next();
    if matches.next().is_some() {
        Err(Error::DuplicateChunk(id))
    } else {
        Ok(result)
    }
}

fn parse_format(bytes: &[u8], limits: Limits) -> Result<Format> {
    if bytes.len() < 16 {
        return Err(Error::InvalidFormatChunk(bytes.len()));
    }
    let mut reader = Reader::new(bytes);
    let format_tag = reader.read_u16_le()?;
    let channels = reader.read_u16_le()?;
    let sample_rate = reader.read_u32_le()?;
    let average_bytes_per_second = reader.read_u32_le()?;
    let block_align = reader.read_u16_le()?;
    let bits_per_sample = reader.read_u16_le()?;
    if channels == 0 || channels > limits.max_channels {
        return Err(Error::InvalidChannels(channels));
    }
    if sample_rate == 0 || sample_rate > limits.max_sample_rate {
        return Err(Error::InvalidSampleRate(sample_rate));
    }
    if block_align == 0 {
        return Err(Error::InvalidBlockAlign(block_align));
    }
    if bits_per_sample == 0 || bits_per_sample > 64 {
        return Err(Error::InvalidBitsPerSample(bits_per_sample));
    }
    let extension = if reader.remaining() == 0 {
        Vec::new()
    } else if matches!(format_tag, WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT) {
        // WAVEFORMAT (PCM) has no cbSize field. A set of Valve-authored files
        // nevertheless writes an 18-byte fmt chunk with a two-byte marker;
        // preserve it as opaque extension data just as the legacy loader does.
        let remaining = reader.remaining();
        reader.take(remaining)?.to_vec()
    } else {
        let declared = reader.read_u16_le()? as usize;
        if declared > reader.remaining() {
            return Err(Error::InvalidFormatExtension {
                declared,
                available: reader.remaining(),
            });
        }
        reader.take(declared)?.to_vec()
    };
    let mut pcm_layout_consistent = true;
    if matches!(format_tag, WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT) {
        let bytes_per_sample = usize::from(bits_per_sample).div_ceil(8);
        let expected_align = usize::from(channels)
            .checked_mul(bytes_per_sample)
            .ok_or(Error::SizeOverflow)?;
        let expected_rate = (sample_rate as usize)
            .checked_mul(expected_align)
            .ok_or(Error::SizeOverflow)?;
        if average_bytes_per_second as usize != expected_rate {
            return Err(Error::InvalidPcmLayout);
        }
        pcm_layout_consistent = usize::from(block_align) == expected_align;
    }
    Ok(Format {
        format_tag,
        channels,
        sample_rate,
        average_bytes_per_second,
        block_align,
        bits_per_sample,
        extension,
        pcm_layout_consistent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm_wave(data: &[u8]) -> Vec<u8> {
        let padded = data.len() + data.len() % 2;
        let file_size = 12 + 8 + 16 + 8 + padded;
        let mut bytes = Vec::with_capacity(file_size);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&u32::try_from(file_size - 8).unwrap().to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&WAVE_FORMAT_PCM.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(data);
        if data.len() % 2 != 0 {
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn parses_pcm_wave() {
        let bytes = pcm_wave(&[1, 2, 3, 4]);
        let wave = Wave::parse(&bytes).unwrap();
        assert_eq!(wave.format().channels, 1);
        assert_eq!(wave.data_chunks().next(), Some(&[1, 2, 3, 4][..]));
    }

    #[test]
    fn rejects_declared_size_and_chunk_truncation() {
        let mut bytes = pcm_wave(&[1, 2, 3, 4]);
        bytes[4] = bytes[4].wrapping_add(1);
        assert!(matches!(
            Wave::parse(&bytes),
            Err(Error::DeclaredSizeMismatch { .. })
        ));

        let mut bytes = pcm_wave(&[1, 2, 3, 4]);
        bytes[40..44].copy_from_slice(&100u32.to_le_bytes());
        assert!(matches!(
            Wave::parse(&bytes),
            Err(Error::InvalidChunkRange { .. })
        ));
    }

    #[test]
    fn rejects_inconsistent_pcm_layout() {
        let mut bytes = pcm_wave(&[1, 2, 3, 4]);
        bytes[32..34].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..32].copy_from_slice(&123u32.to_le_bytes());
        assert!(matches!(Wave::parse(&bytes), Err(Error::InvalidPcmLayout)));
    }

    #[test]
    fn accepts_valve_pcm_block_alignment_quirk_without_trusting_it() {
        let mut bytes = pcm_wave(&[1, 2, 3, 4]);
        bytes[32..34].copy_from_slice(&1u16.to_le_bytes());
        let wave = Wave::parse(&bytes).unwrap();
        assert!(!wave.format().pcm_layout_consistent);
        assert_eq!(wave.format().sample_frame_bytes(), Some(2));
    }

    #[test]
    fn accepts_source_style_missing_final_pad_only_at_eof() {
        let mut bytes = pcm_wave(&[1, 2, 3, 4]);
        bytes.pop();
        bytes[40..44].copy_from_slice(&3u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&8_000u32.to_le_bytes());
        bytes[32..34].copy_from_slice(&1u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&8u16.to_le_bytes());
        let wave = Wave::parse(&bytes).unwrap();
        assert!(wave.missing_final_padding());
        assert_eq!(wave.data_chunks().next().unwrap(), &[1, 2, 3]);
    }
}
