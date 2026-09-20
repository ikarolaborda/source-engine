//! Reader for the ZIP archive a map carries in its pakfile lump.
//!
//! A compiled map embeds the content that only it uses. Most of that is the
//! cubemap materials VBSP generates: for every surface whose material
//! reflects its surroundings, it writes a small `patch` material named after
//! the surface's position, pointing at a cubemap baked for that spot. Those
//! materials exist nowhere else, so a map's world cannot be textured without
//! reading this archive first.
//!
//! Supports the native Source reader's stored and LZMA (method 14) entries.
//! Split, encrypted, ZIP64 and other compression formats are rejected.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Write};

/// `PK\x03\x04`, before each entry's own header.
const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
/// `PK\x01\x02`, before each central directory record.
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
/// `PK\x05\x06`, at the end of the archive.
const END_OF_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;

const LOCAL_HEADER_SIZE: usize = 30;
const CENTRAL_HEADER_SIZE: usize = 46;
const END_OF_DIRECTORY_SIZE: usize = 22;

const METHOD_STORED: u16 = 0;
const METHOD_LZMA: u16 = 14;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    pub max_entry_size: usize,
    pub max_name_length: usize,
    pub max_archive_size: usize,
    pub max_dictionary_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 65_536,
            max_entry_size: 256 * 1024 * 1024,
            max_name_length: 1024,
            max_archive_size: 512 * 1024 * 1024,
            max_dictionary_size: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Lowercased and forward-slashed, so a lookup matches however the
    /// compiler happened to write the name.
    pub path: String,
    pub crc32: u32,
    pub compressed_length: u32,
    pub length: u32,
    pub method: u16,
    /// Offset of this entry's local header within the archive.
    pub header_offset: u32,
    flags: u16,
    data_offset: usize,
}

impl Entry {
    /// Validated payload offset relative to the start of the ZIP, not its BSP.
    pub fn data_offset(&self) -> u64 {
        self.data_offset as u64
    }

    pub fn is_stored(&self) -> bool {
        self.method == METHOD_STORED
    }
}

/// An owned, metadata-only index. Parsing does not retain or copy archive bytes.
/// Entries have stable lexicographic indices, not collision-prone name hashes.
#[derive(Debug)]
pub struct Index {
    entries: Vec<Entry>,
}

impl Index {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let entries = Pak::parse_entries(bytes, Limits::default())?
            .into_values()
            // Native preload data is an optional cache, not a visible file.
            .filter(|entry| entry.path != "__preload_section.pre")
            .collect();
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn find(&self, path: &str) -> Option<usize> {
        if path.starts_with(['/', '\\']) || path.contains(['\0', ':']) {
            return None;
        }
        let path = path.replace('\\', "/").to_ascii_lowercase();
        let mut parts = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                _ => parts.push(part),
            }
        }
        let path = parts.join("/");
        self.entries
            .binary_search_by(|entry| entry.path.cmp(&path))
            .ok()
    }
}

/// A map's embedded archive, read from the pakfile lump.
#[derive(Debug, Clone)]
pub struct Pak {
    entries: BTreeMap<String, Entry>,
    bytes: Vec<u8>,
    max_dictionary_size: usize,
}

impl Pak {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Self> {
        Ok(Self {
            entries: Self::parse_entries(bytes, limits)?,
            bytes: bytes.to_vec(),
            max_dictionary_size: limits.max_dictionary_size,
        })
    }

    fn parse_entries(bytes: &[u8], limits: Limits) -> Result<BTreeMap<String, Entry>> {
        if bytes.len() > limits.max_archive_size {
            return Err(Error::ArchiveTooLarge);
        }
        // An empty lump is how a map with no embedded content states it,
        // which is not an error.
        if bytes.is_empty() {
            return Ok(BTreeMap::new());
        }

        let directory = find_end_of_directory(bytes)?;
        if directory.count > limits.max_entries {
            return Err(Error::TooManyEntries {
                limit: limits.max_entries,
            });
        }
        let mut entries = BTreeMap::new();
        let mut at = directory.offset;

        for index in 0..directory.count {
            let header = bytes[..directory.end]
                .get(at..at.checked_add(CENTRAL_HEADER_SIZE).ok_or(Error::Overflow)?)
                .ok_or(Error::DirectoryTruncated { index })?;
            if read_u32(header, 0) != CENTRAL_HEADER_SIGNATURE {
                return Err(Error::InvalidDirectoryRecord { index });
            }

            let method = read_u16(header, 10);
            let flags = read_u16(header, 8);
            if flags & !0x080a != 0 || read_u16(header, 34) != 0 {
                return Err(Error::UnsupportedLayout);
            }
            let crc32 = read_u32(header, 16);
            let compressed_length = read_u32(header, 20);
            let length = read_u32(header, 24);
            let name_length = read_u16(header, 28) as usize;
            let extra_length = read_u16(header, 30) as usize;
            let comment_length = read_u16(header, 32) as usize;
            let header_offset = read_u32(header, 42);

            if name_length == 0 || name_length > limits.max_name_length {
                return Err(Error::InvalidName { index });
            }
            if length as usize > limits.max_entry_size {
                return Err(Error::EntryTooLarge {
                    size: length as usize,
                    limit: limits.max_entry_size,
                });
            }

            let name_at = at.checked_add(CENTRAL_HEADER_SIZE).ok_or(Error::Overflow)?;
            let name_end = name_at.checked_add(name_length).ok_or(Error::Overflow)?;
            let raw = bytes
                .get(name_at..name_end)
                .ok_or(Error::DirectoryTruncated { index })?;
            let path =
                normalize(std::str::from_utf8(raw).map_err(|_| Error::InvalidName { index })?);
            if raw.starts_with(b"/")
                || raw.starts_with(b"\\")
                || path.contains(['\0', ':'])
                || path
                    .trim_end_matches('/')
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(Error::InvalidName { index });
            }

            at = name_end
                .checked_add(extra_length)
                .and_then(|next| next.checked_add(comment_length))
                .ok_or(Error::Overflow)?;
            if at > directory.end {
                return Err(Error::DirectoryTruncated { index });
            }
            if !matches!(method, METHOD_STORED | METHOD_LZMA) {
                return Err(Error::UnsupportedMethod { path, method });
            }
            if method == METHOD_STORED && compressed_length != length {
                return Err(Error::InvalidLocalHeader(path));
            }
            let offset = header_offset as usize;
            let local = bytes[..directory.offset]
                .get(
                    offset
                        ..offset
                            .checked_add(LOCAL_HEADER_SIZE)
                            .ok_or(Error::Overflow)?,
                )
                .ok_or_else(|| Error::EntryOutOfRange(path.clone()))?;
            if read_u32(local, 0) != LOCAL_HEADER_SIGNATURE
                || read_u16(local, 6) != flags
                || read_u16(local, 8) != method
            {
                return Err(Error::InvalidLocalHeader(path));
            }
            // With a data descriptor, the central record supplies CRC/sizes.
            if flags & 8 == 0
                && (read_u32(local, 14) != crc32
                    || read_u32(local, 18) != compressed_length
                    || read_u32(local, 22) != length)
            {
                return Err(Error::InvalidLocalHeader(path));
            }
            let name_at = offset + LOCAL_HEADER_SIZE;
            let name_end = name_at
                .checked_add(read_u16(local, 26) as usize)
                .ok_or(Error::Overflow)?;
            if bytes.get(name_at..name_end) != Some(raw) {
                return Err(Error::InvalidLocalHeader(path));
            }
            let data_offset = name_end
                .checked_add(read_u16(local, 28) as usize)
                .ok_or(Error::Overflow)?;
            if data_offset
                .checked_add(compressed_length as usize)
                .ok_or(Error::Overflow)?
                > directory.offset
            {
                return Err(Error::EntryOutOfRange(path));
            }

            // A directory entry has no content and would otherwise shadow a
            // file lookup of the same name.
            if path.is_empty() || path.ends_with('/') {
                continue;
            }

            entries.insert(
                path.clone(),
                Entry {
                    path,
                    crc32,
                    compressed_length,
                    length,
                    method,
                    header_offset,
                    flags,
                    data_offset,
                },
            );
        }
        if at != directory.end {
            return Err(Error::InvalidDirectoryRecord {
                index: directory.count,
            });
        }

        Ok(entries)
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Looks up an entry however the caller spelled the path.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.get(&normalize(path))
    }

    /// Every entry beneath a directory prefix, in path order.
    ///
    /// The archive stores no directories of its own, so a directory exists
    /// exactly when something is under it.
    pub fn entries_under(&self, prefix: &str) -> impl Iterator<Item = &Entry> {
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            let mut prefix = normalize(prefix);
            if !prefix.ends_with('/') {
                prefix.push('/');
            }
            prefix
        };
        self.entries
            .range(prefix.clone()..)
            .take_while(move |(path, _)| path.starts_with(&prefix))
            .map(|(_, entry)| entry)
    }

    pub fn has_directory(&self, prefix: &str) -> bool {
        self.entries_under(prefix).next().is_some()
    }

    /// The contents of one entry, checked against the CRC the archive
    /// recorded for it.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let entry = self
            .entry(path)
            .ok_or_else(|| Error::MissingEntry(normalize(path)))?;
        // All ranges and local/central agreement were validated at mount time.
        let compressed =
            &self.bytes[entry.data_offset..entry.data_offset + entry.compressed_length as usize];
        entry.decode_payload(compressed, self.max_dictionary_size)
    }
}

impl Entry {
    /// Decode exactly this entry's compressed bytes, checking size, dictionary
    /// bound and CRC. Shared by owned archives and file-backed index readers.
    pub fn decode_payload(&self, compressed: &[u8], max_dictionary_size: usize) -> Result<Vec<u8>> {
        let entry = self;
        if compressed.len() != entry.compressed_length as usize
            || !matches!(entry.method, METHOD_STORED | METHOD_LZMA)
            || (entry.is_stored() && compressed.len() != entry.length as usize)
        {
            return Err(Error::InvalidCompressedData(entry.path.clone()));
        }
        let data = if entry.is_stored() {
            compressed.to_vec()
        } else {
            let bad = || Error::InvalidCompressedData(entry.path.clone());
            if compressed.len() < 9
                || read_u16(compressed, 2) != 5
                || read_u32(compressed, 5) as usize > max_dictionary_size
            {
                return Err(bad());
            }
            let mut input = &compressed[4..]; // 5 properties, then raw LZMA
            let mut output = BoundedOutput {
                bytes: Vec::new(),
                limit: entry.length as usize,
            };
            let options = lzma_rs::decompress::Options {
                // ZIP bit 1 requires an end marker; otherwise size terminates it.
                unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(
                    if entry.flags & 2 != 0 {
                        None
                    } else {
                        Some(entry.length as u64)
                    },
                ),
                memlimit: Some(max_dictionary_size),
                allow_incomplete: false,
            };
            lzma_rs::lzma_decompress_with_options(&mut input, &mut output, &options)
                .map_err(|_| bad())?;
            if output.bytes.len() != entry.length as usize || !input.is_empty() {
                return Err(bad());
            }
            output.bytes
        };
        let actual = source_vpk::crc32(&data);
        if actual != entry.crc32 {
            return Err(Error::CrcMismatch {
                path: entry.path.clone(),
                expected: entry.crc32,
                actual,
            });
        }
        Ok(data)
    }
}

struct Directory {
    offset: usize,
    count: usize,
    end: usize,
}

struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedOutput {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.len() > self.limit - self.bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LZMA output exceeds ZIP size",
            ));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn find_end_of_directory(bytes: &[u8]) -> Result<Directory> {
    // The record sits at the end but may be followed by a comment, so it is
    // searched for backwards.
    let start = bytes
        .len()
        .checked_sub(END_OF_DIRECTORY_SIZE)
        .ok_or(Error::NoEndOfDirectory)?;
    let found = (start.saturating_sub(u16::MAX as usize)..=start)
        .rev()
        .find(|at| {
            read_u32(bytes, *at) == END_OF_DIRECTORY_SIGNATURE
                && *at + END_OF_DIRECTORY_SIZE + read_u16(bytes, *at + 20) as usize == bytes.len()
        })
        .ok_or(Error::NoEndOfDirectory)?;

    let record = &bytes[found..];
    let count = read_u16(record, 10) as usize;
    let size = read_u32(record, 12) as usize;
    let offset = read_u32(record, 16) as usize;
    if read_u16(record, 4) != 0
        || read_u16(record, 6) != 0
        || read_u16(record, 8) as usize != count
        || count == u16::MAX as usize
        || size == u32::MAX as usize
    {
        return Err(Error::UnsupportedLayout);
    }

    let end = offset.checked_add(size).ok_or(Error::Overflow)?;
    if end != found {
        return Err(Error::DirectoryOutOfRange { offset, size });
    }
    Ok(Directory { offset, count, end })
}

/// Reduces a stored name to the one form a lookup uses.
///
/// The compiler writes these with whatever case and separator the map's
/// author used, and callers ask for the lowercase forward-slashed path the
/// rest of the content system speaks.
fn normalize(path: &str) -> String {
    path.trim_start_matches(['/', '\\'])
        .chars()
        .map(|character| match character {
            '\\' => '/',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    bytes
        .get(at..at + 2)
        .map(|slice| u16::from_le_bytes(slice.try_into().expect("two bytes")))
        .unwrap_or(0)
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .map(|slice| u32::from_le_bytes(slice.try_into().expect("four bytes")))
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    ArchiveTooLarge,
    UnsupportedLayout,
    InvalidCompressedData(String),
    NoEndOfDirectory,
    DirectoryOutOfRange {
        offset: usize,
        size: usize,
    },
    DirectoryTruncated {
        index: usize,
    },
    InvalidDirectoryRecord {
        index: usize,
    },
    InvalidName {
        index: usize,
    },
    TooManyEntries {
        limit: usize,
    },
    EntryTooLarge {
        size: usize,
        limit: usize,
    },
    MissingEntry(String),
    EntryOutOfRange(String),
    InvalidLocalHeader(String),
    /// A method other than stored or LZMA.
    UnsupportedMethod {
        path: String,
        method: u16,
    },
    CrcMismatch {
        path: String,
        expected: u32,
        actual: u32,
    },
    Overflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArchiveTooLarge => write!(f, "pakfile exceeds archive size limit"),
            Self::UnsupportedLayout => write!(f, "unsupported split, encrypted or ZIP64 pakfile"),
            Self::InvalidCompressedData(path) => {
                write!(f, "invalid or excessive LZMA data in {path}")
            }
            Self::NoEndOfDirectory => write!(f, "pakfile has no end-of-directory record"),
            Self::DirectoryOutOfRange { offset, size } => {
                write!(
                    f,
                    "pakfile directory of {size} bytes at {offset} is out of range"
                )
            }
            Self::DirectoryTruncated { index } => {
                write!(f, "pakfile directory ends inside record {index}")
            }
            Self::InvalidDirectoryRecord { index } => {
                write!(f, "pakfile directory record {index} is not one")
            }
            Self::InvalidName { index } => {
                write!(f, "pakfile directory record {index} has an unusable name")
            }
            Self::TooManyEntries { limit } => {
                write!(f, "pakfile holds more than {limit} entries")
            }
            Self::EntryTooLarge { size, limit } => {
                write!(f, "pakfile entry of {size} bytes exceeds limit {limit}")
            }
            Self::MissingEntry(path) => write!(f, "pakfile holds no {path}"),
            Self::EntryOutOfRange(path) => write!(f, "pakfile entry {path} is out of range"),
            Self::InvalidLocalHeader(path) => {
                write!(f, "pakfile entry {path} has no local header")
            }
            Self::UnsupportedMethod { path, method } => {
                write!(f, "pakfile entry {path} uses compression method {method}")
            }
            Self::CrcMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "pakfile entry {path} has CRC {actual:#010x}, expected {expected:#010x}"
            ),
            Self::Overflow => write!(f, "pakfile size arithmetic overflow"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an archive the way VBSP writes one: every entry stored, with a
    /// central directory and an end record.
    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut directory = Vec::new();

        for (name, data) in files {
            let offset = bytes.len() as u32;
            let crc = source_vpk::crc32(data);

            bytes.extend_from_slice(&LOCAL_HEADER_SIGNATURE.to_le_bytes());
            bytes.extend_from_slice(&[0; 4]); // version, flags
            bytes.extend_from_slice(&METHOD_STORED.to_le_bytes());
            bytes.extend_from_slice(&[0; 4]); // time, date
            bytes.extend_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes()); // extra
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(data);

            directory.extend_from_slice(&CENTRAL_HEADER_SIGNATURE.to_le_bytes());
            directory.extend_from_slice(&[0; 6]); // versions, flags
            directory.extend_from_slice(&METHOD_STORED.to_le_bytes());
            directory.extend_from_slice(&[0; 4]); // time, date
            directory.extend_from_slice(&crc.to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
            directory.extend_from_slice(&[0; 4]); // external attributes
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }

        let directory_offset = bytes.len() as u32;
        let directory_size = directory.len() as u32;
        bytes.extend_from_slice(&directory);
        bytes.extend_from_slice(&END_OF_DIRECTORY_SIGNATURE.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]); // disk numbers
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&directory_size.to_le_bytes());
        bytes.extend_from_slice(&directory_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes()); // comment
        bytes
    }

    #[test]
    fn detached_entry_decoder_checks_exact_payload_size_method_and_crc() {
        let bytes = archive(&[("a.txt", b"payload")]);
        let index = Index::parse(&bytes).unwrap();
        let entry = &index.entries()[0];
        assert_eq!(entry.decode_payload(b"payload", 1024).unwrap(), b"payload");
        for data in [b"payloa".as_slice(), b"payload!", b"changed"] {
            assert!(entry.decode_payload(data, 1024).is_err());
        }
        let mut modified = entry.clone();
        modified.length += 1;
        assert!(modified.decode_payload(b"payload", 1024).is_err());
        modified.length = entry.length;
        modified.method = 8;
        assert!(modified.decode_payload(b"payload", 1024).is_err());
    }

    #[test]
    fn metadata_index_owns_names_and_canonical_lookup_without_payloads() {
        let mut bytes = archive(&[
            ("__preload_section.pre", b"optional cache"),
            ("Z/Second.vmt", b"two"),
            ("A/First.vmt", b"one"),
        ]);
        let index = Index::parse(&bytes).unwrap();
        assert_eq!(index.entries().len(), 2);
        assert_eq!(index.find("a/./deeper/../FIRST.vmt"), Some(0));
        assert_eq!(index.find("Z\\Second.vmt"), Some(1));
        for path in [
            "../a/first.vmt",
            "/a/first.vmt",
            "c:a/first.vmt",
            "__preload_section.pre",
        ] {
            assert_eq!(index.find(path), None);
        }
        let entry = &index.entries()[0];
        assert_eq!(&bytes[entry.data_offset() as usize..][..3], b"one");
        bytes.fill(0);
        drop(bytes);
        assert_eq!(index.entries()[0].path, "a/first.vmt");
        assert_eq!(index.find("A/FIRST.VMT"), Some(0));
    }

    #[test]
    fn metadata_index_uses_last_canonical_duplicate_and_checks_bounds() {
        let bytes = archive(&[("A.txt", b"old"), ("a.txt", b"new payload")]);
        let index = Index::parse(&bytes).unwrap();
        assert_eq!(index.entries().len(), 1);
        assert_eq!(index.entries()[0].length, 11);
        assert!(Index::parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(Index::parse(&[]).unwrap().entries().is_empty());
        assert_eq!(Limits::default().max_archive_size, 512 * 1024 * 1024);
    }

    #[test]
    fn reads_the_entries_a_map_embeds() {
        let bytes = archive(&[
            ("materials/maps/test/brick_0_0_0.vmt", b"\"patch\" { }"),
            ("materials/maps/test/c0_0_0.vtf", b"VTF\0payload"),
        ]);
        let pak = Pak::parse(&bytes).unwrap();

        assert_eq!(pak.len(), 2);
        assert_eq!(
            pak.read("materials/maps/test/brick_0_0_0.vmt").unwrap(),
            b"\"patch\" { }"
        );
        assert_eq!(
            pak.read("materials/maps/test/c0_0_0.vtf").unwrap(),
            b"VTF\0payload"
        );
    }

    #[test]
    fn finds_an_entry_however_the_caller_spells_it() {
        // The compiler writes the name in the author's case and sometimes
        // with backslashes, while callers ask with the lowercase
        // forward-slashed path the content system uses.
        let bytes = archive(&[("Materials\\Maps\\Test\\Brick.vmt", b"data")]);
        let pak = Pak::parse(&bytes).unwrap();

        assert_eq!(
            pak.entries().next().map(|entry| entry.path.as_str()),
            Some("materials/maps/test/brick.vmt")
        );
        assert_eq!(pak.read("materials/maps/test/brick.vmt").unwrap(), b"data");
        assert_eq!(pak.read("Materials/Maps/Test/Brick.vmt").unwrap(), b"data");
        assert_eq!(
            pak.read("materials\\maps\\test\\brick.vmt").unwrap(),
            b"data"
        );
    }

    #[test]
    fn treats_a_map_with_nothing_embedded_as_empty() {
        // Most maps carry a pakfile lump; one that carries nothing has a
        // zero-length lump rather than an empty archive.
        let pak = Pak::parse(&[]).unwrap();

        assert!(pak.is_empty());
        assert_eq!(pak.len(), 0);
        assert_eq!(
            pak.read("anything"),
            Err(Error::MissingEntry("anything".to_owned()))
        );
    }

    #[test]
    fn skips_the_directory_entries_an_archive_lists() {
        let bytes = archive(&[("materials/maps/", b""), ("materials/maps/a.vmt", b"x")]);
        let pak = Pak::parse(&bytes).unwrap();

        assert_eq!(pak.len(), 1, "only the file is an entry");
        assert_eq!(pak.read("materials/maps/a.vmt").unwrap(), b"x");
    }

    #[test]
    fn refuses_an_entry_whose_bytes_do_not_match_its_recorded_crc() {
        let mut bytes = archive(&[("a.vmt", b"original")]);
        // Corrupt the stored data, leaving the recorded CRC alone.
        let at = bytes
            .windows(8)
            .position(|window| window == b"original")
            .expect("the data is in the archive");
        bytes[at] = b'c';

        let pak = Pak::parse(&bytes).unwrap();
        assert!(matches!(pak.read("a.vmt"), Err(Error::CrcMismatch { .. })));
    }

    #[test]
    fn reports_a_compressed_entry_rather_than_returning_it() {
        // VBSP stores everything, so a deflated entry means the archive came
        // from elsewhere. Returning the compressed bytes as though they were
        // the file would produce a material that parses to nothing.
        let mut bytes = archive(&[("a.vmt", b"data")]);
        let at = bytes
            .windows(4)
            .position(|window| window == CENTRAL_HEADER_SIGNATURE.to_le_bytes())
            .expect("the directory is in the archive");
        bytes[at + 10..at + 12].copy_from_slice(&8u16.to_le_bytes());

        assert_eq!(
            Pak::parse(&bytes).err(),
            Some(Error::UnsupportedMethod {
                path: "a.vmt".to_owned(),
                method: 8,
            })
        );
    }

    fn lzma_fixture() -> Vec<u8> {
        // Python 3 zipfile.ZIP_LZMA (liblzma), independent of lzma-rs.
        // Payload: b"Rust owns this packed content. " * 32. ZIP bit 1 has EOS.
        let hex = "504b03043f0002000e000756345ddc8b91053a000000e003000005000000612e766d74090405005d0000800000291d4a676c9b71390d2924e683ef35e403fac99f3b036583d8a4f1760229c2949df858761c56c577f0e7ffffd886f000504b01023f033f0002000e000756345ddc8b91053a000000e0030000050000000000000000000000800100000000612e766d74504b05060000000001000100330000005d0000000000";
        hex.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn decodes_independent_zip_lzma_and_rejects_bombs_and_truncation() {
        let bytes = lzma_fixture();
        let expected = b"Rust owns this packed content. ".repeat(32);
        assert_eq!(Pak::parse(&bytes).unwrap().read("a.vmt").unwrap(), expected);
        let at = 35;
        for offset in [at + 2, at + 4, at + 15, at + 57] {
            let mut damaged = bytes.clone();
            damaged[offset] ^= 255;
            assert!(
                Pak::parse(&damaged).unwrap().read("a.vmt").is_err(),
                "corruption at {offset}"
            );
        }
        for length in [0u32, 1, 991, 993] {
            let mut damaged = bytes.clone();
            damaged[22..26].copy_from_slice(&length.to_le_bytes());
            damaged[93 + 24..93 + 28].copy_from_slice(&length.to_le_bytes());
            assert!(Pak::parse(&damaged).unwrap().read("a.vmt").is_err());
        }
        assert!(Pak::parse_with_limits(
            &bytes,
            Limits {
                max_dictionary_size: 4096,
                ..Default::default()
            }
        )
        .unwrap()
        .read("a.vmt")
        .is_err());
        // Each shorter declared compressed stream must fail, not expose a prefix.
        for length in 0u32..58 {
            let mut damaged = bytes.clone();
            damaged[18..22].copy_from_slice(&length.to_le_bytes());
            damaged[93 + 20..93 + 24].copy_from_slice(&length.to_le_bytes());
            assert!(Pak::parse(&damaged).unwrap().read("a.vmt").is_err());
        }
    }

    #[test]
    fn reads_size_terminated_lzma_without_an_end_marker() {
        for length in [0, 1, 4096] {
            let data = vec![b'x'; length];
            let mut compressed = vec![9, 4, 5, 0];
            lzma_rs::lzma_compress_with_options(
                &mut data.as_slice(),
                &mut compressed,
                &lzma_rs::compress::Options {
                    unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader,
                },
            )
            .unwrap();
            let original = archive(&[("a.vmt", &data)]);
            let old_cd = 35 + data.len();
            let mut zip = original[..35].to_vec();
            zip.extend_from_slice(&compressed);
            let cd = zip.len();
            zip.extend_from_slice(&original[old_cd..]);
            zip[8..10].copy_from_slice(&METHOD_LZMA.to_le_bytes());
            zip[18..22].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
            zip[cd + 10..cd + 12].copy_from_slice(&METHOD_LZMA.to_le_bytes());
            zip[cd + 20..cd + 24].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
            let end = zip.len() - END_OF_DIRECTORY_SIZE;
            zip[end + 16..end + 20].copy_from_slice(&(cd as u32).to_le_bytes());
            assert_eq!(Pak::parse(&zip).unwrap().read("a.vmt").unwrap(), data);
        }
    }

    #[test]
    fn validates_directory_layout_names_flags_and_local_agreement() {
        for name in ["../a", "/a", "a/../b", "a//b", "C:/a", "a\0b", "./a"] {
            assert!(matches!(
                Pak::parse(&archive(&[(name, b"x")])),
                Err(Error::InvalidName { .. })
            ));
        }
        let bytes = archive(&[("a.vmt", b"data")]);
        let end = bytes.len() - 22;
        let cd = read_u32(&bytes, end + 16) as usize;
        for at in [
            6,
            8,
            14,
            18,
            22,
            26,
            cd + 8,
            cd + 20,
            cd + 30,
            cd + 34,
            end + 4,
            end + 6,
            end + 8,
            end + 12,
            end + 20,
        ] {
            let mut bad = bytes.clone();
            bad[at] ^= 1;
            assert!(Pak::parse(&bad).is_err(), "field at {at}");
        }
        // Signatures inside comments cannot be mistaken for the real EOCD.
        let mut commented = bytes.clone();
        commented[end + 20..end + 22].copy_from_slice(&24u16.to_le_bytes());
        commented.extend_from_slice(&END_OF_DIRECTORY_SIGNATURE.to_le_bytes());
        commented.extend_from_slice(&[0; 20]);
        assert_eq!(
            Pak::parse(&commented).unwrap().read("a.vmt").unwrap(),
            b"data"
        );
        // Descriptor mode uses directory sizes and CRC, not local placeholders.
        let mut descriptor = bytes.clone();
        descriptor[6] = 8;
        descriptor[cd + 8] = 8;
        descriptor[14..26].fill(0);
        assert_eq!(
            Pak::parse(&descriptor).unwrap().read("a.vmt").unwrap(),
            b"data"
        );
        assert!(matches!(
            Pak::parse_with_limits(
                &bytes,
                Limits {
                    max_archive_size: 1,
                    ..Default::default()
                }
            ),
            Err(Error::ArchiveTooLarge)
        ));
        // Count directory records too; skipping folders must not bypass limits.
        assert!(matches!(
            Pak::parse_with_limits(
                &archive(&[("a/", b""), ("a/", b"")]),
                Limits {
                    max_entries: 1,
                    ..Default::default()
                }
            ),
            Err(Error::TooManyEntries { .. })
        ));
    }

    #[test]
    fn refuses_archives_it_cannot_trust() {
        assert_eq!(
            Pak::parse(b"not an archive").err(),
            Some(Error::NoEndOfDirectory)
        );

        // An end record pointing past the archive.
        let mut bytes = archive(&[("a.vmt", b"data")]);
        let end = bytes.len() - END_OF_DIRECTORY_SIZE;
        bytes[end + 16..end + 20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            Pak::parse(&bytes),
            Err(Error::DirectoryOutOfRange { .. })
        ));

        // A directory record that is not one.
        let mut bytes = archive(&[("a.vmt", b"data")]);
        let at = bytes
            .windows(4)
            .position(|window| window == CENTRAL_HEADER_SIGNATURE.to_le_bytes())
            .expect("the directory is in the archive");
        bytes[at] = 0;
        assert_eq!(
            Pak::parse(&bytes).err(),
            Some(Error::InvalidDirectoryRecord { index: 0 })
        );

        // An entry the archive claims is larger than the reader allows.
        let bytes = archive(&[("a.vmt", b"data")]);
        assert!(matches!(
            Pak::parse_with_limits(
                &bytes,
                Limits {
                    max_entry_size: 1,
                    ..Default::default()
                }
            ),
            Err(Error::EntryTooLarge { .. })
        ));
        assert!(matches!(
            Pak::parse_with_limits(
                &bytes,
                Limits {
                    max_entries: 0,
                    ..Default::default()
                }
            ),
            Err(Error::TooManyEntries { .. })
        ));
    }

    #[test]
    fn reads_an_entry_whose_local_header_differs_from_the_directory() {
        // The two headers are allowed to carry different extra fields, so
        // the data offset has to come from the local header.
        let mut bytes = archive(&[("a.vmt", b"data")]);
        let extra_at = LOCAL_HEADER_SIZE - 2;
        bytes[extra_at..extra_at + 2].copy_from_slice(&4u16.to_le_bytes());
        let name_end = LOCAL_HEADER_SIZE + "a.vmt".len();
        for (index, byte) in [0xde, 0xad, 0xbe, 0xef].into_iter().enumerate() {
            bytes.insert(name_end + index, byte);
        }
        // Everything after the entry moved, so the directory offset moves too.
        let end = bytes.len() - END_OF_DIRECTORY_SIZE;
        let directory_offset = read_u32(&bytes, end + 16) + 4;
        bytes[end + 16..end + 20].copy_from_slice(&directory_offset.to_le_bytes());

        let pak = Pak::parse(&bytes).unwrap();
        assert_eq!(pak.read("a.vmt").unwrap(), b"data");
    }

    #[test]
    fn lists_what_lives_under_a_directory() {
        let bytes = archive(&[
            ("materials/maps/test/a.vmt", b"1"),
            ("materials/maps/test/b.vmt", b"2"),
            ("materials/other/c.vmt", b"3"),
        ]);
        let pak = Pak::parse(&bytes).unwrap();

        let under: Vec<&str> = pak
            .entries_under("materials/maps/test")
            .map(|entry| entry.path.as_str())
            .collect();
        assert_eq!(
            under,
            vec!["materials/maps/test/a.vmt", "materials/maps/test/b.vmt"]
        );
        assert_eq!(
            pak.entries_under("").count(),
            3,
            "an empty prefix is the root"
        );

        // A directory exists exactly when something is under it, because the
        // archive stores no directories of its own.
        assert!(pak.has_directory("materials/maps"));
        assert!(pak.has_directory("materials/maps/test/"));
        assert!(!pak.has_directory("materials/maps/tes"));
        assert!(!pak.has_directory("nothing"));
    }

    #[test]
    fn describes_every_failure() {
        for error in [
            Error::NoEndOfDirectory,
            Error::DirectoryOutOfRange { offset: 1, size: 2 },
            Error::DirectoryTruncated { index: 0 },
            Error::InvalidDirectoryRecord { index: 0 },
            Error::InvalidName { index: 0 },
            Error::TooManyEntries { limit: 1 },
            Error::EntryTooLarge { size: 2, limit: 1 },
            Error::MissingEntry("a".to_owned()),
            Error::EntryOutOfRange("a".to_owned()),
            Error::InvalidLocalHeader("a".to_owned()),
            Error::UnsupportedMethod {
                path: "a".to_owned(),
                method: 8,
            },
            Error::CrcMismatch {
                path: "a".to_owned(),
                expected: 1,
                actual: 2,
            },
            Error::Overflow,
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}
