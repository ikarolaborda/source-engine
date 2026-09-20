//! Path contracts for the Rust-owned content boundary.

pub mod gameinfo;
pub mod mount_table;
pub mod pack_archive;
pub mod pack_mounts;
pub mod search_plan;
pub mod selection;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MAX_VIRTUAL_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyPath,
    PathTooLong(usize),
    AbsolutePath,
    InvalidComponent(String),
    NonUtf8Output,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "content path is empty"),
            Self::PathTooLong(length) => write!(f, "content path is too long ({length} bytes)"),
            Self::AbsolutePath => write!(f, "content path must be relative"),
            Self::InvalidComponent(component) => {
                write!(f, "invalid content path component {component:?}")
            }
            Self::NonUtf8Output => write!(f, "resolved content path is not UTF-8"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn normalize_virtual_path(path: &str) -> Result<String> {
    if path.is_empty() {
        return Err(Error::EmptyPath);
    }
    if path.len() > MAX_VIRTUAL_PATH_BYTES {
        return Err(Error::PathTooLong(path.len()));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(Error::AbsolutePath);
    }

    let mut normalized = String::with_capacity(path.len());
    for component in path.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.contains(['\\', '\0', ':'])
        {
            return Err(Error::InvalidComponent(component.to_owned()));
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(component);
    }
    Ok(normalized)
}

pub fn resolve_content_path(root: &Path, virtual_path: &str) -> Result<PathBuf> {
    let normalized = normalize_virtual_path(virtual_path)?;
    Ok(root.join(normalized))
}

pub fn resolve_content_path_utf8(root: &Path, virtual_path: &str) -> Result<String> {
    resolve_content_path(root, virtual_path)?
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or(Error::NonUtf8Output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Head,
    Tail,
}

#[derive(Debug)]
enum MountKind {
    Directory {
        /// The root as the engine configured it. Resolved paths stay rooted
        /// here because the engine attributes an absolute path back to a search
        /// path by prefix, and a search path reached through a symlink would
        /// never match a symlink-resolved result.
        root: PathBuf,
        /// The same root with symlinks resolved, used only to detect a
        /// traversal that escapes the mount.
        canonical_root: PathBuf,
        allow_symlink_escape: bool,
    },
    Vpk {
        directory_path: PathBuf,
        archive: Arc<source_vpk::OwnedArchive>,
    },
    /// A standalone ZIP or the archive a map carries inside itself.
    Pak {
        archive_path: PathBuf,
        pak: Arc<pack_archive::Archive>,
        is_map: bool,
    },
}

/// Makes a mount root absolute without resolving symlinks, so the engine's own
/// view of its search paths survives into every path this crate hands back.
fn absolute_path(path: &Path) -> std::result::Result<PathBuf, ReadError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir().map_err(ReadError::Io)?.join(path))
}

/// Resolves `relative` under a directory mount, returning the path rooted at
/// the configured root alongside the symlink-resolved path the escape check and
/// metadata queries need. `None` means the candidate does not exist.
fn resolve_in_directory(
    root: &Path,
    canonical_root: &Path,
    allow_symlink_escape: bool,
    relative: &Path,
) -> std::result::Result<Option<(PathBuf, PathBuf)>, ReadError> {
    let logical = if relative.as_os_str().is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let canonical = match std::fs::canonicalize(&logical) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ReadError::Io(error)),
    };
    if !allow_symlink_escape && !canonical.starts_with(canonical_root) {
        return Err(ReadError::EscapedRoot(canonical));
    }
    Ok(Some((logical, canonical)))
}

#[derive(Debug)]
struct Mount {
    path_id: String,
    by_request_only: bool,
    kind: MountKind,
}

impl Mount {
    fn matches(&self, requested: Option<&str>) -> bool {
        selection::path_id_matches(
            &self.path_id,
            requested,
            self.by_request_only,
            matches!(self.kind, MountKind::Pak { is_map: true, .. }),
        )
    }
}

#[derive(Debug)]
pub enum ReadError {
    Path(Error),
    Io(io::Error),
    Vpk(source_vpk::Error),
    Pak(source_pak::Error),
    NotFound(String),
    EscapedRoot(PathBuf),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::Vpk(error) => error.fmt(f),
            Self::Pak(error) => error.fmt(f),
            Self::NotFound(path) => write!(f, "content file not found: {path}"),
            Self::EscapedRoot(path) => {
                write!(
                    f,
                    "content path escaped its mounted root: {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ReadError {}

impl From<Error> for ReadError {
    fn from(value: Error) -> Self {
        Self::Path(value)
    }
}

impl From<io::Error> for ReadError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<source_vpk::Error> for ReadError {
    fn from(value: source_vpk::Error) -> Self {
        Self::Vpk(value)
    }
}

impl From<source_pak::Error> for ReadError {
    fn from(value: source_pak::Error) -> Self {
        Self::Pak(value)
    }
}

#[derive(Debug)]
pub enum ReadSource {
    Disk(PathBuf),
    Memory(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPathKind {
    Disk,
    Vpk,
    /// From the archive embedded in the loaded map.
    Pak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedReadPath {
    pub path: PathBuf,
    pub kind: ReadPathKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindEntry {
    pub name: String,
    pub is_directory: bool,
}

#[derive(Debug, Default)]
pub struct SearchPaths {
    mounts: Vec<Mount>,
    vpk_cache: HashMap<PathBuf, Arc<source_vpk::OwnedArchive>>,
    pak_cache: HashMap<PakCacheKey, Arc<pack_archive::Archive>>,
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct PakCacheKey {
    path: PathBuf,
    offset: u64,
    length: u64,
    file_length: u64,
    modified: Option<std::time::SystemTime>,
}

impl SearchPaths {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.mounts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }

    pub fn clear(&mut self) {
        // Keep the current generation for a cheap search-path rebuild, but
        // release archives no longer mounted on the following clear. Do not
        // accumulate every map visited during a campaign.
        self.pak_cache.retain(|_, pak| Arc::strong_count(pak) > 1);
        self.mounts.clear();
    }

    pub fn mount_directory(
        &mut self,
        root: impl AsRef<Path>,
        path_id: impl Into<String>,
        position: Position,
    ) -> std::result::Result<(), ReadError> {
        self.mount_directory_with_flags(root, path_id, position, false, false)
    }

    pub fn mount_directory_with_flags(
        &mut self,
        root: impl AsRef<Path>,
        path_id: impl Into<String>,
        position: Position,
        by_request_only: bool,
        allow_symlink_escape: bool,
    ) -> std::result::Result<(), ReadError> {
        let root = absolute_path(root.as_ref())?;
        let canonical_root = std::fs::canonicalize(&root)?;
        if !canonical_root.is_dir() {
            return Err(ReadError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "content search root is not a directory",
            )));
        }
        self.insert(
            Mount {
                path_id: path_id.into(),
                by_request_only,
                kind: MountKind::Directory {
                    root,
                    canonical_root,
                    allow_symlink_escape,
                },
            },
            position,
        );
        Ok(())
    }

    pub fn mount_vpk(
        &mut self,
        directory_path: impl AsRef<Path>,
        path_id: impl Into<String>,
        position: Position,
    ) -> std::result::Result<(), ReadError> {
        self.mount_vpk_with_flags(directory_path, path_id, position, false)
    }

    pub fn mount_vpk_with_flags(
        &mut self,
        directory_path: impl AsRef<Path>,
        path_id: impl Into<String>,
        position: Position,
        by_request_only: bool,
    ) -> std::result::Result<(), ReadError> {
        // Keyed on the canonical path so two mounts of the same archive share
        // one parse, but composed paths use the configured location so they
        // stay under the engine's search path.
        let directory_path = absolute_path(directory_path.as_ref())?;
        let cache_key = std::fs::canonicalize(&directory_path)?;
        let archive = if let Some(archive) = self.vpk_cache.get(&cache_key) {
            Arc::clone(archive)
        } else {
            let directory_bytes = std::fs::read(&directory_path)?;
            let archive = Arc::new(source_vpk::OwnedArchive::parse(&directory_bytes)?);
            self.vpk_cache.insert(cache_key, Arc::clone(&archive));
            archive
        };
        self.insert(
            Mount {
                path_id: path_id.into(),
                by_request_only,
                kind: MountKind::Vpk {
                    directory_path,
                    archive,
                },
            },
            position,
        );
        Ok(())
    }

    /// Mounts the archive a map embeds, from the bytes of its pakfile lump.
    ///
    /// The engine adds this ahead of the game's own archives while a map is
    /// loaded, because a map's content is allowed to stand in for content of
    /// the same name shipped with the game, and because its cubemap
    /// materials exist nowhere else. Mount it with [`Position::Head`] to get
    /// that precedence.
    pub fn mount_pak(
        &mut self,
        pakfile: &[u8],
        name: impl Into<String>,
        path_id: impl Into<String>,
        position: Position,
    ) -> std::result::Result<(), ReadError> {
        let pak = Arc::new(pack_archive::Archive::memory(pakfile)?);
        self.insert(
            Mount {
                path_id: path_id.into(),
                by_request_only: false,
                kind: MountKind::Pak {
                    archive_path: PathBuf::from(format!("{}.bsp", name.into())),
                    pak,
                    is_map: true,
                },
            },
            position,
        );
        Ok(())
    }

    /// Opens and validates only the ZIP range in a physical archive/BSP file.
    /// The caller supplies archive location, not parsed entries or payloads.
    #[allow(clippy::too_many_arguments)]
    pub fn mount_pak_file_with_flags(
        &mut self,
        archive_path: impl AsRef<Path>,
        offset: u64,
        length: u64,
        path_id: impl Into<String>,
        position: Position,
        by_request_only: bool,
    ) -> std::result::Result<(), ReadError> {
        let archive_path = absolute_path(archive_path.as_ref())?;
        let (file, metadata) = pack_archive::open_regular_file(&archive_path)?;
        if !metadata.is_file()
            || offset
                .checked_add(length)
                .is_none_or(|end| end > metadata.len())
        {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "pak range outside file").into(),
            );
        }
        if length > source_pak::Limits::default().max_archive_size as u64 {
            return Err(source_pak::Error::ArchiveTooLarge.into());
        }
        let key = PakCacheKey {
            path: std::fs::canonicalize(&archive_path)?,
            offset,
            length,
            file_length: metadata.len(),
            modified: metadata.modified().ok(),
        };
        let pak = if let Some(pak) = self.pak_cache.get(&key) {
            Arc::clone(pak)
        } else {
            let pak = Arc::new(pack_archive::Archive::from_file_range(
                &archive_path,
                file,
                &metadata,
                offset,
                length,
            )?);
            self.pak_cache.insert(key, Arc::clone(&pak));
            pak
        };
        self.insert(
            Mount {
                path_id: path_id.into(),
                by_request_only,
                kind: MountKind::Pak {
                    archive_path,
                    pak,
                    is_map: false,
                },
            },
            position,
        );
        Ok(())
    }

    /// Share an already-open archive, including its validated index and retained
    /// descriptor. No path lookup, stat, reopen, reparse or byte-copy occurs.
    /// Clearing mounts drops this reference, not independent indexes or files.
    pub fn mount_pack_archive(
        &mut self,
        pak: Arc<pack_archive::Archive>,
        path_id: impl Into<String>,
        position: Position,
        by_request_only: bool,
    ) -> std::result::Result<(), ReadError> {
        let archive_path = pak
            .path()
            .ok_or(pack_archive::Error::InvalidArgument)?
            .to_path_buf();
        self.insert(
            Mount {
                path_id: path_id.into(),
                by_request_only,
                kind: MountKind::Pak {
                    archive_path,
                    is_map: pak.kind() == Some(pack_archive::Kind::Bsp),
                    pak,
                },
            },
            position,
        );
        Ok(())
    }

    pub fn read(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<Vec<u8>, ReadError> {
        match self.open_read_source(virtual_path, path_id)? {
            ReadSource::Disk(path) => std::fs::read(path).map_err(ReadError::Io),
            ReadSource::Memory(data) => Ok(data),
        }
    }

    pub fn open_read_source(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<ReadSource, ReadError> {
        let normalized = normalize_read_path(virtual_path, path_id)?;
        for mount in &self.mounts {
            if !mount.matches(path_id) {
                continue;
            }
            match &mount.kind {
                MountKind::Directory {
                    root,
                    canonical_root,
                    allow_symlink_escape,
                } => {
                    let Some((logical, canonical)) = resolve_in_directory(
                        root,
                        canonical_root,
                        *allow_symlink_escape,
                        Path::new(&normalized),
                    )?
                    else {
                        continue;
                    };
                    if canonical.is_file() {
                        return Ok(ReadSource::Disk(logical));
                    }
                }
                MountKind::Vpk {
                    directory_path,
                    archive,
                } => {
                    if archive.entry(&normalized).is_none() {
                        continue;
                    }
                    let data = archive
                        .read_file(&normalized, |index| {
                            std::fs::read(vpk_chunk_path(directory_path, index))
                                .map_err(|_| source_vpk::Error::MissingArchive(index))
                        })
                        .map_err(ReadError::Vpk)?;
                    return Ok(ReadSource::Memory(data));
                }
                MountKind::Pak { pak, .. } => {
                    if pak.entry(&normalized).is_none() {
                        continue;
                    }
                    let data = pak.read(&normalized).map_err(ReadError::from)?;
                    return Ok(ReadSource::Memory(data));
                }
            }
        }
        Err(ReadError::NotFound(normalized))
    }

    pub fn file_size(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<u64, ReadError> {
        let normalized = normalize_read_path(virtual_path, path_id)?;
        for mount in &self.mounts {
            if !mount.matches(path_id) {
                continue;
            }
            match &mount.kind {
                MountKind::Directory {
                    root,
                    canonical_root,
                    allow_symlink_escape,
                } => {
                    let Some((_, canonical)) = resolve_in_directory(
                        root,
                        canonical_root,
                        *allow_symlink_escape,
                        Path::new(&normalized),
                    )?
                    else {
                        continue;
                    };
                    let metadata = std::fs::metadata(canonical)?;
                    if metadata.is_file() {
                        return Ok(metadata.len());
                    }
                }
                MountKind::Vpk { archive, .. } => {
                    if let Some(entry) = archive.entry(&normalized) {
                        return Ok(entry.total_length());
                    }
                }
                MountKind::Pak { pak, .. } => {
                    if let Some(entry) = pak.entry(&normalized) {
                        return Ok(u64::from(entry.length));
                    }
                }
            }
        }
        Err(ReadError::NotFound(normalized))
    }

    /// Resolves an existing relative path to the native path spelling used by
    /// Source. VPK entries use the encoded `<base>.vpk/<virtual-path>` form,
    /// which the legacy filesystem can accept as an absolute packed path.
    pub fn resolve_read_path(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<ResolvedReadPath, ReadError> {
        let normalized = normalize_read_path(virtual_path, path_id)?;
        for mount in &self.mounts {
            if !mount.matches(path_id) {
                continue;
            }
            match &mount.kind {
                MountKind::Directory {
                    root,
                    canonical_root,
                    allow_symlink_escape,
                } => {
                    let Some((logical, _)) = resolve_in_directory(
                        root,
                        canonical_root,
                        *allow_symlink_escape,
                        Path::new(&normalized),
                    )?
                    else {
                        continue;
                    };
                    return Ok(ResolvedReadPath {
                        path: logical,
                        kind: ReadPathKind::Disk,
                    });
                }
                MountKind::Vpk {
                    directory_path,
                    archive,
                } => {
                    if archive.entry(&normalized).is_some() {
                        return Ok(ResolvedReadPath {
                            path: vpk_logical_path(directory_path).join(&normalized),
                            kind: ReadPathKind::Vpk,
                        });
                    }
                }
                MountKind::Pak {
                    archive_path, pak, ..
                } => {
                    if pak.entry(&normalized).is_some() {
                        // Named the same way a packed VPK path is, so a
                        // caller reporting where a file came from can say
                        // which map carried it.
                        return Ok(ResolvedReadPath {
                            path: archive_path.join(&normalized),
                            kind: ReadPathKind::Pak,
                        });
                    }
                }
            }
        }
        Err(ReadError::NotFound(normalized))
    }

    pub fn is_directory(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<bool, ReadError> {
        let normalized = normalize_read_path(virtual_path, path_id)?;
        let vpk_prefix = format!("{}/", normalized.to_ascii_lowercase());
        for mount in &self.mounts {
            if !mount.matches(path_id) {
                continue;
            }
            match &mount.kind {
                MountKind::Directory {
                    root,
                    canonical_root,
                    allow_symlink_escape,
                } => {
                    let Some((_, canonical)) = resolve_in_directory(
                        root,
                        canonical_root,
                        *allow_symlink_escape,
                        Path::new(&normalized),
                    )?
                    else {
                        continue;
                    };
                    if canonical.is_dir() {
                        return Ok(true);
                    }
                }
                MountKind::Vpk { archive, .. } => {
                    let entries = archive.entries();
                    let first =
                        entries.partition_point(|entry| entry.path.as_str() < vpk_prefix.as_str());
                    if entries
                        .get(first)
                        .is_some_and(|entry| entry.path.starts_with(&vpk_prefix))
                    {
                        return Ok(true);
                    }
                }
                MountKind::Pak { pak, .. } => {
                    if pak.has_directory(&normalized) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    /// Enumerates the immediate children matching a Source-style wildcard.
    /// Results retain mount precedence, suppress duplicate names across mounts,
    /// and include virtual directories implied by VPK entry paths.
    pub fn find(
        &self,
        wildcard: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<Vec<FindEntry>, ReadError> {
        let (directory, pattern) = if path_id.is_some_and(|id| id.eq_ignore_ascii_case("BSP")) {
            pack_find_pattern(wildcard)?
        } else {
            normalize_find_pattern(wildcard)?
        };
        let prefix = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        for mount in &self.mounts {
            if !mount.matches(path_id) {
                continue;
            }

            let mut mount_entries = match &mount.kind {
                MountKind::Directory {
                    root,
                    canonical_root,
                    allow_symlink_escape,
                } => {
                    let Some((logical_directory, canonical)) = resolve_in_directory(
                        root,
                        canonical_root,
                        *allow_symlink_escape,
                        Path::new(&directory),
                    )?
                    else {
                        continue;
                    };
                    if !canonical.is_dir() {
                        continue;
                    }

                    let mut entries = Vec::new();
                    for entry in std::fs::read_dir(&logical_directory)? {
                        let entry = entry?;
                        let name = match entry.file_name().into_string() {
                            Ok(name) => name,
                            Err(_) => continue,
                        };
                        if !wildcard_matches(&pattern, &name) {
                            continue;
                        }
                        let resolved = match std::fs::canonicalize(entry.path()) {
                            Ok(path) => path,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                            Err(error) => return Err(ReadError::Io(error)),
                        };
                        if !allow_symlink_escape && !resolved.starts_with(canonical_root) {
                            return Err(ReadError::EscapedRoot(resolved));
                        }
                        let metadata = std::fs::metadata(resolved)?;
                        if metadata.is_file() || metadata.is_dir() {
                            entries.push(FindEntry {
                                name,
                                is_directory: metadata.is_dir(),
                            });
                        }
                    }
                    entries.sort_by(find_entry_order);
                    entries
                }
                MountKind::Vpk { archive, .. } => {
                    let vpk_prefix = prefix.to_ascii_lowercase();
                    let mut files = BTreeMap::new();
                    let mut directories = BTreeMap::new();
                    let archive_entries = archive.entries();
                    let first = if vpk_prefix.is_empty() {
                        0
                    } else {
                        archive_entries
                            .partition_point(|entry| entry.path.as_str() < vpk_prefix.as_str())
                    };
                    for entry in &archive_entries[first..] {
                        if !vpk_prefix.is_empty() && !entry.path.starts_with(&vpk_prefix) {
                            break;
                        }
                        let Some(remainder) = entry.path.strip_prefix(&vpk_prefix) else {
                            continue;
                        };
                        if remainder.is_empty() {
                            continue;
                        }
                        let (name, is_directory) = match remainder.split_once('/') {
                            Some((name, _)) => (name, true),
                            None => (remainder, false),
                        };
                        if name.is_empty() || !wildcard_matches(&pattern, name) {
                            continue;
                        }
                        let key = name.to_ascii_lowercase();
                        let value = FindEntry {
                            name: name.to_owned(),
                            is_directory,
                        };
                        if is_directory {
                            directories.entry(key).or_insert(value);
                        } else {
                            files.entry(key).or_insert(value);
                        }
                    }
                    files
                        .into_values()
                        .chain(directories.into_values())
                        .collect()
                }
                MountKind::Pak { pak, .. } => {
                    let prefix = prefix.to_ascii_lowercase();
                    archive_matches(
                        pak.entries_under(prefix.trim_end_matches('/'))
                            .map(|entry| entry.path.as_str()),
                        &prefix,
                        &pattern,
                        false,
                    )
                }
            };

            for entry in mount_entries.drain(..) {
                if seen.insert(entry.name.to_ascii_lowercase()) {
                    results.push(entry);
                }
            }
        }
        Ok(results)
    }

    fn insert(&mut self, mount: Mount, position: Position) {
        match position {
            Position::Head => self.mounts.insert(0, mount),
            Position::Tail => self.mounts.push(mount),
        }
    }
}

/// Normalize archive-local paths without allowing traversal outside the root.
fn normalize_pack_path(path: &str, reject_wildcards: bool) -> Result<String> {
    if path.len() > MAX_VIRTUAL_PATH_BYTES {
        return Err(Error::PathTooLong(path.len()));
    }
    if path.starts_with(['/', '\\']) {
        return Err(Error::AbsolutePath);
    }
    if path.contains(['\0', ':']) {
        return Err(Error::InvalidComponent(path.to_owned()));
    }
    let path = path.replace('\\', "/");
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts
                    .pop()
                    .ok_or_else(|| Error::InvalidComponent(path.to_owned()))?;
            }
            _ if reject_wildcards && part.contains(['*', '?']) => {
                return Err(Error::InvalidComponent(part.to_owned()))
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/").to_ascii_lowercase())
}

fn pack_find_pattern(wildcard: &str) -> Result<(String, String)> {
    if wildcard.is_empty() {
        return Err(Error::EmptyPath);
    }
    if wildcard.len() > MAX_VIRTUAL_PATH_BYTES {
        return Err(Error::PathTooLong(wildcard.len()));
    }
    if wildcard.starts_with(['/', '\\']) {
        return Err(Error::AbsolutePath);
    }
    if wildcard.contains(['\0', ':']) {
        return Err(Error::InvalidComponent(wildcard.to_owned()));
    }
    let wildcard = wildcard.replace('\\', "/");
    let (directory, pattern) = wildcard.rsplit_once('/').unwrap_or(("", &wildcard));
    if pattern.is_empty() || matches!(pattern, "." | "..") {
        return Err(Error::InvalidComponent(pattern.to_owned()));
    }
    Ok((normalize_pack_path(directory, true)?, pattern.to_owned()))
}

fn normalize_read_path(path: &str, requested: Option<&str>) -> Result<String> {
    if requested.is_some_and(|id| id.eq_ignore_ascii_case("BSP")) {
        normalize_pack_path(path, false)
    } else {
        normalize_virtual_path(path)
    }
}

/// Match one metadata-only pack, returning canonical relative paths (not just
/// basenames), files then directories, sorted/deduplicated within each kind.
/// Uses the same glob policy as ordinary read mounts, but accepts archive-local
/// dot/slash normalization and rejects traversal outside the archive root.
pub fn find_pack_index(index: &source_pak::Index, wildcard: &str) -> Result<Vec<FindEntry>> {
    let (directory, pattern) = pack_find_pattern(wildcard)?;
    let prefix = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    let entries = index.entries();
    let first = entries.partition_point(|entry| entry.path < prefix);
    Ok(archive_matches(
        entries[first..]
            .iter()
            .take_while(|entry| entry.path.starts_with(&prefix))
            .map(|entry| entry.path.as_str()),
        &prefix,
        &pattern,
        true,
    ))
}

fn archive_matches<'a>(
    paths: impl Iterator<Item = &'a str>,
    prefix: &str,
    pattern: &str,
    qualified: bool,
) -> Vec<FindEntry> {
    let mut files = BTreeMap::new();
    let mut directories = BTreeMap::new();
    for path in paths {
        // Optional native preload cache is never an asset or directory child.
        if path == "__preload_section.pre" {
            continue;
        }
        let Some(remainder) = path.strip_prefix(prefix) else {
            continue;
        };
        let (name, is_directory) = remainder
            .split_once('/')
            .map_or((remainder, false), |(name, _)| (name, true));
        if name.is_empty() || !wildcard_matches(pattern, name) {
            continue;
        }
        let value = FindEntry {
            name: if qualified {
                format!("{prefix}{name}")
            } else {
                name.to_owned()
            },
            is_directory,
        };
        if is_directory {
            directories.entry(name.to_owned()).or_insert(value);
        } else {
            files.entry(name.to_owned()).or_insert(value);
        }
    }
    // As in mount-wide find, a file wins a colliding virtual directory name.
    directories.retain(|name, _| !files.contains_key(name));
    files
        .into_values()
        .chain(directories.into_values())
        .collect()
}

fn normalize_find_pattern(pattern: &str) -> Result<(String, String)> {
    if pattern.is_empty() {
        return Err(Error::EmptyPath);
    }
    if pattern.len() > MAX_VIRTUAL_PATH_BYTES {
        return Err(Error::PathTooLong(pattern.len()));
    }
    let pattern = pattern.replace('\\', "/");
    if pattern.starts_with('/') {
        return Err(Error::AbsolutePath);
    }
    let (directory, basename) = pattern
        .rsplit_once('/')
        .map_or(("", pattern.as_str()), |(directory, basename)| {
            (directory, basename)
        });
    if basename.is_empty()
        || basename == "."
        || basename == ".."
        || basename.contains(['/', '\\', '\0', ':'])
    {
        return Err(Error::InvalidComponent(basename.to_owned()));
    }
    let directory = if directory.is_empty() {
        String::new()
    } else {
        let normalized = normalize_virtual_path(directory)?;
        if normalized.contains(['*', '?']) {
            return Err(Error::InvalidComponent(directory.to_owned()));
        }
        normalized
    };
    Ok((directory, basename.to_owned()))
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    // The native POSIX compatibility layer intentionally preserves Win32's
    // historical `*.*` meaning of every directory entry, including names
    // without a dot.
    if pattern.eq_ignore_ascii_case("*.*") {
        return true;
    }
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut star_index, mut star_value_index) = (None, 0);

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index].eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_value_index = value_index;
        } else if let Some(star) = star_index {
            star_value_index += 1;
            value_index = star_value_index;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn find_entry_order(left: &FindEntry, right: &FindEntry) -> std::cmp::Ordering {
    left.name
        .to_ascii_lowercase()
        .cmp(&right.name.to_ascii_lowercase())
        .then_with(|| left.name.cmp(&right.name))
}

#[derive(Debug)]
struct WriteMount {
    path_id: String,
    canonical_root: PathBuf,
    by_request_only: bool,
}

/// Ordered loose-directory mounts used to reproduce Source's write-path
/// selection independently from its read search order.
#[derive(Debug, Default)]
pub struct WritePaths {
    mounts: Vec<WriteMount>,
}

impl WritePaths {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.mounts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mounts.is_empty()
    }

    pub fn clear(&mut self) {
        self.mounts.clear();
    }

    pub fn mount_directory(
        &mut self,
        root: impl AsRef<Path>,
        path_id: impl Into<String>,
        position: Position,
    ) -> std::result::Result<(), ReadError> {
        self.mount_directory_with_request_only(root, path_id, position, false)
    }

    pub fn mount_directory_with_request_only(
        &mut self,
        root: impl AsRef<Path>,
        path_id: impl Into<String>,
        position: Position,
        by_request_only: bool,
    ) -> std::result::Result<(), ReadError> {
        let canonical_root = std::fs::canonicalize(root)?;
        if !canonical_root.is_dir() {
            return Err(ReadError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "write search root is not a directory",
            )));
        }
        let mount = WriteMount {
            path_id: path_id.into(),
            canonical_root,
            by_request_only,
        };
        match position {
            Position::Head => self.mounts.insert(0, mount),
            Position::Tail => self.mounts.push(mount),
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<PathBuf, ReadError> {
        let normalized = normalize_write_path(virtual_path)?;
        let requested = path_id.filter(|path_id| !path_id.is_empty());
        let preferred = requested.and_then(|path_id| {
            if path_id.eq_ignore_ascii_case("game") {
                self.find("game_write")
            } else if path_id.eq_ignore_ascii_case("mod") {
                self.find("mod_write")
            } else {
                None
            }
        });
        let mount = preferred
            .or_else(|| requested.and_then(|path_id| self.find(path_id)))
            .or_else(|| self.find("default_write_path"))
            .or_else(|| self.mounts.first())
            .ok_or_else(|| ReadError::NotFound(normalized.clone()))?;
        let candidate = mount.canonical_root.join(&normalized);
        validate_write_ancestor(&mount.canonical_root, &candidate)?;
        Ok(candidate)
    }

    pub fn create_directory_all(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<PathBuf, ReadError> {
        let trimmed = virtual_path.trim_end_matches(['/', '\\']);
        let resolved = self.resolve(trimmed, path_id)?;
        std::fs::create_dir_all(&resolved)?;
        Ok(resolved)
    }

    pub fn remove_file(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<PathBuf, ReadError> {
        let resolved = self.resolve(virtual_path, path_id)?;
        std::fs::remove_file(&resolved)?;
        Ok(resolved)
    }

    pub fn rename_file(
        &self,
        old_virtual_path: &str,
        old_path_id: Option<&str>,
        new_virtual_path: &str,
        new_path_id: Option<&str>,
    ) -> std::result::Result<(PathBuf, PathBuf), ReadError> {
        let source = self.resolve_existing_file(old_virtual_path, old_path_id)?;
        let destination = self.resolve(new_virtual_path, new_path_id)?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&source, &destination)?;
        Ok((source, destination))
    }

    pub fn is_file_writable(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<bool, ReadError> {
        let path = self.resolve_existing_file(virtual_path, path_id)?;
        let permissions = std::fs::metadata(path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Ok(permissions.mode() & 0o200 != 0)
        }
        #[cfg(not(unix))]
        {
            Ok(!permissions.readonly())
        }
    }

    pub fn set_file_writable(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
        writable: bool,
    ) -> std::result::Result<PathBuf, ReadError> {
        let path = self.resolve_existing_file(virtual_path, path_id)?;
        let mut permissions = std::fs::metadata(&path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(if writable { 0o600 } else { 0o400 });
        }
        #[cfg(not(unix))]
        permissions.set_readonly(!writable);
        std::fs::set_permissions(&path, permissions)?;
        Ok(path)
    }

    fn resolve_existing_file(
        &self,
        virtual_path: &str,
        path_id: Option<&str>,
    ) -> std::result::Result<PathBuf, ReadError> {
        let normalized = normalize_write_path(virtual_path)?;
        let requested = path_id.filter(|path_id| !path_id.is_empty());
        for mount in &self.mounts {
            if requested.is_some_and(|wanted| !mount.path_id.eq_ignore_ascii_case(wanted)) {
                continue;
            }
            if requested.is_none() && mount.by_request_only {
                continue;
            }
            let candidate = mount.canonical_root.join(&normalized);
            let canonical = match std::fs::canonicalize(candidate) {
                Ok(path) => path,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(ReadError::Io(error)),
            };
            if !canonical.starts_with(&mount.canonical_root) {
                return Err(ReadError::EscapedRoot(canonical));
            }
            if canonical.is_file() {
                return Ok(canonical);
            }
        }
        Err(ReadError::NotFound(normalized))
    }

    fn find(&self, path_id: &str) -> Option<&WriteMount> {
        self.mounts
            .iter()
            .find(|mount| mount.path_id.eq_ignore_ascii_case(path_id))
    }
}

fn normalize_write_path(path: &str) -> Result<String> {
    let slash_normalized = path.replace('\\', "/");
    normalize_virtual_path(&slash_normalized)
}

fn validate_write_ancestor(root: &Path, candidate: &Path) -> std::result::Result<(), ReadError> {
    let mut existing = candidate;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| ReadError::EscapedRoot(candidate.to_owned()))?;
    }
    let canonical = std::fs::canonicalize(existing)?;
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err(ReadError::EscapedRoot(canonical))
    }
}

#[derive(Debug)]
pub enum FileError {
    InvalidMode(String),
    Missing(u64),
    Io(io::Error),
}

impl fmt::Display for FileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMode(mode) => write!(formatter, "unsupported file mode {mode:?}"),
            Self::Missing(handle) => write!(formatter, "file handle {handle} is not open"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FileError {}

impl From<io::Error> for FileError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug)]
struct FindCursor {
    entries: Vec<FindEntry>,
    next_index: usize,
}

/// Context-owned wildcard cursors. The first entry is returned when the cursor
/// is opened, matching the `FindFirst`/`FindNext` native contract.
#[derive(Debug, Default)]
pub struct OpenFinds {
    next_handle: u64,
    cursors: HashMap<u64, FindCursor>,
}

impl OpenFinds {
    pub fn new() -> Self {
        Self {
            next_handle: 1,
            cursors: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }

    pub fn open(&mut self, entries: Vec<FindEntry>) -> Option<(u64, FindEntry)> {
        let first = entries.first()?.clone();
        let handle = loop {
            let candidate = self.next_handle;
            self.next_handle = self.next_handle.wrapping_add(1).max(1);
            if candidate != 0 && !self.cursors.contains_key(&candidate) {
                break candidate;
            }
        };
        self.cursors.insert(
            handle,
            FindCursor {
                entries,
                next_index: 1,
            },
        );
        Some((handle, first))
    }

    pub fn next(&mut self, handle: u64) -> std::result::Result<Option<FindEntry>, u64> {
        let entry = self.peek(handle)?.cloned();
        self.advance(handle)?;
        Ok(entry)
    }

    pub fn peek(&self, handle: u64) -> std::result::Result<Option<&FindEntry>, u64> {
        let cursor = self.cursors.get(&handle).ok_or(handle)?;
        Ok(cursor.entries.get(cursor.next_index))
    }

    pub fn advance(&mut self, handle: u64) -> std::result::Result<(), u64> {
        let cursor = self.cursors.get_mut(&handle).ok_or(handle)?;
        if cursor.next_index < cursor.entries.len() {
            cursor.next_index += 1;
        }
        Ok(())
    }

    pub fn close(&mut self, handle: u64) -> bool {
        self.cursors.remove(&handle).is_some()
    }
}

#[derive(Debug)]
pub struct OpenFiles {
    next_handle: u64,
    files: HashMap<u64, OpenFile>,
}

#[derive(Debug)]
struct OpenFile {
    backend: FileBackend,
    writable: bool,
}

#[derive(Debug)]
enum FileBackend {
    Disk(std::fs::File),
    Memory(Cursor<Vec<u8>>),
    Pack(Cursor<Vec<u8>>),
}

impl Default for OpenFiles {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenFiles {
    pub fn new() -> Self {
        Self {
            next_handle: 1,
            files: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn open(&mut self, path: &Path, mode: &str) -> std::result::Result<(u64, u64), FileError> {
        let (options, writable) = open_options(mode)?;
        let file = options.open(path)?;
        let size = file.metadata()?.len();
        let handle = self.insert(OpenFile {
            backend: FileBackend::Disk(file),
            writable,
        });
        Ok((handle, size))
    }

    pub fn open_memory(&mut self, data: Vec<u8>) -> (u64, u64) {
        let size = data.len() as u64;
        let handle = self.insert(OpenFile {
            backend: FileBackend::Memory(Cursor::new(data)),
            writable: false,
        });
        (handle, size)
    }

    /// Validated pack bytes with seeks clamped to [0, size], matching compressed
    /// pack handles. Unlike ordinary memory/disk files, seeks cannot pass EOF.
    pub fn open_pack(&mut self, data: Vec<u8>) -> (u64, u64) {
        let size = data.len() as u64;
        let handle = self.insert(OpenFile {
            backend: FileBackend::Pack(Cursor::new(data)),
            writable: false,
        });
        (handle, size)
    }

    fn insert(&mut self, file: OpenFile) -> u64 {
        let handle = loop {
            let candidate = self.next_handle;
            self.next_handle = self.next_handle.wrapping_add(1).max(1);
            if candidate != 0 && !self.files.contains_key(&candidate) {
                break candidate;
            }
        };
        self.files.insert(handle, file);
        handle
    }

    pub fn close(&mut self, handle: u64) -> bool {
        self.files.remove(&handle).is_some()
    }

    pub fn read(
        &mut self,
        handle: u64,
        output: &mut [u8],
    ) -> std::result::Result<usize, FileError> {
        match &mut self.file_mut(handle)?.backend {
            FileBackend::Disk(file) => file.read(output).map_err(FileError::Io),
            FileBackend::Memory(cursor) | FileBackend::Pack(cursor) => {
                cursor.read(output).map_err(FileError::Io)
            }
        }
    }

    pub fn write(&mut self, handle: u64, input: &[u8]) -> std::result::Result<usize, FileError> {
        let file = self.file_mut(handle)?;
        if !file.writable {
            return Err(FileError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle is read-only",
            )));
        }
        match &mut file.backend {
            FileBackend::Disk(file) => file.write(input).map_err(FileError::Io),
            FileBackend::Memory(_) | FileBackend::Pack(_) => {
                unreachable!("writable memory handles are not created")
            }
        }
    }

    pub fn flush(&mut self, handle: u64) -> std::result::Result<(), FileError> {
        match &mut self.file_mut(handle)?.backend {
            FileBackend::Disk(file) => file.flush().map_err(FileError::Io),
            FileBackend::Memory(_) | FileBackend::Pack(_) => Ok(()),
        }
    }

    pub fn seek(
        &mut self,
        handle: u64,
        offset: i64,
        origin: u32,
    ) -> std::result::Result<u64, FileError> {
        let backend = &mut self.file_mut(handle)?.backend;
        if let FileBackend::Pack(cursor) = backend {
            let size = cursor.get_ref().len() as i128;
            let base = match origin {
                0 => 0,
                1 => cursor.position() as i128,
                2 => size,
                _ => return Err(FileError::InvalidMode(format!("seek origin {origin}"))),
            };
            let position = (base + offset as i128).clamp(0, size) as u64;
            cursor.set_position(position);
            return Ok(position);
        }
        let position = match origin {
            0 => SeekFrom::Start(
                u64::try_from(offset)
                    .map_err(|_| FileError::InvalidMode("negative start seek".to_owned()))?,
            ),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return Err(FileError::InvalidMode(format!("seek origin {origin}"))),
        };
        match backend {
            FileBackend::Disk(file) => file.seek(position).map_err(FileError::Io),
            FileBackend::Memory(cursor) => cursor.seek(position).map_err(FileError::Io),
            FileBackend::Pack(_) => unreachable!("pack seeks handled above"),
        }
    }

    pub fn tell(&mut self, handle: u64) -> std::result::Result<u64, FileError> {
        match &mut self.file_mut(handle)?.backend {
            FileBackend::Disk(file) => file.stream_position().map_err(FileError::Io),
            FileBackend::Memory(cursor) | FileBackend::Pack(cursor) => {
                cursor.stream_position().map_err(FileError::Io)
            }
        }
    }

    pub fn size(&self, handle: u64) -> std::result::Result<u64, FileError> {
        match &self.file(handle)?.backend {
            FileBackend::Disk(file) => file
                .metadata()
                .map(|value| value.len())
                .map_err(FileError::Io),
            FileBackend::Memory(cursor) | FileBackend::Pack(cursor) => {
                Ok(cursor.get_ref().len() as u64)
            }
        }
    }

    pub fn contains(&self, handle: u64) -> bool {
        self.files.contains_key(&handle)
    }

    fn file(&self, handle: u64) -> std::result::Result<&OpenFile, FileError> {
        self.files.get(&handle).ok_or(FileError::Missing(handle))
    }

    fn file_mut(&mut self, handle: u64) -> std::result::Result<&mut OpenFile, FileError> {
        self.files
            .get_mut(&handle)
            .ok_or(FileError::Missing(handle))
    }
}

fn open_options(mode: &str) -> std::result::Result<(std::fs::OpenOptions, bool), FileError> {
    let mut characters = mode.chars();
    let operation = characters
        .next()
        .ok_or_else(|| FileError::InvalidMode(mode.to_owned()))?;
    if characters.any(|character| !matches!(character, 'b' | 't' | '+')) {
        return Err(FileError::InvalidMode(mode.to_owned()));
    }
    let update = mode.contains('+');
    let writable = operation != 'r' || update;
    let mut options = std::fs::OpenOptions::new();
    match operation {
        'r' => {
            options.read(true).write(update);
        }
        'w' => {
            options.write(true).read(update).create(true).truncate(true);
        }
        'a' => {
            options.append(true).read(update).create(true);
        }
        _ => return Err(FileError::InvalidMode(mode.to_owned())),
    }
    Ok((options, writable))
}

fn vpk_chunk_path(directory_file: &Path, index: u16) -> PathBuf {
    let name = directory_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let base = name
        .strip_suffix("_dir.vpk")
        .or_else(|| name.strip_suffix(".vpk"))
        .unwrap_or(name);
    directory_file.with_file_name(format!("{base}_{index:03}.vpk"))
}

fn vpk_logical_path(directory_file: &Path) -> PathBuf {
    let Some(name) = directory_file.file_name().and_then(|name| name.to_str()) else {
        return directory_file.to_owned();
    };
    let Some(base) = name.strip_suffix("_dir.vpk") else {
        return directory_file.to_owned();
    };
    directory_file.with_file_name(format!("{base}.vpk"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory() -> PathBuf {
        std::env::temp_dir().join(format!(
            "source-filesystem-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn resolves_source_virtual_paths() {
        assert_eq!(
            resolve_content_path_utf8(Path::new("/game"), "hl2/cfg/valve.rc").unwrap(),
            "/game/hl2/cfg/valve.rc"
        );
        assert_eq!(
            normalize_virtual_path("materials/brick/test.vmt").unwrap(),
            "materials/brick/test.vmt"
        );
    }

    #[test]
    fn rejects_escaping_and_platform_ambiguous_paths() {
        for path in [
            "",
            "/absolute",
            "../escape",
            "safe/../escape",
            "safe//file",
            "safe/./file",
            "C:/drive",
            "safe\\file",
            "safe\0file",
        ] {
            assert!(normalize_virtual_path(path).is_err(), "accepted {path:?}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn keeps_resolved_paths_under_a_symlinked_search_root() {
        // The engine attributes an absolute path back to a search path by
        // prefix, so a root reached through a symlink has to keep reporting
        // paths under that root. Resolving the link instead leaves the engine
        // unable to place the file, which silently disables the audio source
        // cache and with it every sound.
        let root = temporary_directory().join("symlinked-root");
        let content = root.join("content");
        std::fs::create_dir_all(content.join("sound")).unwrap();
        std::fs::write(content.join("sound/shot.wav"), b"wav").unwrap();

        let mut builder = source_vpk::Builder::new();
        builder
            .add_file("cfg/packed.txt", b"packed".to_vec())
            .unwrap();
        std::fs::write(
            content.join("packed_dir.vpk"),
            builder.build_v1_embedded().unwrap(),
        )
        .unwrap();

        let mounted = root.join("mounted");
        std::os::unix::fs::symlink(&content, &mounted).unwrap();

        let mut paths = SearchPaths::new();
        paths
            .mount_directory_with_flags(&mounted, "GAME", Position::Tail, false, true)
            .unwrap();
        paths
            .mount_vpk(mounted.join("packed_dir.vpk"), "GAME", Position::Tail)
            .unwrap();

        let resolved = paths
            .resolve_read_path("sound/shot.wav", Some("GAME"))
            .unwrap();
        assert_eq!(resolved.kind, ReadPathKind::Disk);
        assert!(
            resolved.path.starts_with(&mounted),
            "disk path left the mounted root: {:?}",
            resolved.path
        );
        assert_eq!(std::fs::read(&resolved.path).unwrap(), b"wav");

        let packed = paths
            .resolve_read_path("cfg/packed.txt", Some("GAME"))
            .unwrap();
        assert_eq!(packed.kind, ReadPathKind::Vpk);
        assert!(
            packed.path.starts_with(&mounted),
            "VPK path left the mounted root: {:?}",
            packed.path
        );

        assert_eq!(paths.read("sound/shot.wav", Some("GAME")).unwrap(), b"wav");
        assert_eq!(paths.file_size("sound/shot.wav", Some("GAME")).unwrap(), 3);
        assert!(paths.is_directory("sound", Some("GAME")).unwrap());
        assert_eq!(
            paths.find("sound/*.wav", Some("GAME")).unwrap(),
            vec![FindEntry {
                name: "shot.wav".to_owned(),
                is_directory: false,
            }]
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enforces_search_order_and_path_ids_across_loose_and_vpk_mounts() {
        let root = temporary_directory();
        std::fs::create_dir_all(root.join("cfg")).unwrap();
        std::fs::write(root.join("cfg/test.txt"), b"loose").unwrap();
        std::fs::write(root.join("cfg/loose-only.txt"), b"loose only").unwrap();

        let mut builder = source_vpk::Builder::new();
        builder
            .add_file("cfg/test.txt", b"packed".to_vec())
            .unwrap();
        builder
            .add_file("cfg/packed.txt", b"only packed".to_vec())
            .unwrap();
        builder
            .add_file("cfg/packed-dir/nested.txt", b"nested".to_vec())
            .unwrap();
        let vpk_path = root.join("test_dir.vpk");
        std::fs::write(&vpk_path, builder.build_v1_embedded().unwrap()).unwrap();

        let mut paths = SearchPaths::new();
        paths.mount_vpk(&vpk_path, "GAME", Position::Tail).unwrap();
        paths
            .mount_directory(&root, "GAME", Position::Head)
            .unwrap();
        assert!(matches!(
            paths.open_read_source("cfg/test.txt", Some("GAME")),
            Ok(ReadSource::Disk(_))
        ));
        assert!(matches!(
            paths.open_read_source("cfg/packed.txt", Some("GAME")),
            Ok(ReadSource::Memory(data)) if data == b"only packed"
        ));
        assert_eq!(paths.read("cfg/test.txt", Some("game")).unwrap(), b"loose");
        assert_eq!(
            paths.read("cfg/packed.txt", Some("GAME")).unwrap(),
            b"only packed"
        );
        assert!(matches!(
            paths.read("cfg/test.txt", Some("MOD")),
            Err(ReadError::NotFound(_))
        ));

        paths.mount_vpk(&vpk_path, "GAME", Position::Head).unwrap();
        assert_eq!(paths.read("cfg/test.txt", None).unwrap(), b"packed");
        assert_eq!(paths.file_size("cfg/test.txt", None).unwrap(), 6);
        assert_eq!(paths.file_size("cfg/packed.txt", Some("GAME")).unwrap(), 11);
        assert_eq!(
            paths
                .resolve_read_path("cfg/test.txt", Some("GAME"))
                .unwrap(),
            ResolvedReadPath {
                // Rooted where the archive was mounted, not where its real path
                // lives, so the engine can still match it to a search path.
                path: vpk_logical_path(&vpk_path).join("cfg/test.txt"),
                kind: ReadPathKind::Vpk,
            }
        );
        assert_eq!(
            paths
                .resolve_read_path("cfg/loose-only.txt", Some("GAME"))
                .unwrap()
                .kind,
            ReadPathKind::Disk
        );
        assert_eq!(
            paths.resolve_read_path("cfg", Some("GAME")).unwrap().kind,
            ReadPathKind::Disk
        );
        assert!(matches!(
            paths.resolve_read_path("cfg/test.txt", Some("MOD")),
            Err(ReadError::NotFound(_))
        ));
        assert!(paths.is_directory("cfg", Some("GAME")).unwrap());
        assert!(!paths.is_directory("cfg/test.txt", Some("GAME")).unwrap());
        assert!(!paths.is_directory("missing", Some("GAME")).unwrap());
        assert_eq!(
            paths.find("CFG/*.TXT", Some("game")).unwrap(),
            vec![
                FindEntry {
                    name: "packed.txt".to_owned(),
                    is_directory: false,
                },
                FindEntry {
                    name: "test.txt".to_owned(),
                    is_directory: false,
                },
                FindEntry {
                    name: "loose-only.txt".to_owned(),
                    is_directory: false,
                },
            ]
        );
        assert_eq!(
            paths.find("cfg/packed-*", Some("GAME")).unwrap(),
            vec![FindEntry {
                name: "packed-dir".to_owned(),
                is_directory: true,
            }]
        );
        assert!(paths.find("cfg/missing*", Some("GAME")).unwrap().is_empty());
        assert!(paths.find("*/test.txt", Some("GAME")).is_err());
        assert!(matches!(
            paths.file_size("cfg/test.txt", Some("MOD")),
            Err(ReadError::NotFound(_))
        ));
        assert!(matches!(
            paths.read("../escape", None),
            Err(ReadError::Path(_))
        ));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owns_find_cursor_lifetimes_and_exhaustion() {
        assert!(wildcard_matches("*.V?T", "brick.vmt"));
        assert!(!wildcard_matches("*.vtf", "brick.vmt"));
        assert!(wildcard_matches("*.*", "cfg"));

        let entries = vec![
            FindEntry {
                name: "first.cfg".to_owned(),
                is_directory: false,
            },
            FindEntry {
                name: "directory".to_owned(),
                is_directory: true,
            },
        ];
        let mut finds = OpenFinds::new();
        assert!(finds.open(Vec::new()).is_none());
        let (handle, first) = finds.open(entries).unwrap();
        assert_eq!(first.name, "first.cfg");
        assert_eq!(finds.len(), 1);
        assert_eq!(finds.next(handle).unwrap().unwrap().name, "directory");
        assert!(finds.next(handle).unwrap().is_none());
        assert!(finds.next(handle).unwrap().is_none());
        assert!(finds.close(handle));
        assert!(finds.is_empty());
        assert_eq!(finds.next(handle), Err(handle));
        assert!(!finds.close(handle));
    }

    #[test]
    fn synchronizes_read_visibility_and_clears_mounts_without_dropping_vpk_cache() {
        let root = temporary_directory().join("read-flags");
        let visible = root.join("visible");
        let requested = root.join("requested");
        for path in [&visible, &requested] {
            std::fs::create_dir_all(path.join("cfg")).unwrap();
        }
        std::fs::write(visible.join("cfg/value.txt"), b"visible").unwrap();
        std::fs::write(requested.join("cfg/value.txt"), b"requested").unwrap();

        let mut paths = SearchPaths::new();
        paths
            .mount_directory_with_flags(&requested, "SPECIAL", Position::Tail, true, false)
            .unwrap();
        paths
            .mount_directory_with_flags(&visible, "GAME", Position::Tail, false, false)
            .unwrap();
        assert_eq!(paths.read("cfg/value.txt", None).unwrap(), b"visible");
        assert_eq!(
            paths.read("cfg/value.txt", Some("special")).unwrap(),
            b"requested"
        );
        paths.clear();
        assert!(paths.is_empty());
        assert!(matches!(
            paths.read("cfg/value.txt", None),
            Err(ReadError::NotFound(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn native_directory_mounts_can_follow_explicitly_trusted_symlinks() {
        use std::os::unix::fs::symlink;

        let root = temporary_directory().join("symlink-root");
        let external = temporary_directory().join("symlink-external");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("asset.txt"), b"external").unwrap();
        symlink(&external, root.join("linked")).unwrap();

        let mut strict = SearchPaths::new();
        strict
            .mount_directory(&root, "GAME", Position::Tail)
            .unwrap();
        assert!(matches!(
            strict.read("linked/asset.txt", Some("GAME")),
            Err(ReadError::EscapedRoot(_))
        ));

        let mut native = SearchPaths::new();
        native
            .mount_directory_with_flags(&root, "GAME", Position::Tail, false, true)
            .unwrap();
        assert_eq!(
            native.read("linked/asset.txt", Some("GAME")).unwrap(),
            b"external"
        );
        assert!(matches!(
            strict.is_directory("linked", Some("GAME")),
            Err(ReadError::EscapedRoot(_))
        ));
        assert!(native.is_directory("linked", Some("GAME")).unwrap());
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(external).unwrap();
    }

    #[test]
    fn resolves_source_write_precedence_separately_from_reads() {
        let root = temporary_directory().join("writes");
        let first = root.join("first");
        let default = root.join("default");
        let game = root.join("game");
        let mod_path = root.join("mod");
        for path in [&first, &default, &game, &mod_path] {
            std::fs::create_dir_all(path).unwrap();
        }

        let mut paths = WritePaths::new();
        paths
            .mount_directory(&first, "GAME", Position::Tail)
            .unwrap();
        paths
            .mount_directory(&default, "DEFAULT_WRITE_PATH", Position::Tail)
            .unwrap();
        paths
            .mount_directory(&game, "GAME_WRITE", Position::Tail)
            .unwrap();
        paths
            .mount_directory(&mod_path, "MOD_WRITE", Position::Tail)
            .unwrap();

        let canonical_default = std::fs::canonicalize(&default).unwrap();
        let canonical_game = std::fs::canonicalize(&game).unwrap();
        let canonical_mod = std::fs::canonicalize(&mod_path).unwrap();

        assert_eq!(
            paths.resolve("cfg/config.cfg", Some("GAME")).unwrap(),
            canonical_game.join("cfg/config.cfg")
        );
        assert_eq!(
            paths.resolve("save/test.sav", Some("mod")).unwrap(),
            canonical_mod.join("save/test.sav")
        );
        assert_eq!(
            paths.resolve("screenshots/test.tga", None).unwrap(),
            canonical_default.join("screenshots/test.tga")
        );
        assert_eq!(
            paths.resolve("cfg\\autoexec.cfg", Some("unknown")).unwrap(),
            canonical_default.join("cfg/autoexec.cfg")
        );
        assert!(matches!(
            paths.resolve("../escape", Some("GAME")),
            Err(ReadError::Path(_))
        ));

        paths.clear();
        assert!(paths.is_empty());
        assert!(matches!(
            paths.resolve("cfg/config.cfg", Some("GAME")),
            Err(ReadError::NotFound(_))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owns_relative_directory_creation_and_file_removal() {
        let root = temporary_directory().join("mutations");
        std::fs::create_dir_all(&root).unwrap();
        let mut paths = WritePaths::new();
        paths
            .mount_directory(&root, "DEFAULT_WRITE_PATH", Position::Tail)
            .unwrap();
        let directory = paths
            .create_directory_all("screenshots/nested/", None)
            .unwrap();
        assert!(directory.is_dir());
        let file = directory.join("capture.tga");
        std::fs::write(&file, b"capture").unwrap();
        assert_eq!(
            paths
                .remove_file("screenshots/nested/capture.tga", None)
                .unwrap(),
            file
        );
        assert!(!file.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owns_relative_rename_and_writability_with_search_filters() {
        let root = temporary_directory().join("metadata");
        let hidden = root.join("hidden");
        let game = root.join("game");
        let destination = root.join("destination");
        for path in [&hidden, &game, &destination] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(hidden.join("state.cfg"), b"hidden").unwrap();
        std::fs::write(game.join("state.cfg"), b"game").unwrap();

        let mut paths = WritePaths::new();
        paths
            .mount_directory_with_request_only(&hidden, "HIDDEN", Position::Tail, true)
            .unwrap();
        paths
            .mount_directory(&game, "GAME", Position::Tail)
            .unwrap();
        paths
            .mount_directory(&destination, "GAME_WRITE", Position::Tail)
            .unwrap();

        assert!(paths.is_file_writable("state.cfg", None).unwrap());
        paths
            .set_file_writable("state.cfg", Some("GAME"), false)
            .unwrap();
        assert!(!paths.is_file_writable("state.cfg", Some("GAME")).unwrap());
        paths
            .set_file_writable("state.cfg", Some("GAME"), true)
            .unwrap();
        let (source, renamed) = paths
            .rename_file("state.cfg", Some("GAME"), "cfg/renamed.cfg", Some("GAME"))
            .unwrap();
        assert_eq!(
            source,
            std::fs::canonicalize(&game).unwrap().join("state.cfg")
        );
        assert_eq!(
            renamed,
            std::fs::canonicalize(&destination)
                .unwrap()
                .join("cfg/renamed.cfg")
        );
        assert_eq!(std::fs::read(renamed).unwrap(), b"game");
        assert_eq!(std::fs::read(hidden.join("state.cfg")).unwrap(), b"hidden");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owns_disk_and_memory_file_handles_and_cursor_state() {
        let root = temporary_directory().join("handles");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("state.bin");
        let mut files = OpenFiles::new();
        let (handle, size) = files.open(&path, "w+b").unwrap();
        assert_eq!(size, 0);
        assert_eq!(files.write(handle, b"source").unwrap(), 6);
        files.flush(handle).unwrap();
        assert_eq!(files.tell(handle).unwrap(), 6);
        assert_eq!(files.size(handle).unwrap(), 6);
        assert_eq!(files.seek(handle, 0, 0).unwrap(), 0);
        let mut output = [0u8; 6];
        assert_eq!(files.read(handle, &mut output).unwrap(), 6);
        assert_eq!(&output, b"source");
        assert!(files.contains(handle));
        assert!(files.close(handle));
        assert!(files.is_empty());
        assert!(matches!(files.tell(handle), Err(FileError::Missing(_))));
        assert!(matches!(
            files.open(&path, "invalid"),
            Err(FileError::InvalidMode(_))
        ));

        let (memory, memory_size) = files.open_memory(b"packed-content".to_vec());
        assert_eq!(memory_size, 14);
        let mut memory_output = [0u8; 6];
        assert_eq!(files.read(memory, &mut memory_output).unwrap(), 6);
        assert_eq!(&memory_output, b"packed");
        assert_eq!(files.tell(memory).unwrap(), 6);
        assert_eq!(files.seek(memory, -7, 2).unwrap(), 7);
        assert_eq!(files.size(memory).unwrap(), 14);
        assert!(matches!(
            files.write(memory, b"no"),
            Err(FileError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert!(files.close(memory));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pack_cursors_clamp_seeks_and_remain_read_only() {
        let mut files = OpenFiles::new();
        let (file, size) = files.open_pack(b"payload".to_vec());
        assert_eq!(size, 7);
        assert_eq!(files.seek(file, i64::MAX, 0).unwrap(), 7);
        assert_eq!(files.seek(file, i64::MIN, 1).unwrap(), 0);
        assert_eq!(files.seek(file, -2, 2).unwrap(), 5);
        let mut bytes = [0; 10];
        assert_eq!(files.read(file, &mut bytes).unwrap(), 2);
        assert_eq!(&bytes[..2], b"ad");
        assert_eq!(files.read(file, &mut bytes).unwrap(), 0);
        assert!(files.seek(file, 0, 3).is_err());
        assert_eq!(files.tell(file).unwrap(), 7);
        assert_eq!(files.seek(file, -1, 0).unwrap(), 0);
        assert!(files.write(file, b"x").is_err());
        files.flush(file).unwrap();
        assert_eq!(files.size(file).unwrap(), 7);
        assert!(files.close(file));
        assert!(!files.close(file));
        assert!(files.read(file, &mut bytes).is_err());
        let (empty, size) = files.open_pack(Vec::new());
        assert_eq!(size, 0);
        assert_eq!(files.seek(empty, i64::MAX, 2).unwrap(), 0);
        assert_eq!(files.read(empty, &mut bytes).unwrap(), 0);
    }

    #[test]
    fn pack_find_uses_shared_globs_and_canonical_deduplicated_children() {
        let bytes = pakfile(&[
            ("README", b"root"),
            ("__preload_section.pre", b"cache"),
            ("cfg/a.vmt", b"a"),
            ("cfg/b.vmt", b"b"),
            ("cfg/multi.part.vmt", b"m"),
            ("cfg/noext", b"n"),
            ("cfg/sub/one.txt", b"1"),
            ("cfg/sub/two.txt", b"2"),
            ("cfg/dot.name/a", b"d"),
            ("cfg/same", b"f"),
            ("cfg/same/child", b"d"),
        ]);
        let index = source_pak::Index::parse(&bytes).unwrap();
        let names = |pattern| {
            find_pack_index(&index, pattern)
                .unwrap()
                .into_iter()
                .map(|entry| (entry.name, entry.is_directory))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names("*.*"),
            vec![("readme".into(), false), ("cfg".into(), true)]
        );
        assert_eq!(
            names("CFG\\.\\sub\\..\\*.VMT"),
            vec![
                ("cfg/a.vmt".into(), false),
                ("cfg/b.vmt".into(), false),
                ("cfg/multi.part.vmt".into(), false),
            ]
        );
        assert_eq!(
            names("cfg/*a*.?mt"),
            vec![
                ("cfg/a.vmt".into(), false),
                ("cfg/multi.part.vmt".into(), false)
            ]
        );
        assert_eq!(
            names("cfg/s*"),
            vec![("cfg/same".into(), false), ("cfg/sub".into(), true)]
        );
        assert_eq!(names("cfg/dot.*"), vec![("cfg/dot.name".into(), true)]);
        assert_eq!(names("cfg//noext"), vec![("cfg/noext".into(), false)]);
        assert!(names("missing/*").is_empty());
        let mut paths = SearchPaths::new();
        paths
            .mount_pak(&bytes, "fixture", "GAME", Position::Head)
            .unwrap();
        assert_eq!(paths.find("CFG/*.VMT", Some("GAME")).unwrap().len(), 3);
        assert_eq!(paths.find("*.*", Some("GAME")).unwrap().len(), 2);
    }

    #[test]
    fn pack_find_rejects_escape_and_unsupported_directory_patterns() {
        let index = source_pak::Index::parse(&pakfile(&[("a.txt", b"a")])).unwrap();
        for pattern in [
            "",
            "/a*",
            "\\a*",
            "C:a*",
            "a\0b",
            "../*",
            "a/../../*",
            "a*/b",
            "?/b",
            "a/",
            "a/.",
            "a/..",
        ] {
            assert!(find_pack_index(&index, pattern).is_err(), "{pattern:?}");
        }
        assert!(find_pack_index(&index, &"x".repeat(MAX_VIRTUAL_PATH_BYTES + 1)).is_err());
    }

    /// Builds a stored-only ZIP the way a map's pakfile lump holds one.
    fn pakfile(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut directory = Vec::new();
        for (name, data) in files {
            let offset = bytes.len() as u32;
            let crc = source_vpk::crc32(data);
            bytes.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            // Version, flags, method and timestamp, up to the CRC at 14.
            // A zeroed method is `stored`, which is all a pakfile uses.
            bytes.extend_from_slice(&[0; 10]);
            bytes.extend_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(data);

            directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            // Versions, flags, method and timestamp, up to the CRC at 16.
            directory.extend_from_slice(&[0; 12]);
            directory.extend_from_slice(&crc.to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 8]);
            directory.extend_from_slice(&[0; 4]);
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let directory_offset = bytes.len() as u32;
        let directory_size = directory.len() as u32;
        bytes.extend_from_slice(&directory);
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&directory_size.to_le_bytes());
        bytes.extend_from_slice(&directory_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    #[test]
    fn file_paks_preserve_range_precedence_visibility_and_handle_lifetime() {
        let root = temporary_directory();
        std::fs::create_dir_all(root.join("materials")).unwrap();
        std::fs::write(root.join("materials/a.vmt"), b"loose").unwrap();
        let zip = pakfile(&[
            ("materials/a.vmt", b"packed"),
            ("materials/b.vmt", b"unique"),
        ]);
        let archive = root.join("test.bsp");
        let mut bsp = vec![0; 1036];
        bsp.extend_from_slice(&zip);
        bsp.extend_from_slice(b"unrelated BSP suffix");
        std::fs::write(&archive, bsp).unwrap();
        let mut paths = SearchPaths::new();
        paths
            .mount_directory(&root, "GAME", Position::Tail)
            .unwrap();
        paths
            .mount_pak_file_with_flags(
                &archive,
                1036,
                zip.len() as u64,
                "GAME",
                Position::Tail,
                false,
            )
            .unwrap();
        assert_eq!(paths.read("materials/a.vmt", None).unwrap(), b"loose");
        paths
            .mount_pak_file_with_flags(
                &archive,
                1036,
                zip.len() as u64,
                "PRIVATE",
                Position::Head,
                true,
            )
            .unwrap();
        assert_eq!(paths.read("materials/a.vmt", None).unwrap(), b"loose");
        assert_eq!(
            paths.read("materials/a.vmt", Some("PRIVATE")).unwrap(),
            b"packed"
        );
        assert_eq!(
            paths
                .resolve_read_path("materials/b.vmt", None)
                .unwrap()
                .path,
            archive.join("materials/b.vmt")
        );
        assert_eq!(paths.file_size("materials/b.vmt", None).unwrap(), 6);
        assert!(paths.is_directory("materials", None).unwrap());
        assert_eq!(paths.find("materials/*.vmt", None).unwrap().len(), 2);
        assert_eq!(
            paths.pak_cache.len(),
            1,
            "different IDs share the parsed range"
        );
        let pak = Arc::clone(paths.pak_cache.values().next().unwrap());
        let mut files = OpenFiles::new();
        let (handle, _) = files.open_memory(paths.read("materials/b.vmt", None).unwrap());
        paths.clear();
        paths
            .mount_pak_file_with_flags(
                &archive,
                1036,
                zip.len() as u64,
                "GAME",
                Position::Head,
                false,
            )
            .unwrap();
        assert!(Arc::ptr_eq(&pak, paths.pak_cache.values().next().unwrap()));
        drop(pak);
        paths.clear();
        paths.clear();
        assert!(
            paths.pak_cache.is_empty(),
            "unmounted maps must be released"
        );
        let mut output = [0; 3];
        files.seek(handle, 3, 0).unwrap();
        files.read(handle, &mut output).unwrap();
        assert_eq!(&output, b"que");
        assert!(files.close(handle));
        for (offset, length) in [
            (u64::MAX, 1),
            (0, u64::MAX),
            (1036, zip.len() as u64 + 100),
            (0, zip.len() as u64),
        ] {
            assert!(paths
                .mount_pak_file_with_flags(&archive, offset, length, "GAME", Position::Tail, false)
                .is_err());
            assert!(paths.is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bsp_selection_uses_typed_map_provenance_for_every_query() {
        use pack_archive::{Archive, Kind};
        let root = temporary_directory();
        std::fs::create_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("dir/a.txt"), b"loose").unwrap();
        let zip = pakfile(&[("dir/a.txt", b"zip"), ("zip-only/a", b"zip")]);
        let zip_path = root.join("standalone.bsp");
        std::fs::write(&zip_path, &zip).unwrap();
        let zip = Arc::new(Archive::open(&zip_path, Kind::Zip).unwrap());
        assert_eq!(zip.kind(), Some(Kind::Zip));
        let bytes = pakfile(&[("dir/a.txt", b"map")]);
        let mut bsp = vec![0; source_bsp::BSP_HEADER_SIZE];
        let offset = bsp.len() as u64;
        bsp[..4].copy_from_slice(b"VBSP");
        bsp[4..8].copy_from_slice(&20i32.to_le_bytes());
        bsp[648..652].copy_from_slice(&(offset as i32).to_le_bytes());
        bsp[652..656].copy_from_slice(&(bytes.len() as i32).to_le_bytes());
        bsp.extend_from_slice(&bytes);
        let bsp_path = root.join("embedded.zip");
        std::fs::write(&bsp_path, &bsp).unwrap();
        let map = Arc::new(Archive::open(&bsp_path, Kind::Bsp).unwrap());
        let range = Arc::new(Archive::open_range(&bsp_path, offset, bytes.len() as u64).unwrap());
        assert_eq!(map.kind(), Some(Kind::Bsp));
        assert_eq!(range.kind(), None);
        let mut paths = SearchPaths::new();
        paths
            .mount_directory(&root, "GAME", Position::Head)
            .unwrap();
        paths
            .mount_pack_archive(Arc::clone(&zip), "GAME", Position::Head, false)
            .unwrap();
        paths.mount_directory(&root, "BSP", Position::Head).unwrap();
        paths
            .mount_pack_archive(Arc::clone(&range), "GAME", Position::Head, false)
            .unwrap();
        paths
            .mount_pack_archive(Arc::clone(&map), "MOD", Position::Head, true)
            .unwrap();
        assert!(matches!(
            paths.read("dir/a.txt", Some("BSP")),
            Err(ReadError::NotFound(_))
        ));
        assert!(paths.find("*", Some("BSP")).unwrap().is_empty());
        paths
            .mount_pack_archive(Arc::clone(&map), "GAME", Position::Tail, true)
            .unwrap();
        let name = "DIR\\.\\sub\\..\\A.TXT";
        assert_eq!(paths.read(name, Some("bSp")).unwrap(), b"map");
        assert_eq!(paths.file_size(name, Some("BSP")).unwrap(), 3);
        assert_eq!(
            paths.resolve_read_path(name, Some("BSP")).unwrap().path,
            bsp_path.join("dir/a.txt")
        );
        assert!(paths.is_directory("DIR/./sub/..", Some("BSP")).unwrap());
        assert!(!paths.is_directory("zip-only", Some("BSP")).unwrap());
        assert_eq!(
            paths.find("DIR/./sub/../*.TXT", Some("BSP")).unwrap(),
            vec![FindEntry {
                name: "a.txt".into(),
                is_directory: false
            }]
        );
        assert!(paths.read("../dir/a.txt", Some("BSP")).is_err());
        assert!(paths.find("dir/*/../*", Some("BSP")).is_err());
        paths.clear();
        paths
            .mount_pak(&bytes, "memory", "GAME", Position::Head)
            .unwrap();
        assert_eq!(paths.read("dir/a.txt", Some("BSP")).unwrap(), b"map");
        drop(paths);
        drop(map);
        drop(range);
        drop(zip);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_archive_mounts_retain_one_owner_without_reopening() {
        use pack_archive::{Archive, Kind};
        let root = temporary_directory();
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("shared.zip");
        let bytes = pakfile(&[
            ("dir/a.txt", b"original"),
            ("__preload_section.pre", b"hidden"),
        ]);
        std::fs::write(&path, &bytes).unwrap();
        let archive = Arc::new(Archive::open(&path, Kind::Zip).unwrap());
        let weak = Arc::downgrade(&archive);
        assert_eq!(archive.index().entries().len(), 1);
        assert_eq!(archive.info().length, bytes.len() as u64);
        let mut paths = SearchPaths::new();
        paths
            .mount_pack_archive(Arc::clone(&archive), "PRIVATE", Position::Head, true)
            .unwrap();
        assert!(matches!(
            paths.read("dir/a.txt", None),
            Err(ReadError::NotFound(_))
        ));
        assert_eq!(
            paths.read("dir/a.txt", Some("private")).unwrap(),
            b"original"
        );
        std::fs::rename(&path, root.join("renamed.zip")).unwrap();
        std::fs::write(&path, pakfile(&[("dir/a.txt", b"replacement")])).unwrap();
        paths
            .mount_pack_archive(Arc::clone(&archive), "GAME", Position::Tail, false)
            .unwrap();
        assert!(
            paths.pak_cache.is_empty(),
            "shared mounts must not reopen or populate a second cache"
        );
        assert_eq!(Arc::strong_count(&archive), 3);
        assert_eq!(paths.read("dir/a.txt", None).unwrap(), b"original");
        assert_eq!(paths.file_size("dir/a.txt", None).unwrap(), 8);
        assert!(paths.is_directory("DIR", None).unwrap());
        assert_eq!(paths.find("DIR/*.TXT", None).unwrap().len(), 1);
        assert_eq!(
            paths.resolve_read_path("dir/a.txt", None).unwrap().path,
            path.join("dir/a.txt")
        );
        assert!(paths.file_size("__preload_section.pre", None).is_err());
        let mut files = OpenFiles::new();
        let (file, _) = files.open_pack(paths.read("dir/a.txt", None).unwrap());
        drop(archive);
        assert_eq!(paths.read("dir/a.txt", None).unwrap(), b"original");
        paths.clear();
        assert!(
            weak.upgrade().is_none(),
            "last mount must release descriptor and metadata"
        );
        let mut output = [0; 8];
        files.read(file, &mut output).unwrap();
        assert_eq!(&output, b"original");
        assert!(files.close(file));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_archive_corruption_rejects_without_loose_fallback() {
        use pack_archive::{Archive, Kind};
        let root = temporary_directory();
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("shared.zip");
        std::fs::write(&path, pakfile(&[("a", b"original")])).unwrap();
        std::fs::write(root.join("a"), b"loose").unwrap();
        let archive = Arc::new(Archive::open(&path, Kind::Zip).unwrap());
        let mut paths = SearchPaths::new();
        paths
            .mount_directory(&root, "GAME", Position::Tail)
            .unwrap();
        paths
            .mount_pack_archive(Arc::clone(&archive), "GAME", Position::Head, false)
            .unwrap();
        let mut writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        writer
            .seek(SeekFrom::Start(archive.entry("a").unwrap().data_offset()))
            .unwrap();
        writer.write_all(b"!").unwrap();
        assert!(matches!(
            archive.read("a"),
            Err(pack_archive::Error::Pak(
                source_pak::Error::CrcMismatch { .. }
            ))
        ));
        assert!(matches!(
            paths.read("a", None),
            Err(ReadError::Pak(source_pak::Error::CrcMismatch { .. }))
        ));
        drop(writer);
        paths.clear();
        drop(archive);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memory_and_metadata_only_archives_keep_distinct_payload_contracts() {
        use pack_archive::Archive;
        let bytes = pakfile(&[("dir/a", b"bytes"), ("__preload_section.pre", b"hidden")]);
        let memory = Archive::memory(&bytes).unwrap();
        let metadata = Arc::new(Archive::metadata_only(&bytes).unwrap());
        assert_eq!(memory.read("dir/a").unwrap(), b"bytes");
        assert!(matches!(
            metadata.read_entry(0),
            Err(pack_archive::Error::InvalidArgument)
        ));
        assert!(matches!(
            metadata.read_entry(99),
            Err(pack_archive::Error::NotFound)
        ));
        assert!(memory.entry("__preload_section.pre").is_none());
        let mut paths = SearchPaths::new();
        assert!(paths
            .mount_pack_archive(metadata, "GAME", Position::Head, false)
            .is_err());
        paths
            .mount_pak(&bytes, "memory", "GAME", Position::Head)
            .unwrap();
        assert_eq!(paths.read("dir/a", None).unwrap(), b"bytes");
        assert!(paths.read("__preload_section.pre", None).is_err());
    }

    #[test]
    fn changed_paks_reload_and_corrupt_winners_do_not_fall_through() {
        let root = temporary_directory();
        std::fs::create_dir_all(&root).unwrap();
        let archive = root.join("pak.zip");
        let zip = pakfile(&[("a", b"old")]);
        std::fs::write(&archive, &zip).unwrap();
        std::fs::write(root.join("a"), b"loose").unwrap();
        let mut paths = SearchPaths::new();
        paths
            .mount_pak_file_with_flags(&archive, 0, zip.len() as u64, "GAME", Position::Tail, false)
            .unwrap();
        assert_eq!(paths.read("a", None).unwrap(), b"old");
        paths.clear();
        let mut zip = pakfile(&[("a", b"new content")]);
        std::fs::write(&archive, &zip).unwrap();
        paths
            .mount_pak_file_with_flags(&archive, 0, zip.len() as u64, "GAME", Position::Head, false)
            .unwrap();
        assert_eq!(paths.read("a", None).unwrap(), b"new content");
        paths.clear();
        zip[31] ^= 1;
        // A different physical path also avoids timestamp granularity assumptions.
        let corrupt = root.join("bad.zip");
        std::fs::write(&corrupt, &zip).unwrap();
        paths
            .mount_directory(&root, "GAME", Position::Tail)
            .unwrap();
        paths
            .mount_pak_file_with_flags(&corrupt, 0, zip.len() as u64, "GAME", Position::Head, false)
            .unwrap();
        assert!(matches!(
            paths.read("a", None),
            Err(ReadError::Pak(source_pak::Error::CrcMismatch { .. }))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn serves_a_maps_own_content_ahead_of_the_games() {
        // A map's cubemap materials exist only in its pakfile, and a map is
        // also allowed to ship its own version of a material the game
        // already has. Both need the map's archive searched first.
        let root = temporary_directory().join("pak");
        std::fs::create_dir_all(root.join("materials/brick")).unwrap();
        std::fs::write(root.join("materials/brick/wall.vmt"), b"from the game").unwrap();

        let patch: &[u8] = b"\"patch\" { \"include\" \"materials/brick/wall.vmt\" }";
        let embedded = pakfile(&[
            ("materials/brick/wall.vmt", b"from the map"),
            ("materials/maps/test/brick/wall_0_0_0.vmt", patch),
        ]);

        let mut paths = SearchPaths::new();
        paths
            .mount_directory(&root, "GAME", Position::Tail)
            .unwrap();
        paths
            .mount_pak(&embedded, "test", "GAME", Position::Head)
            .unwrap();

        assert_eq!(
            paths.read("materials/brick/wall.vmt", None).unwrap(),
            b"from the map"
        );
        assert!(paths
            .read("materials/maps/test/brick/wall_0_0_0.vmt", None)
            .unwrap()
            .starts_with(b"\"patch\""));
        assert_eq!(
            paths
                .file_size("materials/maps/test/brick/wall_0_0_0.vmt", None)
                .unwrap(),
            patch.len() as u64
        );
        assert_eq!(
            paths
                .resolve_read_path("materials/maps/test/brick/wall_0_0_0.vmt", None)
                .unwrap()
                .kind,
            ReadPathKind::Pak
        );
        assert!(paths.is_directory("materials/maps/test", None).unwrap());
        assert!(!paths.is_directory("materials/maps/other", None).unwrap());

        let found = paths.find("materials/maps/test/brick/*", None).unwrap();
        assert_eq!(
            found,
            vec![FindEntry {
                name: "wall_0_0_0.vmt".to_owned(),
                is_directory: false,
            }]
        );
        let directories = paths.find("materials/maps/*", None).unwrap();
        assert_eq!(
            directories,
            vec![FindEntry {
                name: "test".to_owned(),
                is_directory: true,
            }]
        );

        // A map that embeds nothing mounts without error and serves the
        // game's content unchanged.
        let mut empty = SearchPaths::new();
        empty
            .mount_directory(&root, "GAME", Position::Tail)
            .unwrap();
        empty
            .mount_pak(&[], "bare", "GAME", Position::Head)
            .unwrap();
        assert_eq!(
            empty.read("materials/brick/wall.vmt", None).unwrap(),
            b"from the game"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
