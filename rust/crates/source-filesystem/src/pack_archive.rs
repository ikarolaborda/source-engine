//! One archive owner shared by ordinary search mounts and explicit pack APIs.
//! File-backed archives retain the validated descriptor, not compressed bytes.
use source_pak::{Entry, Index, Limits};
use std::fs::{File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug)]
pub enum Error {
    InvalidArgument,
    Io(io::Error),
    Pak(source_pak::Error),
    Bsp(source_bsp::Error),
    NotFound,
    Allocation,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument => write!(f, "invalid pack source"),
            Self::Io(error) => error.fmt(f),
            Self::Pak(error) => error.fmt(f),
            Self::Bsp(error) => error.fmt(f),
            Self::NotFound => write!(f, "pack or entry not found"),
            Self::Allocation => write!(f, "pack allocation failed"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<source_pak::Error> for Error {
    fn from(error: source_pak::Error) -> Self {
        Self::Pak(error)
    }
}
impl From<Error> for super::ReadError {
    fn from(error: Error) -> Self {
        match error {
            Error::Io(error) => Self::Io(error),
            Error::Pak(error) => Self::Pak(error),
            Error::NotFound => Self::NotFound("pack entry".into()),
            error => Self::Io(io::Error::other(error)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Bsp,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Info {
    pub offset: u64,
    pub length: u64,
    pub modified_seconds: i64,
}

#[derive(Debug)]
enum Payload {
    MetadataOnly,
    Memory(Vec<u8>),
    File(Mutex<File>),
}

#[derive(Debug)]
pub struct Archive {
    index: Index,
    payload: Payload,
    path: Option<PathBuf>,
    info: Info,
    kind: Option<Kind>,
}

impl Archive {
    pub fn metadata_only(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Self {
            index: Index::parse(bytes)?,
            payload: Payload::MetadataOnly,
            path: None,
            info: Info::default(),
            kind: None,
        })
    }

    pub fn memory(bytes: &[u8]) -> Result<Self, Error> {
        let mut archive = Self::metadata_only(bytes)?;
        let mut owned = buffer(bytes.len())?;
        owned.copy_from_slice(bytes);
        archive.payload = Payload::Memory(owned);
        archive.info.length = bytes.len() as u64;
        Ok(archive)
    }

    pub fn open_range(path: &Path, offset: u64, length: u64) -> Result<Self, Error> {
        if length > Limits::default().max_archive_size as u64 {
            return Err(source_pak::Error::ArchiveTooLarge.into());
        }
        let (file, metadata) = open_regular_file(path)?;
        Self::from_file_range(path, file, &metadata, offset, length)
    }

    pub fn open(path: &Path, kind: Kind) -> Result<Self, Error> {
        let (mut file, metadata) = open_regular_file(path)?;
        let range = match kind {
            Kind::Zip => {
                if metadata.len() < 22 {
                    return Err(source_pak::Error::NoEndOfDirectory.into());
                }
                0..metadata.len()
            }
            Kind::Bsp => {
                let mut bytes = [0; source_bsp::BSP_HEADER_SIZE];
                if metadata.len() < bytes.len() as u64 {
                    return Err(source_pak::Error::NoEndOfDirectory.into());
                }
                file.read_exact(&mut bytes)?;
                let range = source_bsp::Header::parse(&bytes)
                    .and_then(|header| header.pakfile_range(metadata.len()))
                    .map_err(Error::Bsp)?;
                if range.is_empty() {
                    return Err(Error::NotFound);
                }
                range
            }
        };
        let mut archive =
            Self::from_file_range(path, file, &metadata, range.start, range.end - range.start)?;
        archive.kind = Some(kind);
        Ok(archive)
    }

    pub(crate) fn from_file_range(
        path: &Path,
        mut file: File,
        metadata: &Metadata,
        offset: u64,
        length: u64,
    ) -> Result<Self, Error> {
        if length > Limits::default().max_archive_size as u64 {
            return Err(source_pak::Error::ArchiveTooLarge.into());
        }
        if offset
            .checked_add(length)
            .is_none_or(|end| end > metadata.len())
        {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "pack range outside file").into(),
            );
        }
        let mut bytes = buffer(length as usize)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
        let index = Index::parse(&bytes)?;
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        Ok(Self {
            index,
            payload: Payload::File(Mutex::new(file)),
            path: Some(path),
            info: Info {
                offset,
                length,
                modified_seconds: modified_seconds(metadata),
            },
            kind: None,
        })
    }

    pub fn index(&self) -> &Index {
        &self.index
    }
    pub fn info(&self) -> Info {
        self.info
    }
    /// Only typed discovery establishes file format provenance. A raw range or
    /// memory buffer is not inferred to be a map from its filename or offset.
    pub fn kind(&self) -> Option<Kind> {
        self.kind
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.index.find(path).map(|at| &self.index.entries()[at])
    }
    pub fn entries_under(&self, prefix: &str) -> impl Iterator<Item = &Entry> {
        let mut prefix = prefix.replace('\\', "/").to_ascii_lowercase();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        let start = self
            .index
            .entries()
            .partition_point(|entry| entry.path < prefix);
        self.index.entries()[start..]
            .iter()
            .take_while(move |entry| entry.path.starts_with(&prefix))
    }
    pub fn has_directory(&self, prefix: &str) -> bool {
        self.entries_under(prefix).next().is_some()
    }
    pub fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let at = self.index.find(path).ok_or(Error::NotFound)?;
        self.read_entry(at).map(|(data, _)| data)
    }

    /// No caller-supplied Entry can bypass the validated index. The descriptor
    /// lock protects only seek/read; decompression occurs after it is released.
    pub fn read_entry(&self, at: usize) -> Result<(Vec<u8>, u64), Error> {
        let entry = self.index.entries().get(at).ok_or(Error::NotFound)?;
        if matches!(self.payload, Payload::MetadataOnly) {
            return Err(Error::InvalidArgument);
        }
        let end = entry
            .data_offset()
            .checked_add(entry.compressed_length as u64)
            .ok_or(source_pak::Error::Overflow)?;
        if end > self.info.length {
            return Err(source_pak::Error::EntryOutOfRange(entry.path.clone()).into());
        }
        let absolute = self
            .info
            .offset
            .checked_add(entry.data_offset())
            .ok_or(source_pak::Error::Overflow)?;
        let data = match &self.payload {
            Payload::MetadataOnly => unreachable!(),
            Payload::Memory(bytes) => entry.decode_payload(
                &bytes[entry.data_offset() as usize..end as usize],
                Limits::default().max_dictionary_size,
            )?,
            Payload::File(file) => {
                let mut bytes = buffer(entry.compressed_length as usize)?;
                {
                    let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
                    file.seek(SeekFrom::Start(absolute))?;
                    file.read_exact(&mut bytes)?;
                }
                entry.decode_payload(&bytes, Limits::default().max_dictionary_size)?
            }
        };
        Ok((data, absolute))
    }
}

fn buffer(length: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| Error::Allocation)?;
    bytes.resize(length, 0);
    Ok(bytes)
}

pub(crate) fn open_regular_file(path: &Path) -> Result<(File, Metadata), Error> {
    if path.as_os_str().is_empty() {
        return Err(Error::InvalidArgument);
    }
    // Reject known special files before opening (a FIFO could otherwise block).
    if !std::fs::metadata(path)?.is_file() {
        return Err(Error::InvalidArgument);
    }
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::InvalidArgument);
    }
    Ok((file, metadata))
}

fn modified_seconds(metadata: &Metadata) -> i64 {
    let Ok(time) = metadata.modified() else {
        return 0;
    };
    let seconds = match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i128,
        Err(error) => {
            let duration = error.duration();
            -(duration.as_secs() as i128) - i128::from(duration.subsec_nanos() != 0)
        }
    };
    seconds.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}
