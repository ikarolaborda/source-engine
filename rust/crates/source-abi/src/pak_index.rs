//! Archive metadata and file-backed payload ownership for native pack adapters.
use super::*;
use source_filesystem::pack_archive::{Archive as ArchiveIndex, Error as ArchiveError, Kind};
use source_pak::Entry;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SourceAbiPakEntry {
    pub data_offset: u64,
    pub length: u32,
    pub compressed_length: u32,
    pub method: u32,
    pub index: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SourceAbiPakArchiveInfo {
    pub offset: u64,
    pub length: u64,
    pub modified_seconds: i64,
    pub entry_count: u32,
    pub reserved: u32,
}

impl SourceAbiPakEntry {
    fn new(entry: &Entry, index: usize) -> Self {
        Self {
            data_offset: entry.data_offset(),
            length: entry.length,
            compressed_length: entry.compressed_length,
            method: entry.method as u32,
            index: index as u32,
            crc32: entry.crc32,
            reserved: 0,
        }
    }
}

fn archive_error_status(error: ArchiveError) -> SourceAbiStatus {
    match error {
        ArchiveError::InvalidArgument => SOURCE_ABI_INVALID_ARGUMENT,
        ArchiveError::Io(_) => SOURCE_ABI_IO_ERROR,
        ArchiveError::Pak(_) | ArchiveError::Bsp(_) => SOURCE_ABI_FORMAT_ERROR,
        ArchiveError::NotFound => SOURCE_ABI_NOT_FOUND,
        ArchiveError::Allocation => SOURCE_ABI_INTERNAL_ERROR,
    }
}

fn indexes() -> &'static Mutex<HashMap<u64, Arc<ArchiveIndex>>> {
    static INDEXES: OnceLock<Mutex<HashMap<u64, Arc<ArchiveIndex>>>> = OnceLock::new();
    INDEXES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get(handle: u64) -> Option<Arc<ArchiveIndex>> {
    indexes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&handle)
        .cloned()
}

/// Add a mount sharing an existing file-backed archive. Neither the path nor
/// archive bytes are reopened. Destroying the index handle leaves the mount
/// alive; clearing mounts leaves other index/mount/file references alive.
///
/// # Safety
/// Path-ID slice describes readable UTF-8 bytes for this call.
#[no_mangle]
pub unsafe extern "C" fn source_context_read_path_add_pak_index(
    context: u64,
    index: u64,
    path_id: SourceAbiSlice,
    at_head: u8,
    by_request_only: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if at_head > 1 || by_request_only > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let Some(index) = get(index) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };
        if index.path().is_none() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let position = if at_head == 1 {
            source_filesystem::Position::Head
        } else {
            source_filesystem::Position::Tail
        };
        let mut filesystem = context.filesystem.lock().unwrap_or_else(|e| e.into_inner());
        match filesystem.mount_pack_archive(index, path_id, position, by_request_only != 0) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

/// Snapshot numbered archive paths in mount-precedence order. Naming 0 is
/// desktop, 1 Xbox360; an empty language disables localized discovery. Consume
/// with context find_next/find_close. No native pointers or file handles retained.
///
/// # Safety
/// Input slices are readable UTF-8; output and distinct result pointers writable.
#[no_mangle]
pub unsafe extern "C" fn source_context_find_pack_candidates(
    context: u64,
    root: SourceAbiSlice,
    naming: u32,
    language: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    out_is_directory: *mut u32,
    out_find: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() || out_is_directory.is_null() || out_find.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_written.write(0);
            out_is_directory.write(0);
            out_find.write(0);
        }
        if (output.length != 0 && output.data.is_null()) || checked_len(output.length).is_err() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        use source_filesystem::pack_mounts::{discover, Naming};
        let naming = match naming {
            0 => Naming::Desktop,
            1 => Naming::Xbox360,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let root = match unsafe { read_utf8_slice(root) } {
            Ok(root) => root,
            Err(status) => return status,
        };
        let language = match unsafe { read_optional_utf8_slice(language) } {
            Ok(language) => language,
            Err(status) => return status,
        };
        let entries = match discover(std::path::Path::new(root), naming, language) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                return SOURCE_ABI_INVALID_ARGUMENT
            }
            Err(_) => return SOURCE_ABI_IO_ERROR,
        };
        let Some(first) = entries.first() else {
            return SOURCE_ABI_NOT_FOUND;
        };
        let status = copy_find_entry(first, output, out_written, out_is_directory);
        if status != SOURCE_ABI_OK {
            return status;
        }
        let opened = context
            .finds
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .open(entries);
        let Some((find, _)) = opened else {
            return SOURCE_ABI_INTERNAL_ERROR;
        };
        unsafe { out_find.write(find) };
        SOURCE_ABI_OK
    })
}

/// Snapshot matching files and implied directories into a context-owned find
/// cursor. The cursor survives index destruction. Names are canonical relative
/// paths, files before directories, sorted within each kind. Short output does
/// not publish a cursor; next/close use the ordinary context find APIs.
///
/// # Safety
/// Pattern is readable UTF-8; output and distinct result pointers are writable.
#[no_mangle]
pub unsafe extern "C" fn source_context_find_first_pak(
    context: u64,
    index: u64,
    pattern: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    out_is_directory: *mut u32,
    out_find: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() || out_is_directory.is_null() || out_find.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_written.write(0);
            out_is_directory.write(0);
            out_find.write(0);
        }
        if (output.length != 0 && output.data.is_null()) || checked_len(output.length).is_err() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let Some(index) = get(index) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };
        let pattern = match unsafe { read_utf8_slice(pattern) } {
            Ok(pattern) => pattern,
            Err(status) => return status,
        };
        let entries = match source_filesystem::find_pack_index(index.index(), pattern) {
            Ok(entries) => entries,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let Some(first) = entries.first() else {
            return SOURCE_ABI_NOT_FOUND;
        };
        let status = copy_find_entry(first, output, out_written, out_is_directory);
        if status != SOURCE_ABI_OK {
            return status;
        }
        let opened = context
            .finds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .open(entries);
        let Some((find, _)) = opened else {
            return SOURCE_ABI_INTERNAL_ERROR;
        };
        unsafe {
            out_find.write(find);
        }
        SOURCE_ABI_OK
    })
}

fn publish(index: ArchiveIndex) -> Result<(u64, u32), SourceAbiStatus> {
    let count = index.index().entries().len() as u32;
    let handle = NEXT_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| SOURCE_ABI_INTERNAL_ERROR)?;
    indexes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(handle, Arc::new(index));
    Ok((handle, count))
}

/// Reads and validates a ZIP range in a regular file. Retains metadata and the
/// read-only file descriptor, not the temporary archive bytes. Payload reads
/// use that same descriptor and validate their size and CRC before publishing.
///
/// # Safety
/// Path is readable UTF-8; outputs point to distinct writable storage.
#[no_mangle]
pub unsafe extern "C" fn source_pak_index_open_file(
    path: SourceAbiSlice,
    offset: u64,
    length: u64,
    out_handle: *mut u64,
    out_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() || out_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_handle.write(0);
            out_count.write(0);
        }
        if length > source_pak::Limits::default().max_archive_size as u64 {
            return SOURCE_ABI_FORMAT_ERROR;
        }
        let path = match unsafe { read_utf8_slice(path) } {
            Ok(path) if !path.is_empty() && !path.contains('\0') => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let index = match ArchiveIndex::open_range(Path::new(path), offset, length) {
            Ok(index) => index,
            Err(error) => return archive_error_status(error),
        };
        let (handle, count) = match publish(index) {
            Ok(result) => result,
            Err(status) => return status,
        };
        unsafe {
            out_handle.write(handle);
            out_count.write(count);
        }
        SOURCE_ABI_OK
    })
}

/// Discover a standalone ZIP (kind 0) or a BSP's ZIP lump (kind 1) using one
/// retained descriptor. An empty BSP lump returns NOT_FOUND without an index.
/// Other BSP lumps are not consumed or validated by this archive operation.
///
/// # Safety
/// Path is readable UTF-8; outputs point to distinct writable storage.
#[no_mangle]
pub unsafe extern "C" fn source_pak_index_open_archive(
    path: SourceAbiSlice,
    kind: u32,
    out_handle: *mut u64,
    out_info: *mut SourceAbiPakArchiveInfo,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() || out_info.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_handle.write(0);
            out_info.write(SourceAbiPakArchiveInfo::default());
        }
        if kind > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let path = match unsafe { read_utf8_slice(path) } {
            Ok(path) if !path.contains('\0') => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let index = match ArchiveIndex::open(
            Path::new(path),
            if kind == 0 { Kind::Zip } else { Kind::Bsp },
        ) {
            Ok(index) => index,
            Err(error) => return archive_error_status(error),
        };
        let details = index.info();
        let (handle, entry_count) = match publish(index) {
            Ok(result) => result,
            Err(status) => return status,
        };
        let info = SourceAbiPakArchiveInfo {
            offset: details.offset,
            length: details.length,
            modified_seconds: details.modified_seconds,
            entry_count,
            reserved: 0,
        };
        unsafe {
            out_handle.write(handle);
            out_info.write(info);
        }
        SOURCE_ABI_OK
    })
}

/// Open and fully validate a file-backed pack entry as an independent,
/// read-only context file. Destroying the index does not invalidate this file.
/// Slice-created metadata-only indexes cannot open payloads.
///
/// # Safety
/// Outputs point to distinct writable u64 storage.
#[no_mangle]
pub unsafe extern "C" fn source_context_file_open_pak(
    context: u64,
    index: u64,
    at: u32,
    out_file: *mut u64,
    out_size: *mut u64,
    out_absolute_offset: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_file.is_null() || out_size.is_null() || out_absolute_offset.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_file.write(0);
            out_size.write(0);
            out_absolute_offset.write(0);
        }
        let context = match get_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let Some(index) = get(index) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };
        let (data, absolute_offset) = match index.read_entry(at as usize) {
            Ok(result) => result,
            Err(error) => return archive_error_status(error),
        };
        let (file, size) = context
            .files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .open_pack(data);
        unsafe {
            out_file.write(file);
            out_size.write(size);
            out_absolute_offset.write(absolute_offset);
        }
        SOURCE_ABI_OK
    })
}

/// Validates the entire directory and local headers before publishing an index.
/// Owns metadata only: neither input bytes nor native pointers are retained.
/// Payload decoding/CRC validation remains a separate read operation.
///
/// # Safety
/// Input is readable for its length, outputs point to distinct writable storage.
#[no_mangle]
pub unsafe extern "C" fn source_pak_index_create(
    input: SourceAbiSlice,
    out_handle: *mut u64,
    out_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() || out_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_handle.write(0);
            out_count.write(0);
        }
        // Same bound as the native adapter, checked before touching input bytes.
        if input.length > source_pak::Limits::default().max_archive_size as u64 {
            return SOURCE_ABI_FORMAT_ERROR;
        }
        let input = match unsafe { read_slice(input) } {
            Ok(bytes) => bytes,
            Err(status) => return status,
        };
        let index = match ArchiveIndex::metadata_only(input) {
            Ok(index) => index,
            Err(error) => return archive_error_status(error),
        };
        let (handle, count) = match publish(index) {
            Ok(result) => result,
            Err(status) => return status,
        };
        unsafe {
            out_handle.write(handle);
            out_count.write(count);
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_pak_index_destroy(handle: u64) -> SourceAbiStatus {
    ffi_status(|| {
        if indexes()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&handle)
            .is_some()
        {
            SOURCE_ABI_OK
        } else {
            SOURCE_ABI_INVALID_HANDLE
        }
    })
}

/// # Safety
/// Path must be readable and out_entry must point to writable entry storage.
#[no_mangle]
pub unsafe extern "C" fn source_pak_index_find(
    handle: u64,
    path: SourceAbiSlice,
    out_entry: *mut SourceAbiPakEntry,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_entry.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_entry.write(SourceAbiPakEntry::default());
        }
        let path = match unsafe { read_utf8_slice(path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let Some(index) = get(handle) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };
        let Some(at) = index.index().find(path) else {
            return SOURCE_ABI_NOT_FOUND;
        };
        unsafe {
            out_entry.write(SourceAbiPakEntry::new(&index.index().entries()[at], at));
        }
        SOURCE_ABI_OK
    })
}

/// Returns canonical name bytes without a NUL terminator. A size query or short
/// buffer returns BUFFER_TOO_SMALL and required length without copying bytes.
/// # Safety
/// Outputs are writable, nonoverlapping; name may be null only for zero capacity.
#[no_mangle]
pub unsafe extern "C" fn source_pak_index_entry(
    handle: u64,
    at: u32,
    out_entry: *mut SourceAbiPakEntry,
    name: *mut u8,
    capacity: u64,
    written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_entry.is_null()
            || written.is_null()
            || (capacity != 0 && name.is_null())
            || checked_len(capacity).is_err()
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_entry.write(SourceAbiPakEntry::default());
            written.write(0);
        }
        let Some(index) = get(handle) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };
        let Some(entry) = index.index().entries().get(at as usize) else {
            return SOURCE_ABI_NOT_FOUND;
        };
        unsafe {
            out_entry.write(SourceAbiPakEntry::new(entry, at as usize));
            write_bytes_out(entry.path.as_bytes(), name, capacity, written)
        }
    })
}
