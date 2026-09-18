//! Bounded structural reader for Source `JSAV` containers and `VALV` map
//! state (`.hl1`) files.

use source_binary::Reader;
use std::fmt;

pub const SAVEGAME_VERSION: u32 = 0x73;
pub const JSAV_MAGIC: u32 = u32::from_le_bytes(*b"JSAV");
pub const VALV_MAGIC: u32 = u32::from_le_bytes(*b"VALV");
pub const EMBEDDED_NAME_SIZE: usize = 260;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_token_count: usize,
    pub max_token_bytes: usize,
    pub max_data_bytes: usize,
    pub max_embedded_files: usize,
    pub max_embedded_file_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 4 * 1024 * 1024 * 1024usize,
            max_token_count: 1_000_000,
            max_token_bytes: 64 * 1024 * 1024,
            max_data_bytes: 1024 * 1024 * 1024,
            max_embedded_files: 100_000,
            max_embedded_file_size: 2 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedFile<'a> {
    pub name: &'a [u8],
    pub data: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct SaveContainer<'a> {
    bytes: &'a [u8],
    pub version: u32,
    pub tokens: &'a [u8],
    pub token_count: usize,
    pub game_data: &'a [u8],
    pub declared_file_count: usize,
    pub files: Vec<EmbeddedFile<'a>>,
}

impl<'a> SaveContainer<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        check_file_size(bytes, limits)?;
        let mut reader = Reader::new(bytes);
        let magic = reader.read_u32_le()?;
        if magic != JSAV_MAGIC {
            return Err(Error::InvalidMagic {
                expected: JSAV_MAGIC,
                actual: magic,
            });
        }
        let version = reader.read_u32_le()?;
        if version != SAVEGAME_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let data_size = read_size("game data size", &mut reader, limits.max_data_bytes)?;
        let token_count = read_size("token count", &mut reader, limits.max_token_count)?;
        let token_size = read_size("token table size", &mut reader, limits.max_token_bytes)?;
        let tokens = reader.take(token_size)?;
        validate_tokens(tokens, token_count)?;
        let game_data = reader.take(data_size)?;

        // Current saves force GAME_HEADER::mapCount to zero and append the
        // actual compound-file count immediately before the directory.
        let declared_file_count = read_size(
            "embedded file count",
            &mut reader,
            limits.max_embedded_files,
        )?;
        let mut files = Vec::with_capacity(declared_file_count);
        for _ in 0..declared_file_count {
            let name_field = reader.take(EMBEDDED_NAME_SIZE)?;
            let end = name_field
                .iter()
                .position(|byte| *byte == 0)
                .ok_or(Error::UnterminatedEmbeddedName)?;
            if end == 0 {
                return Err(Error::EmptyEmbeddedName);
            }
            if name_field[end + 1..].iter().any(|byte| *byte != 0) {
                return Err(Error::NonzeroEmbeddedNamePadding);
            }
            let size = read_size(
                "embedded file size",
                &mut reader,
                limits.max_embedded_file_size,
            )?;
            if size == 0 {
                return Err(Error::EmptyEmbeddedFile);
            }
            files.push(EmbeddedFile {
                name: &name_field[..end],
                data: reader.take(size)?,
            });
        }
        if !reader.is_empty() {
            return Err(Error::TrailingData(reader.remaining()));
        }

        Ok(Self {
            bytes,
            version,
            tokens,
            token_count,
            game_data,
            declared_file_count,
            files,
        })
    }

    pub fn original_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Debug, Clone)]
pub struct MapState<'a> {
    bytes: &'a [u8],
    pub version: u32,
    pub tokens: &'a [u8],
    pub token_count: usize,
    pub data_headers: &'a [u8],
    pub data: &'a [u8],
}

impl<'a> MapState<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        check_file_size(bytes, limits)?;
        let mut reader = Reader::new(bytes);
        let magic = reader.read_u32_le()?;
        if magic != VALV_MAGIC {
            return Err(Error::InvalidMagic {
                expected: VALV_MAGIC,
                actual: magic,
            });
        }
        let version = reader.read_u32_le()?;
        if version != SAVEGAME_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let token_size = read_size("token table size", &mut reader, limits.max_token_bytes)?;
        let token_count = read_size("token count", &mut reader, limits.max_token_count)?;
        let data_header_size = read_size("data header size", &mut reader, limits.max_data_bytes)?;
        let data_size = read_size("data size", &mut reader, limits.max_data_bytes)?;
        let section_size = token_size
            .checked_add(data_header_size)
            .and_then(|value| value.checked_add(data_size))
            .ok_or(Error::SectionSizeOverflow)?;
        if section_size != reader.remaining() {
            return Err(Error::SectionSizeMismatch {
                declared: section_size,
                actual: reader.remaining(),
            });
        }
        let tokens = reader.take(token_size)?;
        validate_tokens(tokens, token_count)?;
        let data_headers = reader.take(data_header_size)?;
        let data = reader.take(data_size)?;
        debug_assert!(reader.is_empty());
        Ok(Self {
            bytes,
            version,
            tokens,
            token_count,
            data_headers,
            data,
        })
    }

    pub fn original_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// Composes a `VALV` map state from the three sections a save produces.
///
/// The token table is the NUL-terminated names in order, and `token_count`
/// must match what it contains, because the reader on the other side sizes its
/// symbol table from that count alone.
pub fn encode_map_state(
    tokens: &[u8],
    token_count: usize,
    data_headers: &[u8],
    data: &[u8],
) -> Result<Vec<u8>> {
    encode_map_state_with_limits(tokens, token_count, data_headers, data, Limits::default())
}

pub fn encode_map_state_with_limits(
    tokens: &[u8],
    token_count: usize,
    data_headers: &[u8],
    data: &[u8],
    limits: Limits,
) -> Result<Vec<u8>> {
    check_limit("token count", token_count, limits.max_token_count)?;
    check_limit("token table size", tokens.len(), limits.max_token_bytes)?;
    check_limit(
        "data header size",
        data_headers.len(),
        limits.max_data_bytes,
    )?;
    check_limit("data size", data.len(), limits.max_data_bytes)?;
    validate_tokens(tokens, token_count)?;

    let mut writer = source_binary::Writer::new();
    writer.write_u32_le(VALV_MAGIC);
    writer.write_u32_le(SAVEGAME_VERSION);
    for (what, value) in [
        ("token table size", tokens.len()),
        ("token count", token_count),
        ("data header size", data_headers.len()),
        ("data size", data.len()),
    ] {
        writer.write_i32_le(i32::try_from(value).map_err(|_| Error::LimitExceeded {
            what,
            value,
            limit: i32::MAX as usize,
        })?);
    }
    writer.write_bytes(tokens);
    writer.write_bytes(data_headers);
    writer.write_bytes(data);

    let bytes = writer.into_inner();
    check_file_size(&bytes, limits)?;
    Ok(bytes)
}

/// Composes the leading `JSAV` container: magic, version, game data size,
/// token count, token table size, the token table, and the game data.
///
/// The embedded map states a save carries are appended afterwards by the
/// caller copying them in, so this covers the header and both leading
/// sections rather than the whole file.
pub fn encode_save_container_header(
    tokens: &[u8],
    token_count: usize,
    game_data: &[u8],
) -> Result<Vec<u8>> {
    let limits = Limits::default();
    check_limit("token count", token_count, limits.max_token_count)?;
    check_limit("token table size", tokens.len(), limits.max_token_bytes)?;
    check_limit("game data size", game_data.len(), limits.max_data_bytes)?;
    validate_tokens(tokens, token_count)?;

    let mut writer = source_binary::Writer::new();
    writer.write_u32_le(JSAV_MAGIC);
    writer.write_u32_le(SAVEGAME_VERSION);
    for (what, value) in [
        ("game data size", game_data.len()),
        ("token count", token_count),
        ("token table size", tokens.len()),
    ] {
        writer.write_i32_le(i32::try_from(value).map_err(|_| Error::LimitExceeded {
            what,
            value,
            limit: i32::MAX as usize,
        })?);
    }
    writer.write_bytes(tokens);
    writer.write_bytes(game_data);
    Ok(writer.into_inner())
}

fn check_limit(what: &'static str, value: usize, limit: usize) -> Result<()> {
    if value > limit {
        return Err(Error::LimitExceeded { what, value, limit });
    }
    Ok(())
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    InvalidMagic {
        expected: u32,
        actual: u32,
    },
    UnsupportedVersion(u32),
    InvalidSize {
        what: &'static str,
        value: i32,
    },
    LimitExceeded {
        what: &'static str,
        value: usize,
        limit: usize,
    },
    TokenCountMismatch {
        declared: usize,
        actual: usize,
    },
    TokenTableNotTerminated,
    UnterminatedEmbeddedName,
    EmptyEmbeddedName,
    NonzeroEmbeddedNamePadding,
    EmptyEmbeddedFile,
    SectionSizeOverflow,
    SectionSizeMismatch {
        declared: usize,
        actual: usize,
    },
    TrailingData(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::InvalidMagic { expected, actual } => write!(
                f,
                "invalid save magic {:08x}; expected {:08x}",
                actual, expected
            ),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported save-game version {version:#x}")
            }
            Self::InvalidSize { what, value } => write!(f, "invalid save {what} {value}"),
            Self::LimitExceeded { what, value, limit } => {
                write!(f, "save {what} {value} exceeds limit {limit}")
            }
            Self::TokenCountMismatch { declared, actual } => write!(
                f,
                "save token table declares {declared} entries but contains {actual}"
            ),
            Self::TokenTableNotTerminated => write!(f, "save token table is not NUL-terminated"),
            Self::UnterminatedEmbeddedName => {
                write!(f, "save embedded filename is not NUL-terminated")
            }
            Self::EmptyEmbeddedName => write!(f, "save embedded filename is empty"),
            Self::NonzeroEmbeddedNamePadding => {
                write!(f, "save embedded filename has nonzero padding")
            }
            Self::EmptyEmbeddedFile => write!(f, "save embedded file is empty"),
            Self::SectionSizeOverflow => write!(f, "save section sizes overflow"),
            Self::SectionSizeMismatch { declared, actual } => write!(
                f,
                "save sections declare {declared} bytes but {actual} remain"
            ),
            Self::TrailingData(size) => write!(f, "save file has {size} trailing bytes"),
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

fn check_file_size(bytes: &[u8], limits: Limits) -> Result<()> {
    if bytes.len() > limits.max_file_size {
        return Err(Error::LimitExceeded {
            what: "file size",
            value: bytes.len(),
            limit: limits.max_file_size,
        });
    }
    Ok(())
}

fn read_size(what: &'static str, reader: &mut Reader<'_>, limit: usize) -> Result<usize> {
    let raw = reader.read_i32_le()?;
    let value = usize::try_from(raw).map_err(|_| Error::InvalidSize { what, value: raw })?;
    if value > limit {
        return Err(Error::LimitExceeded { what, value, limit });
    }
    Ok(value)
}

fn validate_tokens(bytes: &[u8], declared: usize) -> Result<()> {
    if declared != 0 && bytes.last() != Some(&0) {
        return Err(Error::TokenTableNotTerminated);
    }
    let actual = bytes.iter().filter(|byte| **byte == 0).count();
    if actual != declared {
        return Err(Error::TokenCountMismatch { declared, actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_binary::Writer;

    fn map_state() -> Vec<u8> {
        let mut writer = Writer::new();
        writer.write_u32_le(VALV_MAGIC);
        writer.write_u32_le(SAVEGAME_VERSION);
        writer.write_i32_le(4);
        writer.write_i32_le(2);
        writer.write_i32_le(2);
        writer.write_i32_le(3);
        writer.write_bytes(b"a\0b\0");
        writer.write_bytes(&[1, 2]);
        writer.write_bytes(&[3, 4, 5]);
        writer.into_inner()
    }

    #[test]
    fn parses_map_state_sections() {
        let bytes = map_state();
        let state = MapState::parse(&bytes).unwrap();
        assert_eq!(state.token_count, 2);
        assert_eq!(state.tokens, b"a\0b\0");
        assert_eq!(state.data_headers, &[1, 2]);
        assert_eq!(state.data, &[3, 4, 5]);
        assert_eq!(state.original_bytes(), bytes);
    }

    #[test]
    fn encodes_map_state_the_reader_accepts() {
        let encoded = encode_map_state(b"a\0b\0", 2, &[1, 2], &[3, 4, 5]).unwrap();
        assert_eq!(encoded, map_state());

        let state = MapState::parse(&encoded).unwrap();
        assert_eq!(state.token_count, 2);
        assert_eq!(state.tokens, b"a\0b\0");
        assert_eq!(state.data_headers, &[1, 2]);
        assert_eq!(state.data, &[3, 4, 5]);

        // An empty save still has to carry its four section sizes.
        let empty = encode_map_state(b"", 0, &[], &[]).unwrap();
        assert_eq!(empty.len(), 24);
        assert!(MapState::parse(&empty).is_ok());

        assert!(matches!(
            encode_map_state(b"a\0", 2, &[], &[]),
            Err(Error::TokenCountMismatch {
                declared: 2,
                actual: 1,
            })
        ));
        assert!(matches!(
            encode_map_state(b"a", 1, &[], &[]),
            Err(Error::TokenTableNotTerminated)
        ));
    }

    #[test]
    fn encodes_the_save_container_header_the_reader_accepts() {
        let header = encode_save_container_header(b"x\0", 1, &[7, 8, 9]).unwrap();

        // The same leading bytes the parser test builds by hand, followed by
        // an embedded map state the caller appends.
        let mut writer = Writer::new();
        writer.write_bytes(&header);
        writer.write_i32_le(1);
        let embedded = map_state();
        let mut name = [0; EMBEDDED_NAME_SIZE];
        name[..11].copy_from_slice(b"sample.hl1\0");
        writer.write_bytes(&name);
        writer.write_i32_le(embedded.len() as i32);
        writer.write_bytes(&embedded);

        let bytes = writer.into_inner();
        let container = SaveContainer::parse(&bytes).unwrap();
        assert_eq!(container.token_count, 1);
        assert_eq!(container.tokens, b"x\0");
        assert_eq!(container.game_data, &[7, 8, 9]);
        assert_eq!(container.files.len(), 1);

        assert!(matches!(
            encode_save_container_header(b"x\0", 2, &[]),
            Err(Error::TokenCountMismatch {
                declared: 2,
                actual: 1,
            })
        ));
    }

    #[test]
    fn parses_save_container_and_embedded_file() {
        let embedded = map_state();
        let mut writer = Writer::new();
        writer.write_u32_le(JSAV_MAGIC);
        writer.write_u32_le(SAVEGAME_VERSION);
        writer.write_i32_le(3);
        writer.write_i32_le(1);
        writer.write_i32_le(2);
        writer.write_bytes(b"x\0");
        writer.write_bytes(&[7, 8, 9]);
        writer.write_i32_le(1);
        let mut name = [0; EMBEDDED_NAME_SIZE];
        name[..11].copy_from_slice(b"sample.hl1\0");
        writer.write_bytes(&name);
        writer.write_i32_le(embedded.len() as i32);
        writer.write_bytes(&embedded);
        let bytes = writer.into_inner();

        let save = SaveContainer::parse(&bytes).unwrap();
        assert_eq!(save.game_data, &[7, 8, 9]);
        assert_eq!(save.declared_file_count, 1);
        assert_eq!(save.files[0].name, b"sample.hl1");
        MapState::parse(save.files[0].data).unwrap();
        assert_eq!(save.original_bytes(), bytes);
    }

    #[test]
    fn rejects_bad_token_and_section_counts() {
        let mut bytes = map_state();
        bytes[12..16].copy_from_slice(&3i32.to_le_bytes());
        assert!(matches!(
            MapState::parse(&bytes),
            Err(Error::TokenCountMismatch {
                declared: 3,
                actual: 2
            })
        ));

        let mut bytes = map_state();
        bytes.pop();
        assert!(matches!(
            MapState::parse(&bytes),
            Err(Error::SectionSizeMismatch { .. })
        ));
    }
}
