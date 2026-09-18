//! Reader for the ZIP archive a map carries in its pakfile lump.
//!
//! A compiled map embeds the content that only it uses. Most of that is the
//! cubemap materials VBSP generates: for every surface whose material
//! reflects its surroundings, it writes a small `patch` material named after
//! the surface's position, pointing at a cubemap baked for that spot. Those
//! materials exist nowhere else, so a map's world cannot be textured without
//! reading this archive first.
//!
//! The archive is a plain ZIP. VBSP writes every entry stored rather than
//! compressed, which is why only that method is read here; a compressed entry
//! is reported rather than returned as the bytes it isn't.

use std::collections::BTreeMap;
use std::fmt;

/// `PK\x03\x04`, before each entry's own header.
const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
/// `PK\x01\x02`, before each central directory record.
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
/// `PK\x05\x06`, at the end of the archive.
const END_OF_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;

const LOCAL_HEADER_SIZE: usize = 30;
const CENTRAL_HEADER_SIZE: usize = 46;
const END_OF_DIRECTORY_SIZE: usize = 22;

/// The only compression method a map's pakfile uses.
const METHOD_STORED: u16 = 0;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    pub max_entry_size: usize,
    pub max_name_length: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 65_536,
            max_entry_size: 256 * 1024 * 1024,
            max_name_length: 1024,
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
}

impl Entry {
    pub fn is_stored(&self) -> bool {
        self.method == METHOD_STORED
    }
}

/// A map's embedded archive, read from the pakfile lump.
#[derive(Debug, Clone)]
pub struct Pak {
    entries: BTreeMap<String, Entry>,
    bytes: Vec<u8>,
}

impl Pak {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Self> {
        // An empty lump is how a map with no embedded content states it,
        // which is not an error.
        if bytes.is_empty() {
            return Ok(Self {
                entries: BTreeMap::new(),
                bytes: Vec::new(),
            });
        }

        let directory = find_end_of_directory(bytes)?;
        let mut entries = BTreeMap::new();
        let mut at = directory.offset;

        for index in 0..directory.count {
            if entries.len() >= limits.max_entries {
                return Err(Error::TooManyEntries {
                    limit: limits.max_entries,
                });
            }
            let header = bytes
                .get(at..at.checked_add(CENTRAL_HEADER_SIZE).ok_or(Error::Overflow)?)
                .ok_or(Error::DirectoryTruncated { index })?;
            if read_u32(header, 0) != CENTRAL_HEADER_SIGNATURE {
                return Err(Error::InvalidDirectoryRecord { index });
            }

            let method = read_u16(header, 10);
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

            at = name_end
                .checked_add(extra_length)
                .and_then(|next| next.checked_add(comment_length))
                .ok_or(Error::Overflow)?;

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
                },
            );
        }

        Ok(Self {
            entries,
            bytes: bytes.to_vec(),
        })
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
    pub fn read(&self, path: &str) -> Result<&[u8]> {
        let entry = self
            .entry(path)
            .ok_or_else(|| Error::MissingEntry(normalize(path)))?;
        if !entry.is_stored() {
            return Err(Error::UnsupportedMethod {
                path: entry.path.clone(),
                method: entry.method,
            });
        }

        // The name and extra field of the local header are not required to
        // match the central directory's, so the data offset is found by
        // reading the local header rather than assuming either length.
        let offset = entry.header_offset as usize;
        let header = self
            .bytes
            .get(
                offset
                    ..offset
                        .checked_add(LOCAL_HEADER_SIZE)
                        .ok_or(Error::Overflow)?,
            )
            .ok_or_else(|| Error::EntryOutOfRange(entry.path.clone()))?;
        if read_u32(header, 0) != LOCAL_HEADER_SIGNATURE {
            return Err(Error::InvalidLocalHeader(entry.path.clone()));
        }
        let name_length = read_u16(header, 26) as usize;
        let extra_length = read_u16(header, 28) as usize;

        let start = offset
            .checked_add(LOCAL_HEADER_SIZE)
            .and_then(|at| at.checked_add(name_length))
            .and_then(|at| at.checked_add(extra_length))
            .ok_or(Error::Overflow)?;
        let end = start
            .checked_add(entry.length as usize)
            .ok_or(Error::Overflow)?;
        let data = self
            .bytes
            .get(start..end)
            .ok_or_else(|| Error::EntryOutOfRange(entry.path.clone()))?;

        let actual = source_vpk::crc32(data);
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
}

fn find_end_of_directory(bytes: &[u8]) -> Result<Directory> {
    // The record sits at the end but may be followed by a comment, so it is
    // searched for backwards.
    let start = bytes
        .len()
        .checked_sub(END_OF_DIRECTORY_SIZE)
        .ok_or(Error::NoEndOfDirectory)?;
    let found = (0..=start)
        .rev()
        .find(|at| read_u32(bytes, *at) == END_OF_DIRECTORY_SIGNATURE)
        .ok_or(Error::NoEndOfDirectory)?;

    let record = &bytes[found..];
    let count = read_u16(record, 10) as usize;
    let size = read_u32(record, 12) as usize;
    let offset = read_u32(record, 16) as usize;

    let end = offset.checked_add(size).ok_or(Error::Overflow)?;
    if end > bytes.len() {
        return Err(Error::DirectoryOutOfRange { offset, size });
    }
    Ok(Directory { offset, count })
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
    /// A compressed entry. VBSP stores everything it embeds, so this means
    /// the archive was written by something else.
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

        let pak = Pak::parse(&bytes).unwrap();
        assert_eq!(
            pak.read("a.vmt"),
            Err(Error::UnsupportedMethod {
                path: "a.vmt".to_owned(),
                method: 8,
            })
        );
        assert!(!pak.entry("a.vmt").unwrap().is_stored());
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
