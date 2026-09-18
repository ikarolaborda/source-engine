//! Strict parser and deterministic writer for Source VPK versions 1 and 2.

use source_binary::{Reader, Writer};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

pub const VPK_SIGNATURE: u32 = 0x55aa_1234;
pub const VPK_VERSION_1: u32 = 1;
pub const VPK_VERSION_2: u32 = 2;
pub const VPK_EMBEDDED_ARCHIVE: u16 = 0x7fff;
const VPK_ENTRY_TERMINATOR: u16 = 0xffff;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_tree_size: usize,
    pub max_entries: usize,
    pub max_string_length: usize,
    pub max_file_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tree_size: 64 * 1024 * 1024,
            max_entries: 2_000_000,
            max_string_length: 4096,
            max_file_size: 4 * 1024 * 1024 * 1024usize,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    InvalidSignature(u32),
    UnsupportedVersion(u32),
    InvalidHeader(&'static str),
    InvalidUtf8(&'static str),
    InvalidPath(String),
    DuplicatePath(String),
    EntryLimitExceeded(usize),
    TreeTooLarge(usize),
    FileTooLarge {
        path: String,
        size: usize,
    },
    InvalidTerminator {
        path: String,
        value: u16,
    },
    InvalidRange {
        path: String,
        offset: u64,
        length: u64,
    },
    MissingEntry(String),
    MissingArchive(u16),
    CrcMismatch {
        path: String,
        expected: u32,
        actual: u32,
    },
    SizeOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::InvalidSignature(value) => write!(f, "invalid VPK signature 0x{value:08x}"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported VPK version {version}"),
            Self::InvalidHeader(message) => write!(f, "invalid VPK header: {message}"),
            Self::InvalidUtf8(field) => write!(f, "VPK {field} is not valid UTF-8"),
            Self::InvalidPath(path) => write!(f, "unsafe or invalid VPK path: {path:?}"),
            Self::DuplicatePath(path) => write!(f, "duplicate VPK path: {path}"),
            Self::EntryLimitExceeded(count) => {
                write!(f, "VPK entry count exceeds limit at {count}")
            }
            Self::TreeTooLarge(size) => write!(f, "VPK tree size {size} exceeds configured limit"),
            Self::FileTooLarge { path, size } => {
                write!(f, "VPK file {path} is too large ({size} bytes)")
            }
            Self::InvalidTerminator { path, value } => {
                write!(f, "VPK entry {path} has terminator 0x{value:04x}")
            }
            Self::InvalidRange {
                path,
                offset,
                length,
            } => {
                write!(f, "VPK entry {path} has invalid range {offset}+{length}")
            }
            Self::MissingEntry(path) => write!(f, "VPK entry not found: {path}"),
            Self::MissingArchive(index) => write!(f, "VPK chunk {index:03} is unavailable"),
            Self::CrcMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "VPK CRC mismatch for {path}: expected {expected:08x}, got {actual:08x}"
            ),
            Self::SizeOverflow => write!(f, "VPK size cannot be represented by the file format"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub tree_size: u32,
    pub file_data_size: u32,
    pub archive_md5_size: u32,
    pub other_md5_size: u32,
    pub signature_size: u32,
}

impl Header {
    pub fn encoded_size(&self) -> usize {
        if self.version == VPK_VERSION_1 {
            12
        } else {
            28
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub crc32: u32,
    pub preload: Vec<u8>,
    pub archive_index: u16,
    pub offset: u32,
    pub length: u32,
}

impl Entry {
    pub fn total_length(&self) -> u64 {
        self.preload.len() as u64 + u64::from(self.length)
    }
}

#[derive(Debug)]
pub struct Archive<'a> {
    header: Header,
    entries: Vec<Entry>,
    embedded_data: &'a [u8],
}

/// An owned VPK directory index suitable for long-lived filesystem mounts.
#[derive(Debug, Clone)]
pub struct OwnedArchive {
    header: Header,
    entries: Vec<Entry>,
    embedded_data: Vec<u8>,
}

impl OwnedArchive {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Self> {
        let archive = Archive::parse_with_limits(bytes, limits)?;
        Ok(Self {
            header: archive.header,
            entries: archive.entries,
            embedded_data: archive.embedded_data.to_vec(),
        })
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn entry(&self, path: &str) -> Option<&Entry> {
        normalize_path(path)
            .ok()
            .and_then(|path| self.entries.iter().find(|entry| entry.path == path))
    }

    pub fn read_file<F>(&self, path: &str, mut load_chunk: F) -> Result<Vec<u8>>
    where
        F: FnMut(u16) -> Result<Vec<u8>>,
    {
        let normalized = normalize_path(path)?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.path == normalized)
            .ok_or_else(|| Error::MissingEntry(normalized.clone()))?;

        let archive_bytes;
        let source = if entry.archive_index == VPK_EMBEDDED_ARCHIVE {
            self.embedded_data.as_slice()
        } else {
            archive_bytes = load_chunk(entry.archive_index)?;
            &archive_bytes
        };
        let range = checked_range(source, entry)?;
        let mut result = Vec::with_capacity(entry.total_length() as usize);
        result.extend_from_slice(&entry.preload);
        result.extend_from_slice(range);
        let actual = crc32(&result);
        if actual != entry.crc32 {
            return Err(Error::CrcMismatch {
                path: entry.path.clone(),
                expected: entry.crc32,
                actual,
            });
        }
        Ok(result)
    }
}

impl<'a> Archive<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        let mut reader = Reader::new(bytes);
        let signature = reader.read_u32_le()?;
        if signature != VPK_SIGNATURE {
            return Err(Error::InvalidSignature(signature));
        }
        let version = reader.read_u32_le()?;
        if version != VPK_VERSION_1 && version != VPK_VERSION_2 {
            return Err(Error::UnsupportedVersion(version));
        }
        let tree_size = reader.read_u32_le()?;
        let (file_data_size, archive_md5_size, other_md5_size, signature_size) =
            if version == VPK_VERSION_2 {
                (
                    reader.read_u32_le()?,
                    reader.read_u32_le()?,
                    reader.read_u32_le()?,
                    reader.read_u32_le()?,
                )
            } else {
                (0, 0, 0, 0)
            };
        let header = Header {
            version,
            tree_size,
            file_data_size,
            archive_md5_size,
            other_md5_size,
            signature_size,
        };

        let tree_size_usize = usize::try_from(tree_size).map_err(|_| Error::SizeOverflow)?;
        if tree_size_usize > limits.max_tree_size {
            return Err(Error::TreeTooLarge(tree_size_usize));
        }
        let tree = reader.take(tree_size_usize)?;
        let embedded_start = header
            .encoded_size()
            .checked_add(tree_size_usize)
            .ok_or(Error::SizeOverflow)?;
        let embedded_end = if version == VPK_VERSION_2 {
            embedded_start
                .checked_add(file_data_size as usize)
                .ok_or(Error::SizeOverflow)?
        } else {
            bytes.len()
        };
        let embedded_data = bytes
            .get(embedded_start..embedded_end)
            .ok_or(Error::InvalidHeader(
                "embedded data extends beyond the directory file",
            ))?;

        if version == VPK_VERSION_2 {
            let sections_end = embedded_end
                .checked_add(archive_md5_size as usize)
                .and_then(|value| value.checked_add(other_md5_size as usize))
                .and_then(|value| value.checked_add(signature_size as usize))
                .ok_or(Error::SizeOverflow)?;
            if sections_end > bytes.len() {
                return Err(Error::InvalidHeader(
                    "version 2 sections extend beyond the file",
                ));
            }
        }

        let entries = parse_tree(tree, limits)?;
        for entry in &entries {
            if entry.total_length() as usize > limits.max_file_size {
                return Err(Error::FileTooLarge {
                    path: entry.path.clone(),
                    size: entry.total_length() as usize,
                });
            }
            if entry.archive_index == VPK_EMBEDDED_ARCHIVE {
                checked_range(embedded_data, entry)?;
            }
        }

        Ok(Self {
            header,
            entries,
            embedded_data,
        })
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn entry(&self, path: &str) -> Option<&Entry> {
        normalize_path(path)
            .ok()
            .and_then(|path| self.entries.iter().find(|entry| entry.path == path))
    }

    pub fn read_file<F>(&self, path: &str, mut load_chunk: F) -> Result<Vec<u8>>
    where
        F: FnMut(u16) -> Result<Vec<u8>>,
    {
        let normalized = normalize_path(path)?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.path == normalized)
            .ok_or_else(|| Error::MissingEntry(normalized.clone()))?;

        let archive_bytes;
        let source = if entry.archive_index == VPK_EMBEDDED_ARCHIVE {
            self.embedded_data
        } else {
            archive_bytes = load_chunk(entry.archive_index)?;
            &archive_bytes
        };
        let range = checked_range(source, entry)?;
        let mut result = Vec::with_capacity(entry.total_length() as usize);
        result.extend_from_slice(&entry.preload);
        result.extend_from_slice(range);
        let actual = crc32(&result);
        if actual != entry.crc32 {
            return Err(Error::CrcMismatch {
                path: entry.path.clone(),
                expected: entry.crc32,
                actual,
            });
        }
        Ok(result)
    }
}

fn checked_range<'a>(bytes: &'a [u8], entry: &Entry) -> Result<&'a [u8]> {
    let offset = entry.offset as usize;
    let length = entry.length as usize;
    let end = offset.checked_add(length).ok_or(Error::InvalidRange {
        path: entry.path.clone(),
        offset: offset as u64,
        length: length as u64,
    })?;
    bytes.get(offset..end).ok_or_else(|| Error::InvalidRange {
        path: entry.path.clone(),
        offset: offset as u64,
        length: length as u64,
    })
}

fn parse_tree(tree: &[u8], limits: Limits) -> Result<Vec<Entry>> {
    let mut reader = Reader::new(tree);
    let mut entries = Vec::new();
    loop {
        let extension = read_tree_string(&mut reader, limits.max_string_length, "extension")?;
        if extension.is_empty() {
            break;
        }
        loop {
            let directory = read_tree_string(&mut reader, limits.max_string_length, "directory")?;
            if directory.is_empty() {
                break;
            }
            loop {
                let stem = read_tree_string(&mut reader, limits.max_string_length, "file name")?;
                if stem.is_empty() {
                    break;
                }
                if entries.len() >= limits.max_entries {
                    return Err(Error::EntryLimitExceeded(entries.len() + 1));
                }
                let path = join_tree_path(&directory, &stem, &extension)?;
                let crc32 = reader.read_u32_le()?;
                let preload_length = reader.read_u16_le()? as usize;
                let archive_index = reader.read_u16_le()?;
                let offset = reader.read_u32_le()?;
                let length = reader.read_u32_le()?;
                let terminator = reader.read_u16_le()?;
                if terminator != VPK_ENTRY_TERMINATOR {
                    return Err(Error::InvalidTerminator {
                        path,
                        value: terminator,
                    });
                }
                let preload = reader.take(preload_length)?.to_vec();
                entries.push(Entry {
                    path,
                    crc32,
                    preload,
                    archive_index,
                    offset,
                    length,
                });
            }
        }
    }
    if !reader.is_empty() {
        return Err(Error::InvalidHeader("directory tree has trailing bytes"));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    if entries.windows(2).any(|pair| pair[0].path == pair[1].path) {
        let duplicate = entries
            .windows(2)
            .find(|pair| pair[0].path == pair[1].path)
            .unwrap()[0]
            .path
            .clone();
        return Err(Error::DuplicatePath(duplicate));
    }
    Ok(entries)
}

fn read_tree_string(reader: &mut Reader<'_>, limit: usize, field: &'static str) -> Result<String> {
    let bytes = reader.read_cstr(limit)?;
    let string = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8(field))?;
    Ok(string.to_owned())
}

fn join_tree_path(directory: &str, stem: &str, extension: &str) -> Result<String> {
    let directory = if directory == " " { "" } else { directory };
    let extension = if extension == " " { "" } else { extension };
    let name = if extension.is_empty() {
        stem.to_owned()
    } else {
        format!("{stem}.{extension}")
    };
    normalize_path(if directory.is_empty() {
        name
    } else {
        format!("{directory}/{name}")
    })
}

fn split_tree_path(path: &str) -> Result<(String, String, String)> {
    let normalized = normalize_path(path)?;
    let (directory, file_name) = normalized
        .rsplit_once('/')
        .map_or((" ", normalized.as_str()), |(directory, name)| {
            (directory, name)
        });
    let (stem, extension) = file_name
        .rsplit_once('.')
        .filter(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
        .map_or((file_name, " "), |(stem, extension)| (stem, extension));
    Ok((extension.to_owned(), directory.to_owned(), stem.to_owned()))
}

pub fn normalize_path(path: impl AsRef<str>) -> Result<String> {
    let path = path.as_ref().replace('\\', "/");
    if path.is_empty() || path.starts_with('/') || path.contains('\0') {
        return Err(Error::InvalidPath(path));
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(Error::InvalidPath(path));
        }
        components.push(component);
    }
    Ok(components.join("/").to_ascii_lowercase())
}

#[derive(Debug, Default)]
pub struct Builder {
    files: BTreeMap<String, Vec<u8>>,
    limits: Limits,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            limits: Limits::default(),
        }
    }

    pub fn with_limits(limits: Limits) -> Self {
        Self {
            files: BTreeMap::new(),
            limits,
        }
    }

    pub fn add_file(&mut self, path: impl AsRef<str>, data: impl Into<Vec<u8>>) -> Result<()> {
        let path = normalize_path(path)?;
        let data = data.into();
        if data.len() > self.limits.max_file_size {
            return Err(Error::FileTooLarge {
                path,
                size: data.len(),
            });
        }
        if self.files.len() >= self.limits.max_entries && !self.files.contains_key(&path) {
            return Err(Error::EntryLimitExceeded(self.files.len() + 1));
        }
        if self.files.insert(path.clone(), data).is_some() {
            return Err(Error::DuplicatePath(path));
        }
        Ok(())
    }

    /// Produces a deterministic VPK v1 directory file with embedded payloads.
    pub fn build_v1_embedded(&self) -> Result<Vec<u8>> {
        type Tree<'a> = BTreeMap<String, BTreeMap<String, BTreeMap<String, (&'a str, &'a [u8])>>>;
        let mut tree: Tree<'_> = BTreeMap::new();
        for (path, data) in &self.files {
            let (extension, directory, stem) = split_tree_path(path)?;
            tree.entry(extension)
                .or_default()
                .entry(directory)
                .or_default()
                .insert(stem, (path, data));
        }

        let mut directory = Writer::new();
        let mut payload = Vec::new();
        for (extension, directories) in &tree {
            directory.write_cstr(extension.as_bytes());
            for (path, files) in directories {
                directory.write_cstr(path.as_bytes());
                for (stem, (full_path, data)) in files {
                    let offset = u32::try_from(payload.len()).map_err(|_| Error::SizeOverflow)?;
                    let length = u32::try_from(data.len()).map_err(|_| Error::FileTooLarge {
                        path: (*full_path).to_owned(),
                        size: data.len(),
                    })?;
                    directory.write_cstr(stem.as_bytes());
                    directory.write_u32_le(crc32(data));
                    directory.write_u16_le(0);
                    directory.write_u16_le(VPK_EMBEDDED_ARCHIVE);
                    directory.write_u32_le(offset);
                    directory.write_u32_le(length);
                    directory.write_u16_le(VPK_ENTRY_TERMINATOR);
                    payload.extend_from_slice(data);
                }
                directory.write_u8(0);
            }
            directory.write_u8(0);
        }
        directory.write_u8(0);
        if directory.position() > self.limits.max_tree_size {
            return Err(Error::TreeTooLarge(directory.position()));
        }
        let tree_size = u32::try_from(directory.position()).map_err(|_| Error::SizeOverflow)?;
        let mut output = Writer::with_capacity(12 + directory.position() + payload.len());
        output.write_u32_le(VPK_SIGNATURE);
        output.write_u32_le(VPK_VERSION_1);
        output.write_u32_le(tree_size);
        output.write_bytes(directory.as_slice());
        output.write_bytes(&payload);
        Ok(output.into_inner())
    }
}

pub fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = (value >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(value & 1)));
            }
            *slot = value;
        }
        table
    });
    let mut crc = !0u32;
    for byte in bytes {
        crc = (crc >> 8) ^ table[((crc ^ u32::from(*byte)) & 0xff) as usize];
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_standard_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn deterministic_embedded_round_trip() {
        let mut builder = Builder::new();
        builder
            .add_file("materials/Test.VMT", b"shader".to_vec())
            .unwrap();
        builder.add_file("README", b"hello".to_vec()).unwrap();
        builder
            .add_file("cfg/autoexec.cfg", b"echo rust".to_vec())
            .unwrap();
        let first = builder.build_v1_embedded().unwrap();
        let second = builder.build_v1_embedded().unwrap();
        assert_eq!(first, second);

        let archive = Archive::parse(&first).unwrap();
        assert_eq!(archive.header().version, 1);
        assert_eq!(archive.entries().len(), 3);
        assert_eq!(
            archive
                .read_file("materials/test.vmt", |_| unreachable!())
                .unwrap(),
            b"shader"
        );
        assert_eq!(
            archive.read_file("README", |_| unreachable!()).unwrap(),
            b"hello"
        );

        let owned = OwnedArchive::parse(&first).unwrap();
        assert_eq!(owned.header(), archive.header());
        assert_eq!(owned.entries(), archive.entries());
        assert_eq!(
            owned
                .read_file("materials/test.vmt", |_| unreachable!())
                .unwrap(),
            b"shader"
        );
    }

    #[test]
    fn golden_single_file_v1() {
        let mut builder = Builder::new();
        builder.add_file("a.txt", b"x".to_vec()).unwrap();
        let bytes = builder.build_v1_embedded().unwrap();
        let expected: &[u8] = &[
            0x34, 0x12, 0xaa, 0x55, 1, 0, 0, 0, 29, 0, 0, 0, b't', b'x', b't', 0, b' ', 0, b'a', 0,
            0x83, 0x16, 0xdc, 0x8c, 0, 0, 0xff, 0x7f, 0, 0, 0, 0, 1, 0, 0, 0, 0xff, 0xff, 0, 0, 0,
            b'x',
        ];
        assert_eq!(bytes, expected);
    }

    #[test]
    fn malformed_inputs_are_bounded() {
        assert!(matches!(
            Archive::parse(b"tiny"),
            Err(Error::InvalidSignature(_))
        ));

        let mut bad = Writer::new();
        bad.write_u32_le(VPK_SIGNATURE);
        bad.write_u32_le(VPK_VERSION_1);
        bad.write_u32_le(u32::MAX);
        assert!(matches!(
            Archive::parse(bad.as_slice()),
            Err(Error::TreeTooLarge(_))
        ));

        let mut bad_terminator = Builder::new();
        bad_terminator.add_file("a.txt", b"x".to_vec()).unwrap();
        let mut bytes = bad_terminator.build_v1_embedded().unwrap();
        bytes[36] = 0;
        assert!(matches!(
            Archive::parse(&bytes),
            Err(Error::InvalidTerminator { .. })
        ));
    }

    #[test]
    fn rejects_traversal_and_case_collisions() {
        let mut builder = Builder::new();
        assert!(builder.add_file("../secret", Vec::new()).is_err());
        builder.add_file("A.TXT", Vec::new()).unwrap();
        assert!(matches!(
            builder.add_file("a.txt", Vec::new()),
            Err(Error::DuplicatePath(_))
        ));
    }
}
