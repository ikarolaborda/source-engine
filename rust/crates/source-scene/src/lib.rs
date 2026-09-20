//! Parser for Source `scenes/scenes.image` caches and compiled-scene envelopes,
//! and in [`cache`] the lookups the running game makes against a loaded image.

pub mod cache;

use source_binary::Reader;
use std::fmt;
use std::ops::Range;

pub const SCENE_IMAGE_MAGIC: u32 = u32::from_le_bytes(*b"VSIF");
pub const SCENE_IMAGE_VERSION: i32 = 2;
pub const SCENE_BINARY_MAGIC: u32 = u32::from_le_bytes(*b"bvcd");
pub const SCENE_BINARY_VERSION: u8 = 4;
pub const LZMA_MAGIC: u32 = u32::from_le_bytes(*b"LZMA");

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_scenes: usize,
    pub max_strings: usize,
    pub max_sounds_per_scene: usize,
    pub max_uncompressed_scene_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 1024 * 1024 * 1024,
            max_scenes: 1_000_000,
            max_strings: 1_000_000,
            max_sounds_per_scene: 65_536,
            max_uncompressed_scene_size: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneBlob {
    Binary {
        source_crc: u32,
    },
    Lzma {
        uncompressed_size: u32,
        compressed_size: u32,
        properties: [u8; 5],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub milliseconds: u32,
    pub sound_string_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub filename_crc: u32,
    pub data: Range<usize>,
    pub summary_offset: usize,
    pub summary: Summary,
    pub blob: SceneBlob,
}

#[derive(Debug, Clone)]
pub struct SceneImage<'a> {
    bytes: &'a [u8],
    strings: Vec<&'a [u8]>,
    entries: Vec<Entry>,
}

impl<'a> SceneImage<'a> {
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
        let magic = reader.read_u32_le()?;
        if magic != SCENE_IMAGE_MAGIC {
            return Err(Error::InvalidMagic(magic));
        }
        let version = reader.read_i32_le()?;
        if version != SCENE_IMAGE_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let scene_count = count("scenes", reader.read_i32_le()?, limits.max_scenes)?;
        let string_count = count("strings", reader.read_i32_le()?, limits.max_strings)?;
        let scene_entry_offset = positive_offset("scene directory", reader.read_i32_le()?)?;

        let offset_table_size = string_count.checked_mul(4).ok_or(Error::SizeOverflow)?;
        let string_pool_start = 20usize
            .checked_add(offset_table_size)
            .ok_or(Error::SizeOverflow)?;
        if string_pool_start > scene_entry_offset || scene_entry_offset > bytes.len() {
            return Err(Error::InvalidRange {
                field: "string table and pool",
                offset: 20,
                size: scene_entry_offset.saturating_sub(20),
            });
        }
        let mut strings = Vec::with_capacity(string_count);
        let mut previous_offset = None;
        for _ in 0..string_count {
            let offset = reader.read_u32_le()? as usize;
            if offset < string_pool_start || offset >= scene_entry_offset {
                return Err(Error::InvalidStringOffset(offset));
            }
            if previous_offset.is_some_and(|previous| offset <= previous) {
                return Err(Error::UnsortedStringOffsets);
            }
            previous_offset = Some(offset);
            strings.push(c_string(bytes, offset, scene_entry_offset)?);
        }

        let directory_size = scene_count.checked_mul(16).ok_or(Error::SizeOverflow)?;
        let directory_end = scene_entry_offset
            .checked_add(directory_size)
            .ok_or(Error::SizeOverflow)?;
        if directory_end > bytes.len() {
            return Err(Error::InvalidRange {
                field: "scene directory",
                offset: scene_entry_offset,
                size: directory_size,
            });
        }
        reader.seek(scene_entry_offset)?;
        let mut raw_entries = Vec::with_capacity(scene_count);
        let mut previous_crc = None;
        for _ in 0..scene_count {
            let filename_crc = reader.read_u32_le()?;
            if previous_crc.is_some_and(|previous| filename_crc <= previous) {
                return Err(Error::UnsortedSceneDirectory);
            }
            previous_crc = Some(filename_crc);
            let data_offset = positive_offset("scene data", reader.read_i32_le()?)?;
            let data_length = count(
                "scene data bytes",
                reader.read_i32_le()?,
                limits.max_file_size,
            )?;
            let summary_offset = positive_offset("scene summary", reader.read_i32_le()?)?;
            raw_entries.push((filename_crc, data_offset, data_length, summary_offset));
        }

        let mut entries = Vec::with_capacity(scene_count);
        let mut previous_summary_end = directory_end;
        let mut previous_data_end = 0usize;
        for (index, (filename_crc, data_offset, data_length, summary_offset)) in
            raw_entries.into_iter().enumerate()
        {
            if summary_offset != previous_summary_end {
                return Err(Error::NonContiguousSummaries {
                    expected: previous_summary_end,
                    actual: summary_offset,
                });
            }
            let mut summary_reader = Reader::with_position(bytes, summary_offset)?;
            let milliseconds = summary_reader.read_u32_le()?;
            let sound_count = count(
                "scene sounds",
                summary_reader.read_i32_le()?,
                limits.max_sounds_per_scene,
            )?;
            let mut sound_string_ids = Vec::with_capacity(sound_count);
            for _ in 0..sound_count {
                let id = summary_reader.read_u32_le()?;
                if id as usize >= string_count {
                    return Err(Error::InvalidSoundStringId(id));
                }
                sound_string_ids.push(id);
            }
            previous_summary_end = summary_reader.position();

            let data_end = data_offset
                .checked_add(data_length)
                .ok_or(Error::SizeOverflow)?;
            if data_end > bytes.len() {
                return Err(Error::InvalidRange {
                    field: "scene data",
                    offset: data_offset,
                    size: data_length,
                });
            }
            if index == 0 {
                if data_offset != previous_summary_end
                    && scene_count == 1
                    && data_offset < previous_summary_end
                {
                    return Err(Error::OverlappingSceneSections);
                }
            } else if data_offset != previous_data_end {
                return Err(Error::NonContiguousSceneData {
                    expected: previous_data_end,
                    actual: data_offset,
                });
            }
            previous_data_end = data_end;
            let data = data_offset..data_end;
            let blob = parse_blob(&bytes[data.clone()], limits)?;
            entries.push(Entry {
                filename_crc,
                data,
                summary_offset,
                summary: Summary {
                    milliseconds,
                    sound_string_ids,
                },
                blob,
            });
        }
        if let Some(first) = entries.first() {
            if first.data.start != previous_summary_end {
                return Err(Error::NonContiguousSceneData {
                    expected: previous_summary_end,
                    actual: first.data.start,
                });
            }
        }
        if previous_data_end != 0 && previous_data_end != bytes.len() {
            return Err(Error::TrailingData(bytes.len() - previous_data_end));
        }

        Ok(Self {
            bytes,
            strings,
            entries,
        })
    }

    pub fn strings(&self) -> &[&'a [u8]] {
        &self.strings
    }

    pub fn string_utf8(&self, index: usize) -> Option<&'a str> {
        self.strings
            .get(index)
            .and_then(|value| std::str::from_utf8(value).ok())
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn scene_data(&self, entry: &Entry) -> &'a [u8] {
        &self.bytes[entry.data.clone()]
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    FileTooLarge {
        size: usize,
        limit: usize,
    },
    InvalidMagic(u32),
    UnsupportedVersion(i32),
    InvalidCount {
        field: &'static str,
        value: i32,
    },
    CountLimitExceeded {
        field: &'static str,
        value: usize,
        limit: usize,
    },
    InvalidOffset {
        field: &'static str,
        value: i32,
    },
    InvalidRange {
        field: &'static str,
        offset: usize,
        size: usize,
    },
    InvalidStringOffset(usize),
    UnterminatedString(usize),
    UnsortedStringOffsets,
    UnsortedSceneDirectory,
    InvalidSoundStringId(u32),
    NonContiguousSummaries {
        expected: usize,
        actual: usize,
    },
    NonContiguousSceneData {
        expected: usize,
        actual: usize,
    },
    OverlappingSceneSections,
    InvalidSceneBlobMagic(u32),
    UnsupportedSceneBinaryVersion(u8),
    InvalidLzmaSize {
        declared: usize,
        actual: usize,
    },
    UncompressedSceneTooLarge {
        size: usize,
        limit: usize,
    },
    TrailingData(usize),
    SizeOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::FileTooLarge { size, limit } => {
                write!(f, "scene image size {size} exceeds limit {limit}")
            }
            Self::InvalidMagic(value) => write!(f, "invalid scene image magic 0x{value:08x}"),
            Self::UnsupportedVersion(value) => write!(f, "unsupported scene image version {value}"),
            Self::InvalidCount { field, value } => write!(f, "invalid {field} count {value}"),
            Self::CountLimitExceeded {
                field,
                value,
                limit,
            } => {
                write!(f, "{field} count {value} exceeds limit {limit}")
            }
            Self::InvalidOffset { field, value } => write!(f, "invalid {field} offset {value}"),
            Self::InvalidRange {
                field,
                offset,
                size,
            } => {
                write!(
                    f,
                    "{field} range {offset}+{size} is outside the scene image"
                )
            }
            Self::InvalidStringOffset(value) => write!(f, "invalid scene string offset {value}"),
            Self::UnterminatedString(offset) => write!(f, "unterminated scene string at {offset}"),
            Self::UnsortedStringOffsets => {
                write!(f, "scene string offsets are not strictly sorted")
            }
            Self::UnsortedSceneDirectory => {
                write!(f, "scene directory CRCs are not strictly sorted")
            }
            Self::InvalidSoundStringId(value) => write!(f, "invalid scene sound string ID {value}"),
            Self::NonContiguousSummaries { expected, actual } => write!(
                f,
                "non-contiguous scene summaries: expected {expected}, found {actual}"
            ),
            Self::NonContiguousSceneData { expected, actual } => write!(
                f,
                "non-contiguous scene data: expected {expected}, found {actual}"
            ),
            Self::OverlappingSceneSections => write!(f, "scene summary and data sections overlap"),
            Self::InvalidSceneBlobMagic(value) => {
                write!(f, "invalid compiled scene/LZMA magic 0x{value:08x}")
            }
            Self::UnsupportedSceneBinaryVersion(value) => {
                write!(f, "unsupported compiled scene version {value}")
            }
            Self::InvalidLzmaSize { declared, actual } => write!(
                f,
                "compiled scene LZMA payload declares {declared} bytes, file contains {actual}"
            ),
            Self::UncompressedSceneTooLarge { size, limit } => {
                write!(
                    f,
                    "compiled scene expands to {size} bytes, limit is {limit}"
                )
            }
            Self::TrailingData(size) => write!(f, "scene image has {size} trailing bytes"),
            Self::SizeOverflow => write!(f, "scene image size arithmetic overflow"),
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

fn parse_blob(bytes: &[u8], limits: Limits) -> Result<SceneBlob> {
    let mut reader = Reader::new(bytes);
    let magic = reader.read_u32_le()?;
    if magic == SCENE_BINARY_MAGIC {
        let version = reader.read_u8()?;
        if version != SCENE_BINARY_VERSION {
            return Err(Error::UnsupportedSceneBinaryVersion(version));
        }
        return Ok(SceneBlob::Binary {
            source_crc: reader.read_u32_le()?,
        });
    }
    if magic != LZMA_MAGIC {
        return Err(Error::InvalidSceneBlobMagic(magic));
    }
    let uncompressed_size = reader.read_u32_le()?;
    if uncompressed_size as usize > limits.max_uncompressed_scene_size {
        return Err(Error::UncompressedSceneTooLarge {
            size: uncompressed_size as usize,
            limit: limits.max_uncompressed_scene_size,
        });
    }
    let compressed_size = reader.read_u32_le()?;
    let properties: [u8; 5] = reader.take(5)?.try_into().expect("five-byte slice");
    if reader.remaining() != compressed_size as usize {
        return Err(Error::InvalidLzmaSize {
            declared: compressed_size as usize,
            actual: reader.remaining(),
        });
    }
    Ok(SceneBlob::Lzma {
        uncompressed_size,
        compressed_size,
        properties,
    })
}

fn count(field: &'static str, value: i32, limit: usize) -> Result<usize> {
    let value = usize::try_from(value).map_err(|_| Error::InvalidCount { field, value })?;
    if value > limit {
        Err(Error::CountLimitExceeded {
            field,
            value,
            limit,
        })
    } else {
        Ok(value)
    }
}

fn positive_offset(field: &'static str, value: i32) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::InvalidOffset { field, value })
}

fn c_string(bytes: &[u8], offset: usize, limit: usize) -> Result<&[u8]> {
    let tail = bytes
        .get(offset..limit)
        .ok_or(Error::InvalidStringOffset(offset))?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::UnterminatedString(offset))?;
    Ok(&tail[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn scene_image(blob: &[u8]) -> Vec<u8> {
        // header 20, one string offset 4, pool 6, directory 16, summary 12.
        let entry_offset = 30;
        let summary_offset = 46;
        let data_offset = 58;
        let mut bytes = vec![0; data_offset];
        bytes[0..4].copy_from_slice(b"VSIF");
        put_i32(&mut bytes, 4, 2);
        put_i32(&mut bytes, 8, 1);
        put_i32(&mut bytes, 12, 1);
        put_i32(&mut bytes, 16, entry_offset);
        put_i32(&mut bytes, 20, 24);
        bytes[24..30].copy_from_slice(b"sound\0");
        bytes[30..34].copy_from_slice(&10u32.to_le_bytes());
        put_i32(&mut bytes, 34, data_offset as i32);
        put_i32(&mut bytes, 38, blob.len() as i32);
        put_i32(&mut bytes, 42, summary_offset);
        bytes[46..50].copy_from_slice(&1000u32.to_le_bytes());
        put_i32(&mut bytes, 50, 1);
        bytes[54..58].copy_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(blob);
        bytes
    }

    #[test]
    fn parses_uncompressed_scene_image() {
        let mut blob = Vec::new();
        blob.extend_from_slice(b"bvcd");
        blob.push(4);
        blob.extend_from_slice(&123u32.to_le_bytes());
        let bytes = scene_image(&blob);
        let image = SceneImage::parse(&bytes).unwrap();
        assert_eq!(image.strings(), [&b"sound"[..]]);
        assert_eq!(image.string_utf8(0), Some("sound"));
        assert_eq!(image.entries()[0].summary.milliseconds, 1000);
        assert_eq!(
            image.entries()[0].blob,
            SceneBlob::Binary { source_crc: 123 }
        );
    }

    #[test]
    fn parses_lzma_envelope_and_rejects_bad_size() {
        let mut blob = Vec::new();
        blob.extend_from_slice(b"LZMA");
        blob.extend_from_slice(&100u32.to_le_bytes());
        blob.extend_from_slice(&3u32.to_le_bytes());
        blob.extend_from_slice(&[1, 2, 3, 4, 5]);
        blob.extend_from_slice(&[9, 8, 7]);
        assert!(matches!(
            SceneImage::parse(&scene_image(&blob)).unwrap().entries()[0].blob,
            SceneBlob::Lzma {
                uncompressed_size: 100,
                compressed_size: 3,
                ..
            }
        ));
        blob.pop();
        assert!(matches!(
            SceneImage::parse(&scene_image(&blob)),
            Err(Error::InvalidLzmaSize { .. })
        ));
    }

    #[test]
    fn rejects_unsorted_and_invalid_string_ids() {
        let blob = [b'b', b'v', b'c', b'd', 4, 0, 0, 0, 0];
        let mut bytes = scene_image(&blob);
        bytes[54..58].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            SceneImage::parse(&bytes),
            Err(Error::InvalidSoundStringId(1))
        ));
    }
}
