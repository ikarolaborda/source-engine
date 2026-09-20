//! Versioned, allocation-neutral C ABI for transitional engine adapters.

use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

mod app_system;
mod compression;
mod d3d9;
mod mount_table;
mod pak_index;
mod presenter;
mod search_plan;
pub use presenter::*;

pub const SOURCE_ABI_VERSION: u32 = 1;

pub type SourceAbiStatus = i32;
pub const SOURCE_ABI_OK: SourceAbiStatus = 0;
pub const SOURCE_ABI_INVALID_ARGUMENT: SourceAbiStatus = 1;
pub const SOURCE_ABI_UNSUPPORTED_VERSION: SourceAbiStatus = 2;
pub const SOURCE_ABI_INVALID_HANDLE: SourceAbiStatus = 3;
pub const SOURCE_ABI_BUFFER_TOO_SMALL: SourceAbiStatus = 4;
pub const SOURCE_ABI_PANIC: SourceAbiStatus = 5;
pub const SOURCE_ABI_INTERNAL_ERROR: SourceAbiStatus = 6;
pub const SOURCE_ABI_IO_ERROR: SourceAbiStatus = 7;
pub const SOURCE_ABI_FORMAT_ERROR: SourceAbiStatus = 8;
pub const SOURCE_ABI_NOT_FOUND: SourceAbiStatus = 9;
/// The operation declined rather than failed: compressing a buffer that did
/// not get smaller, where the caller stores the original instead.
pub const SOURCE_ABI_DECLINED: SourceAbiStatus = 10;

pub const SOURCE_READ_PATH_DISK: u32 = 0;
pub const SOURCE_READ_PATH_VPK: u32 = 1;
/// From the archive embedded in the loaded map.
pub const SOURCE_READ_PATH_PAK: u32 = 2;

type SourceAbiHandle = u64;
type SourceAbiLogFn = unsafe extern "C" fn(*mut c_void, i32, SourceAbiSlice);
type SourceAbiSessionFn = unsafe extern "C" fn(*mut c_void) -> i32;
type SourceAbiFrameFn = unsafe extern "C" fn(*mut c_void) -> i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceAbiSlice {
    pub data: *const u8,
    pub length: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceAbiMutSlice {
    pub data: *mut u8,
    pub length: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceAbiContextConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub log: Option<SourceAbiLogFn>,
    pub user_data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiFramePlan {
    pub interpolation_numerator_ns: u64,
    pub interpolation_denominator_ns: u64,
    pub dropped_ns: u64,
    pub tick_count: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiFrameLoopInfo {
    pub iteration_count: u64,
    pub exit_reason: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiFramePace {
    pub elapsed_ns: u64,
    pub wait_ns: u64,
    pub ready: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiHostOperationInfo {
    pub target_length: u64,
    pub landmark_length: u64,
    pub kind: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiTickPlan {
    pub previous_remainder: f64,
    pub remainder: f64,
    pub next_tick: f64,
    pub tick_count: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiWorldInfo {
    pub plane_count: u64,
    pub node_count: u64,
    pub leaf_count: u64,
    pub cluster_count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiWorldLeaf {
    pub leaf_index: u64,
    pub contents: i32,
    pub cluster: i32,
    pub area: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiCvarInfo {
    pub generation: u64,
    pub flags: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiDemoInfo {
    pub command_count: u64,
    pub packet_count: u64,
    pub playback_ticks: i32,
    pub network_protocol: i32,
    pub string_table_section_count: u64,
    pub string_table_entry_count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiSaveInfo {
    pub embedded_file_count: u64,
    pub embedded_map_state_count: u64,
    pub token_count: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiNetPacketDecision {
    pub incoming_sequence: i32,
    pub outgoing_ack: i32,
    pub dropped: i32,
    pub accepted: u32,
    pub reason: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiNetSequenceAdvance {
    pub previous: i32,
    pub current: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiNetPacketHeader {
    pub sequence: i32,
    pub outgoing_ack: i32,
    pub flags: u32,
    pub reliable_state: u32,
    pub choked: u32,
    pub challenge: u32,
    pub header_bytes: u32,
    pub accepted: u32,
    pub reason: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiNetEncodedPacketHeader {
    pub bytes: [u8; 20],
    pub length: u32,
    pub flags_offset: u32,
    pub checksum_offset: u32,
    pub checksum_start: u32,
    pub base_flags: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiStringTableUpsert {
    pub index: u32,
    pub entry_count: u32,
    pub created: u32,
    pub user_data_changed: u32,
    pub tick_changed: i32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiStringTableChange {
    pub tick_changed: i32,
    pub changed: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceAbiDataTableProperty {
    pub name: SourceAbiSlice,
    pub reference_name: SourceAbiSlice,
    pub property_type: u32,
    pub flags: u32,
    pub bit_count: i32,
    pub elements: u32,
    pub low_value: f32,
    pub high_value: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiDataTableRegistration {
    pub table_id: u32,
    pub created: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiDataTableSummary {
    pub table_count: u32,
    pub property_count: u32,
    pub class_count: u32,
    pub compatibility_crc: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiSnapshotEntity {
    pub entity_index: u32,
    pub serial_number: i32,
    pub class_id: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiSnapshotSummary {
    pub tick: i32,
    pub max_entities: u32,
    pub valid_entity_count: u32,
    pub explicit_delete_count: u32,
}

/// Values of `SourceAbiSnapshotDelta::kind`.
pub const SOURCE_ABI_SNAPSHOT_DELTA_ENTER_PVS: u32 = 0;
pub const SOURCE_ABI_SNAPSHOT_DELTA_LEAVE_PVS: u32 = 1;
pub const SOURCE_ABI_SNAPSHOT_DELTA_CANDIDATE: u32 = 2;

/// Widest packet-entity header the encoder can produce, in bytes.
pub const SOURCE_ABI_MAX_DELTA_HEADER_BYTES: u64 =
    (source_net::MAX_DELTA_HEADER_BITS as u64).div_ceil(8);

// `SOURCE_MAX_DELTA_HEADER_BYTES` in source_abi.h has to match, and C cannot
// see the Rust constant, so pin the value here.
const _: () = assert!(SOURCE_ABI_MAX_DELTA_HEADER_BYTES == 5);

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SourceAbiSnapshotDelta {
    pub entity_index: u32,
    pub kind: u32,
    pub class_id: u32,
    pub serial_number: i32,
    pub recreated: u32,
    pub reserved: u32,
}

#[derive(Clone)]
struct Context {
    log: Option<SourceAbiLogFn>,
    user_data: usize,
    filesystem: Arc<Mutex<source_filesystem::SearchPaths>>,
    write_paths: Arc<Mutex<source_filesystem::WritePaths>>,
    files: Arc<Mutex<source_filesystem::OpenFiles>>,
    finds: Arc<Mutex<source_filesystem::OpenFinds>>,
    host: Arc<Mutex<source_host::Host>>,
    input: Arc<Mutex<source_input::State>>,
    network: Arc<Mutex<source_net::ChannelRegistry>>,
    splits: Arc<Mutex<source_net::SplitPacketRegistry>>,
    string_tables: Arc<Mutex<source_net::StringTableRegistry>>,
    data_tables: Arc<Mutex<source_net::DataTableRegistry>>,
    snapshots: Arc<Mutex<source_net::SnapshotRegistry>>,
    world: Arc<Mutex<Option<source_bsp::World>>>,
    console: Arc<Mutex<source_console::Console>>,
}

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static CONTEXTS: OnceLock<Mutex<HashMap<SourceAbiHandle, Context>>> = OnceLock::new();

fn contexts() -> &'static Mutex<HashMap<SourceAbiHandle, Context>> {
    CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_contexts() -> MutexGuard<'static, HashMap<SourceAbiHandle, Context>> {
    contexts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ffi_status(operation: impl FnOnce() -> SourceAbiStatus) -> SourceAbiStatus {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(SOURCE_ABI_PANIC)
}

fn checked_len(length: u64) -> Result<usize, SourceAbiStatus> {
    usize::try_from(length).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT)
}

unsafe fn read_slice<'a>(slice: SourceAbiSlice) -> Result<&'a [u8], SourceAbiStatus> {
    let length = checked_len(slice.length)?;
    if length == 0 {
        return Ok(&[]);
    }
    if slice.data.is_null() || length > isize::MAX as usize {
        return Err(SOURCE_ABI_INVALID_ARGUMENT);
    }
    // SAFETY: the caller's ABI contract guarantees that a non-null input range is readable
    // for `length` bytes during the call. Length and pointer preconditions are checked above.
    Ok(unsafe { std::slice::from_raw_parts(slice.data, length) })
}

unsafe fn write_slice<'a>(slice: SourceAbiMutSlice) -> Result<&'a mut [u8], SourceAbiStatus> {
    let length = checked_len(slice.length)?;
    if length == 0 {
        return Ok(&mut []);
    }
    if slice.data.is_null() || length > isize::MAX as usize {
        return Err(SOURCE_ABI_INVALID_ARGUMENT);
    }
    // SAFETY: the caller's ABI contract guarantees exclusive writable access to this range
    // during the call. Length and pointer preconditions are checked above.
    Ok(unsafe { std::slice::from_raw_parts_mut(slice.data, length) })
}

fn get_context(handle: SourceAbiHandle) -> Result<Context, SourceAbiStatus> {
    if handle == 0 {
        return Err(SOURCE_ABI_INVALID_HANDLE);
    }
    lock_contexts()
        .get(&handle)
        .cloned()
        .ok_or(SOURCE_ABI_INVALID_HANDLE)
}

#[no_mangle]
pub extern "C" fn source_abi_version() -> u32 {
    SOURCE_ABI_VERSION
}

#[no_mangle]
/// Creates a Rust context and transfers ownership of its opaque handle to the caller.
///
/// # Safety
/// `config` must point to a readable `SourceAbiContextConfig` for the duration of
/// the call and `out_handle` must point to writable `u64` storage. Any callback
/// supplied in the config must not unwind across the C ABI.
pub unsafe extern "C" fn source_context_create(
    config: *const SourceAbiContextConfig,
    out_handle: *mut SourceAbiHandle,
) -> SourceAbiStatus {
    ffi_status(|| {
        if config.is_null() || out_handle.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        // Write a sentinel before reading the caller-owned configuration so every failure
        // leaves the output in a deterministic state.
        unsafe { ptr::write(out_handle, 0) };
        // SAFETY: both pointers were checked and are required by the ABI to be valid for the call.
        let config = unsafe { ptr::read(config) };
        if config.struct_size < std::mem::size_of::<SourceAbiContextConfig>() as u32 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        if config.abi_version != SOURCE_ABI_VERSION {
            return SOURCE_ABI_UNSUPPORTED_VERSION;
        }

        let handle = loop {
            let candidate = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            if candidate != 0 && !lock_contexts().contains_key(&candidate) {
                break candidate;
            }
        };
        lock_contexts().insert(
            handle,
            Context {
                log: config.log,
                user_data: config.user_data as usize,
                filesystem: Arc::new(Mutex::new(source_filesystem::SearchPaths::new())),
                write_paths: Arc::new(Mutex::new(source_filesystem::WritePaths::new())),
                files: Arc::new(Mutex::new(source_filesystem::OpenFiles::new())),
                finds: Arc::new(Mutex::new(source_filesystem::OpenFinds::new())),
                host: Arc::new(Mutex::new(source_host::Host::new())),
                input: Arc::new(Mutex::new(source_input::State::new())),
                network: Arc::new(Mutex::new(source_net::ChannelRegistry::new())),
                splits: Arc::new(Mutex::new(source_net::SplitPacketRegistry::new())),
                string_tables: Arc::new(Mutex::new(source_net::StringTableRegistry::new())),
                data_tables: Arc::new(Mutex::new(source_net::DataTableRegistry::new())),
                snapshots: Arc::new(Mutex::new(source_net::SnapshotRegistry::new())),
                world: Arc::new(Mutex::new(None)),
                console: Arc::new(Mutex::new(source_console::Console::new())),
            },
        );
        unsafe { ptr::write(out_handle, handle) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_destroy(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        if handle == 0 || lock_contexts().remove(&handle).is_none() {
            SOURCE_ABI_INVALID_HANDLE
        } else {
            SOURCE_ABI_OK
        }
    })
}

#[no_mangle]
/// Copies a borrowed input slice into caller-owned output storage.
///
/// # Safety
/// Non-empty input and output slices must describe readable and exclusively
/// writable memory respectively for the duration of the call. `out_written`
/// must point to writable `u64` storage and the two slices must not overlap.
pub unsafe extern "C" fn source_context_echo(
    handle: SourceAbiHandle,
    input: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, input.length) };
        if get_context(handle).is_err() {
            return SOURCE_ABI_INVALID_HANDLE;
        }
        let input = match unsafe { read_slice(input) } {
            Ok(input) => input,
            Err(status) => return status,
        };
        if output.length < input.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..input.len()].copy_from_slice(input);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Invokes the context's logging callback with a borrowed message.
///
/// # Safety
/// A non-empty message must describe readable memory for the duration of the
/// call. The configured callback and user-data pointer must remain valid and
/// the callback must not unwind across the C ABI.
pub unsafe extern "C" fn source_context_emit_log(
    handle: SourceAbiHandle,
    level: i32,
    message: SourceAbiSlice,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if unsafe { read_slice(message) }.is_err() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        if let Some(log) = context.log {
            // SAFETY: the callback and user pointer originate from the caller's live config.
            // The ABI requires callbacks not to throw or unwind across this boundary.
            unsafe { log(context.user_data as *mut c_void, level, message) };
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Resolves a validated Source-style virtual path below a caller-provided root.
///
/// The returned UTF-8 bytes are not NUL-terminated. `out_written` receives the
/// required byte count even when `output` is too small.
///
/// # Safety
/// Non-empty root and virtual-path slices must describe readable UTF-8 bytes
/// for the duration of the call. A non-empty output slice must describe
/// exclusively writable memory and `out_written` must point to writable `u64`
/// storage.
pub unsafe extern "C" fn source_context_resolve_content_path(
    handle: SourceAbiHandle,
    root: SourceAbiSlice,
    virtual_path: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        if get_context(handle).is_err() {
            return SOURCE_ABI_INVALID_HANDLE;
        }
        let root = match unsafe { read_slice(root) } {
            Ok(root) if !root.is_empty() => root,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let root = match std::str::from_utf8(root) {
            Ok(root) => Path::new(root),
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let virtual_path = match unsafe { read_slice(virtual_path) } {
            Ok(path) if !path.is_empty() => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let virtual_path = match std::str::from_utf8(virtual_path) {
            Ok(path) => path,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let resolved = match source_filesystem::resolve_content_path_utf8(root, virtual_path) {
            Ok(path) => path,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe { ptr::write(out_written, resolved.len() as u64) };
        if output.length < resolved.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..resolved.len()].copy_from_slice(resolved.as_bytes());
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns the UTF-8 parent directory of the current executable.
///
/// The returned bytes are not NUL-terminated. `out_written` receives the
/// required byte count even when `output` is too small.
///
/// # Safety
/// A non-empty output slice must describe exclusively writable memory and
/// `out_written` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_executable_base(
    handle: SourceAbiHandle,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        if get_context(handle).is_err() {
            return SOURCE_ABI_INVALID_HANDLE;
        }
        let executable = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return SOURCE_ABI_IO_ERROR,
        };
        let parent = match executable.parent().and_then(Path::to_str) {
            Some(parent) if !parent.is_empty() => parent,
            _ => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe { ptr::write(out_written, parent.len() as u64) };
        if output.length < parent.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..parent.len()].copy_from_slice(parent.as_bytes());
        SOURCE_ABI_OK
    })
}

fn search_error_status(error: &source_filesystem::ReadError) -> SourceAbiStatus {
    match error {
        source_filesystem::ReadError::Io(_) => SOURCE_ABI_IO_ERROR,
        source_filesystem::ReadError::NotFound(_) => SOURCE_ABI_NOT_FOUND,
        source_filesystem::ReadError::Path(_)
        | source_filesystem::ReadError::Vpk(_)
        | source_filesystem::ReadError::Pak(_)
        | source_filesystem::ReadError::EscapedRoot(_) => SOURCE_ABI_FORMAT_ERROR,
    }
}

#[no_mangle]
/// Pure path-ID selection shared by Rust mounts and native search iterators.
/// No context is needed during early startup or cleanup. An explicit empty ID
/// differs from an unspecified ID; flags must be 0/1. Absent requests must have
/// zero length. Invalid input fails closed with a zero result.
///
/// # Safety
/// Slices describe readable UTF-8 bytes; out_matches points to writable storage.
pub unsafe extern "C" fn source_read_path_matches(
    stored: SourceAbiSlice,
    requested: SourceAbiSlice,
    has_requested: u8,
    by_request_only: u8,
    is_map_pack: u8,
    out_matches: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_matches.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_matches.write(0) };
        if has_requested > 1
            || by_request_only > 1
            || is_map_pack > 1
            || (has_requested == 0 && requested.length != 0)
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let stored = match unsafe { read_slice(stored) }
            .and_then(|bytes| std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
        {
            Ok(stored) => stored,
            Err(status) => return status,
        };
        let requested = if has_requested != 0 {
            match unsafe { read_slice(requested) }.and_then(|bytes| {
                std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT)
            }) {
                Ok(requested) => Some(requested),
                Err(status) => return status,
            }
        } else {
            None
        };
        let matches = source_filesystem::selection::path_id_matches(
            stored,
            requested,
            by_request_only != 0,
            is_map_pack != 0,
        );
        unsafe { out_matches.write(u32::from(matches)) };
        SOURCE_ABI_OK
    })
}

fn file_error_status(error: &source_filesystem::FileError) -> SourceAbiStatus {
    match error {
        source_filesystem::FileError::InvalidMode(_) => SOURCE_ABI_INVALID_ARGUMENT,
        source_filesystem::FileError::Missing(_) => SOURCE_ABI_NOT_FOUND,
        source_filesystem::FileError::Io(_) => SOURCE_ABI_IO_ERROR,
    }
}

fn network_error_status(error: source_net::ChannelError) -> SourceAbiStatus {
    match error {
        source_net::ChannelError::Missing(_) => SOURCE_ABI_NOT_FOUND,
        source_net::ChannelError::SequenceOverflow => SOURCE_ABI_INVALID_ARGUMENT,
    }
}

fn network_packet_decision(decision: source_net::PacketDecision) -> SourceAbiNetPacketDecision {
    SourceAbiNetPacketDecision {
        incoming_sequence: decision.incoming_sequence,
        outgoing_ack: decision.outgoing_ack,
        dropped: decision.dropped,
        accepted: u32::from(decision.accepted),
        reason: decision.reason,
        reserved: 0,
    }
}

fn network_packet_header(decision: source_net::PacketHeaderDecision) -> SourceAbiNetPacketHeader {
    SourceAbiNetPacketHeader {
        sequence: decision.sequence,
        outgoing_ack: decision.outgoing_ack,
        flags: u32::from(decision.flags),
        reliable_state: u32::from(decision.reliable_state),
        choked: u32::from(decision.choked),
        challenge: decision.challenge,
        header_bytes: u32::try_from(decision.header_bytes).unwrap_or_default(),
        accepted: u32::from(decision.accepted),
        reason: decision.reason,
        reserved: 0,
    }
}

fn encoded_network_packet_header(
    encoded: source_net::EncodedPacketHeader,
) -> SourceAbiNetEncodedPacketHeader {
    let mut bytes = [0u8; 20];
    bytes[..encoded.length].copy_from_slice(&encoded.bytes[..encoded.length]);
    SourceAbiNetEncodedPacketHeader {
        bytes,
        length: encoded.length as u32,
        flags_offset: encoded.flags_offset as u32,
        checksum_offset: encoded
            .checksum_offset
            .map_or(u32::MAX, |offset| offset as u32),
        checksum_start: encoded.checksum_start as u32,
        base_flags: u32::from(encoded.base_flags),
        reserved: 0,
    }
}

fn string_table_error_status(error: source_net::StringTableError) -> SourceAbiStatus {
    match error {
        source_net::StringTableError::MissingTable(_)
        | source_net::StringTableError::MissingEntry(_)
        | source_net::StringTableError::MissingString => SOURCE_ABI_NOT_FOUND,
        source_net::StringTableError::TableFull(_)
        | source_net::StringTableError::UserDataTooLarge(_) => SOURCE_ABI_BUFFER_TOO_SMALL,
        source_net::StringTableError::InvalidCapacity(_)
        | source_net::StringTableError::InvalidString
        | source_net::StringTableError::TickRegression { .. }
        | source_net::StringTableError::HistoryDisabled
        | source_net::StringTableError::NotEncodable(_) => SOURCE_ABI_INVALID_ARGUMENT,
        source_net::StringTableError::Truncated => SOURCE_ABI_FORMAT_ERROR,
    }
}

fn data_table_error_status(error: source_net::DataTableError) -> SourceAbiStatus {
    match error {
        source_net::DataTableError::MissingTable(_)
        | source_net::DataTableError::MissingClass
        | source_net::DataTableError::NotFinalized => SOURCE_ABI_NOT_FOUND,
        source_net::DataTableError::TooManyTables
        | source_net::DataTableError::TooManyProperties(_)
        | source_net::DataTableError::TooManyClasses => SOURCE_ABI_BUFFER_TOO_SMALL,
        source_net::DataTableError::Finalized
        | source_net::DataTableError::InvalidName
        | source_net::DataTableError::ConflictingTable(_)
        | source_net::DataTableError::DuplicateClass(_)
        | source_net::DataTableError::InvalidPropertyType(_)
        | source_net::DataTableError::InvalidPropertyFlags(_)
        | source_net::DataTableError::InvalidBitCount(_)
        | source_net::DataTableError::InvalidElementCount(_)
        | source_net::DataTableError::UnexpectedReference
        | source_net::DataTableError::MissingReference
        | source_net::DataTableError::PropertyCountMismatch { .. }
        | source_net::DataTableError::UnknownReferencedTable(_)
        | source_net::DataTableError::EmptyClasses
        | source_net::DataTableError::CompatibilityCrcMismatch { .. } => SOURCE_ABI_FORMAT_ERROR,
    }
}

fn data_table_summary(summary: source_net::DataTableSummary) -> SourceAbiDataTableSummary {
    SourceAbiDataTableSummary {
        table_count: summary.table_count,
        property_count: summary.property_count,
        class_count: summary.class_count,
        compatibility_crc: summary.compatibility_crc,
    }
}

fn snapshot_error_status(error: source_net::SnapshotError) -> SourceAbiStatus {
    match error {
        source_net::SnapshotError::Missing(_)
        | source_net::SnapshotError::MissingEntity(_)
        | source_net::SnapshotError::MissingDelete(_) => SOURCE_ABI_NOT_FOUND,
        source_net::SnapshotError::TooManyActive
        | source_net::SnapshotError::TooManyEntities(_) => SOURCE_ABI_BUFFER_TOO_SMALL,
        source_net::SnapshotError::InvalidMaxEntities(_)
        | source_net::SnapshotError::InvalidEntityIndex(_)
        | source_net::SnapshotError::InvalidSerialNumber(_)
        | source_net::SnapshotError::InvalidClassId(_)
        | source_net::SnapshotError::DuplicateEntity(_)
        | source_net::SnapshotError::EntityOrder { .. }
        | source_net::SnapshotError::InvalidDeleteSlot(_) => SOURCE_ABI_INVALID_ARGUMENT,
    }
}

fn snapshot_entity(entity: source_net::SnapshotEntity) -> SourceAbiSnapshotEntity {
    SourceAbiSnapshotEntity {
        entity_index: entity.entity_index,
        serial_number: entity.serial_number,
        class_id: entity.class_id,
        reserved: 0,
    }
}

/// Borrows an optional caller-owned entity-index array.
///
/// A null pointer means the caller opted out of narrowing, which is distinct
/// from an empty but present array.
///
/// # Safety
/// A non-null `indices` must address `count` readable `u32` values.
unsafe fn read_index_slice<'a>(
    indices: *const u32,
    count: u64,
) -> Result<Option<&'a [u32]>, SourceAbiStatus> {
    if indices.is_null() {
        return if count == 0 {
            Ok(None)
        } else {
            Err(SOURCE_ABI_INVALID_ARGUMENT)
        };
    }
    let count = usize::try_from(count).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT)?;
    if count == 0 {
        return Ok(Some(&[]));
    }
    Ok(Some(unsafe { slice::from_raw_parts(indices, count) }))
}

fn snapshot_delta(entry: source_net::SnapshotDeltaEntry) -> SourceAbiSnapshotDelta {
    SourceAbiSnapshotDelta {
        entity_index: entry.entity_index,
        kind: match entry.kind {
            source_net::SnapshotDeltaKind::EnterPvs => SOURCE_ABI_SNAPSHOT_DELTA_ENTER_PVS,
            source_net::SnapshotDeltaKind::LeavePvs => SOURCE_ABI_SNAPSHOT_DELTA_LEAVE_PVS,
            source_net::SnapshotDeltaKind::DeltaCandidate => SOURCE_ABI_SNAPSHOT_DELTA_CANDIDATE,
        },
        class_id: entry.class_id,
        serial_number: entry.serial_number,
        recreated: u32::from(entry.recreated),
        reserved: 0,
    }
}

fn snapshot_summary(summary: source_net::SnapshotSummary) -> SourceAbiSnapshotSummary {
    SourceAbiSnapshotSummary {
        tick: summary.tick,
        max_entities: summary.max_entities,
        valid_entity_count: summary.valid_entity_count,
        explicit_delete_count: summary.explicit_delete_count,
    }
}

/// The content mounts a context has built, shared rather than copied so
/// that a renderer reads a map through the same ordered search path the
/// rest of the engine does, including any archive mounted after it started.
fn context_filesystem(
    handle: SourceAbiHandle,
) -> Option<Arc<Mutex<source_filesystem::SearchPaths>>> {
    lock_contexts()
        .get(&handle)
        .map(|context| Arc::clone(&context.filesystem))
}

unsafe fn read_utf8_slice<'a>(slice: SourceAbiSlice) -> Result<&'a str, SourceAbiStatus> {
    let bytes = unsafe { read_slice(slice) }?;
    if bytes.is_empty() {
        return Err(SOURCE_ABI_INVALID_ARGUMENT);
    }
    std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT)
}

unsafe fn read_optional_utf8_slice<'a>(
    slice: SourceAbiSlice,
) -> Result<Option<&'a str>, SourceAbiStatus> {
    let bytes = unsafe { read_slice(slice) }?;
    if bytes.is_empty() {
        return Ok(None);
    }
    std::str::from_utf8(bytes)
        .map(Some)
        .map_err(|_| SOURCE_ABI_INVALID_ARGUMENT)
}

#[no_mangle]
/// Replaces startup GAME read mounts using the selected game's gameinfo.txt.
/// The replacement is atomic: a malformed declaration leaves existing mounts
/// intact. Native filesystem initialization later supplies its complete registry.
///
/// # Safety
/// Input slices must contain readable UTF-8 for this call. `external_root` may
/// be empty. `out_mount_count` must point to writable u64 storage.
pub unsafe extern "C" fn source_context_mount_gameinfo(
    handle: SourceAbiHandle,
    base: SourceAbiSlice,
    game: SourceAbiSlice,
    external_root: SourceAbiSlice,
    out_mount_count: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_mount_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { *out_mount_count = 0 };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let inputs = unsafe {
            (
                read_utf8_slice(base),
                read_utf8_slice(game),
                read_optional_utf8_slice(external_root),
            )
        };
        let (base, game, external) = match inputs {
            (Ok(base), Ok(game), Ok(external)) => (base, game, external),
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let mounts = match source_filesystem::gameinfo::load(
            std::path::Path::new(base),
            std::path::Path::new(game),
            external.map(std::path::Path::new),
        ) {
            Ok(mounts) => mounts,
            Err(error) => {
                let message = format!("Rust gameinfo mount failed ({game}): {error}");
                if let Some(log) = context.log {
                    let message = SourceAbiSlice {
                        data: message.as_ptr(),
                        length: message.len() as u64,
                    };
                    unsafe { log(context.user_data as *mut c_void, 2, message) };
                }
                return match error {
                    source_filesystem::gameinfo::Error::Io(error)
                        if error.kind() == std::io::ErrorKind::NotFound =>
                    {
                        SOURCE_ABI_NOT_FOUND
                    }
                    source_filesystem::gameinfo::Error::Io(_) => SOURCE_ABI_IO_ERROR,
                    source_filesystem::gameinfo::Error::Mount(error) => search_error_status(&error),
                    _ => SOURCE_ABI_FORMAT_ERROR,
                };
            }
        };
        unsafe { *out_mount_count = mounts.len() as u64 };
        *context
            .filesystem
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = mounts;
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Adds a validated loose directory to the context's ordered content mounts.
///
/// # Safety
/// Root and path-ID slices must describe readable UTF-8 bytes for the duration
/// of the call. `at_head` must be zero for tail precedence or one for head.
pub unsafe extern "C" fn source_context_mount_directory(
    handle: SourceAbiHandle,
    root: SourceAbiSlice,
    path_id: SourceAbiSlice,
    at_head: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let root = match unsafe { read_utf8_slice(root) } {
            Ok(root) => root,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let position = match at_head {
            0 => source_filesystem::Position::Tail,
            1 => source_filesystem::Position::Head,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let mut filesystem = context
            .filesystem
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match filesystem.mount_directory(root, path_id, position) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Adds a validated VPK directory file to the context's ordered content mounts.
///
/// # Safety
/// VPK path and path-ID slices must describe readable UTF-8 bytes for the
/// duration of the call. `at_head` must be zero for tail precedence or one for
/// head.
pub unsafe extern "C" fn source_context_mount_vpk(
    handle: SourceAbiHandle,
    directory_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    at_head: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let directory_path = match unsafe { read_utf8_slice(directory_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let position = match at_head {
            0 => source_filesystem::Position::Tail,
            1 => source_filesystem::Position::Head,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let mut filesystem = context
            .filesystem
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match filesystem.mount_vpk(directory_path, path_id, position) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Clears the active ordered read mounts while retaining validated archive indexes
/// for inexpensive native search-path resynchronization.
pub extern "C" fn source_context_read_paths_clear(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        context
            .filesystem
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Adds a native loose-directory search entry with its visibility flags.
///
/// # Safety
/// Root and path-ID slices must describe readable UTF-8 bytes for the duration
/// of the call. Boolean bytes and `at_head` must each be zero or one.
pub unsafe extern "C" fn source_context_read_path_add_directory_flags(
    handle: SourceAbiHandle,
    root: SourceAbiSlice,
    path_id: SourceAbiSlice,
    at_head: u8,
    by_request_only: u8,
    allow_symlink_escape: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let root = match unsafe { read_utf8_slice(root) } {
            Ok(root) => root,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let position = match at_head {
            0 => source_filesystem::Position::Tail,
            1 => source_filesystem::Position::Head,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let by_request_only = match by_request_only {
            0 => false,
            1 => true,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let allow_symlink_escape = match allow_symlink_escape {
            0 => false,
            1 => true,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let mut filesystem = context
            .filesystem
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match filesystem.mount_directory_with_flags(
            root,
            path_id,
            position,
            by_request_only,
            allow_symlink_escape,
        ) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Adds a native VPK search entry with its visibility flags.
///
/// # Safety
/// VPK and path-ID slices must describe readable UTF-8 bytes for the duration
/// of the call. Boolean bytes and `at_head` must each be zero or one.
pub unsafe extern "C" fn source_context_read_path_add_vpk_flags(
    handle: SourceAbiHandle,
    directory_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    at_head: u8,
    by_request_only: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let directory_path = match unsafe { read_utf8_slice(directory_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let position = match at_head {
            0 => source_filesystem::Position::Tail,
            1 => source_filesystem::Position::Head,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let by_request_only = match by_request_only {
            0 => false,
            1 => true,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let mut filesystem = context
            .filesystem
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match filesystem.mount_vpk_with_flags(directory_path, path_id, position, by_request_only) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Adds a ZIP or BSP ZIP range with the native search entry's visibility flags.
/// Rust opens, validates and owns the archive and entry payloads.
///
/// # Safety
/// Path slices must describe readable UTF-8 bytes for the duration of the call.
/// Boolean bytes must be zero or one.
pub unsafe extern "C" fn source_context_read_path_add_pak_flags(
    handle: SourceAbiHandle,
    archive_path: SourceAbiSlice,
    offset: u64,
    length: u64,
    path_id: SourceAbiSlice,
    at_head: u8,
    by_request_only: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let archive_path = match unsafe { read_utf8_slice(archive_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let position = match at_head {
            0 => source_filesystem::Position::Tail,
            1 => source_filesystem::Position::Head,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        if by_request_only > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let mut filesystem = context
            .filesystem
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match filesystem.mount_pak_file_with_flags(
            archive_path,
            offset,
            length,
            path_id,
            position,
            by_request_only != 0,
        ) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Clears the ordered loose-directory registry used for write resolution.
pub extern "C" fn source_context_write_paths_clear(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Adds a validated loose directory to the ordered write-path registry.
///
/// # Safety
/// Root and path-ID slices must describe readable UTF-8 bytes for the duration
/// of the call. `at_head` must be zero for tail precedence or one for head.
pub unsafe extern "C" fn source_context_write_path_add(
    handle: SourceAbiHandle,
    root: SourceAbiSlice,
    path_id: SourceAbiSlice,
    at_head: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let root = match unsafe { read_utf8_slice(root) } {
            Ok(root) => root,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let position = match at_head {
            0 => source_filesystem::Position::Tail,
            1 => source_filesystem::Position::Head,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let mut write_paths = context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match write_paths.mount_directory(root, path_id, position) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Adds a validated loose directory plus its native search visibility to the
/// ordered write-path registry.
///
/// # Safety
/// Root and path-ID slices must describe readable UTF-8 bytes for the duration
/// of the call. Boolean bytes and `at_head` must each be zero or one.
pub unsafe extern "C" fn source_context_write_path_add_flags(
    handle: SourceAbiHandle,
    root: SourceAbiSlice,
    path_id: SourceAbiSlice,
    at_head: u8,
    by_request_only: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let root = match unsafe { read_utf8_slice(root) } {
            Ok(root) => root,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let position = match at_head {
            0 => source_filesystem::Position::Tail,
            1 => source_filesystem::Position::Head,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let by_request_only = match by_request_only {
            0 => false,
            1 => true,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let mut write_paths = context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match write_paths.mount_directory_with_request_only(
            root,
            path_id,
            position,
            by_request_only,
        ) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Resolves a relative write target through Source-compatible write-path
/// aliases and fallbacks. Returned UTF-8 bytes are not NUL-terminated.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. A non-empty output slice must describe exclusively writable memory,
/// and `out_written` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_resolve_write_path(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let resolved = {
            let write_paths = context
                .write_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match write_paths.resolve(virtual_path, path_id) {
                Ok(path) => path,
                Err(error) => return search_error_status(&error),
            }
        };
        let resolved = match resolved.to_str() {
            Some(resolved) => resolved,
            None => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe { ptr::write(out_written, resolved.len() as u64) };
        if output.length < resolved.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..resolved.len()].copy_from_slice(resolved.as_bytes());
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Creates a relative directory hierarchy below the selected write root.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8.
pub unsafe extern "C" fn source_context_create_write_directory(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let result = context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .create_directory_all(virtual_path, path_id);
        match result {
            Ok(_) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Removes a relative file below the selected write root.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8.
pub unsafe extern "C" fn source_context_remove_write_file(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let result = context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove_file(virtual_path, path_id);
        match result {
            Ok(_) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Renames an existing relative loose file to a relative write target.
///
/// # Safety
/// Path and non-empty path-ID slices must describe readable UTF-8.
pub unsafe extern "C" fn source_context_rename_write_file(
    handle: SourceAbiHandle,
    old_virtual_path: SourceAbiSlice,
    old_path_id: SourceAbiSlice,
    new_virtual_path: SourceAbiSlice,
    new_path_id: SourceAbiSlice,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let old_virtual_path = match unsafe { read_utf8_slice(old_virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let old_path_id = match unsafe { read_optional_utf8_slice(old_path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let new_virtual_path = match unsafe { read_utf8_slice(new_virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let new_path_id = match unsafe { read_optional_utf8_slice(new_path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let result = context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .rename_file(old_virtual_path, old_path_id, new_virtual_path, new_path_id);
        match result {
            Ok(_) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Reports the owner-write permission of the first matching relative loose file.
///
/// # Safety
/// Path and non-empty path-ID slices must describe readable UTF-8, and
/// `out_writable` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_is_write_file_writable(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_writable: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_writable.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_writable, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let result = context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_file_writable(virtual_path, path_id);
        match result {
            Ok(writable) => {
                unsafe { ptr::write(out_writable, u32::from(writable)) };
                SOURCE_ABI_OK
            }
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Replaces the permissions of the first matching relative loose file with
/// owner read/write or owner read-only permissions.
///
/// # Safety
/// Path and non-empty path-ID slices must describe readable UTF-8, and
/// `writable` must be zero or one.
pub unsafe extern "C" fn source_context_set_write_file_writable(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    writable: u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        let writable = match writable {
            0 => false,
            1 => true,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let result = context
            .write_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_file_writable(virtual_path, path_id, writable);
        match result {
            Ok(_) => SOURCE_ABI_OK,
            Err(error) => search_error_status(&error),
        }
    })
}

#[no_mangle]
/// Opens a relative file from the ordered Rust read mounts as a context-owned
/// read-only handle.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. Both output pointers must point to writable storage.
pub unsafe extern "C" fn source_context_file_open_read(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_file: *mut u64,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_file.is_null() || out_size.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_file, 0);
            ptr::write(out_size, 0);
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let source = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.open_read_source(virtual_path, path_id) {
                Ok(source) => source,
                Err(error) => return search_error_status(&error),
            }
        };
        let opened = {
            let mut files = context
                .files
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match source {
                source_filesystem::ReadSource::Disk(path) => files.open(&path, "rb"),
                source_filesystem::ReadSource::Memory(data) => Ok(files.open_memory(data)),
            }
        };
        let (file, size) = match opened {
            Ok(opened) => opened,
            Err(error) => return file_error_status(&error),
        };
        unsafe {
            ptr::write(out_file, file);
            ptr::write(out_size, size);
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Opens a relative file through the Rust-owned write-path and handle registries.
///
/// # Safety
/// Virtual-path, non-empty path-ID, and mode slices must describe readable
/// UTF-8 bytes. Both output pointers must point to writable storage.
pub unsafe extern "C" fn source_context_file_open_write(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    mode: SourceAbiSlice,
    out_file: *mut u64,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_file.is_null() || out_size.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_file, 0);
            ptr::write(out_size, 0);
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let mode = match unsafe { read_utf8_slice(mode) } {
            Ok(mode) => mode,
            Err(status) => return status,
        };
        let resolved = {
            let write_paths = context
                .write_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match write_paths.resolve(virtual_path, path_id) {
                Ok(path) => path,
                Err(error) => return search_error_status(&error),
            }
        };
        let opened = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .open(&resolved, mode);
        match opened {
            Ok((file, size)) => {
                unsafe {
                    ptr::write(out_file, file);
                    ptr::write(out_size, size);
                }
                SOURCE_ABI_OK
            }
            Err(error) => file_error_status(&error),
        }
    })
}

#[no_mangle]
pub extern "C" fn source_context_file_close(handle: SourceAbiHandle, file: u64) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        if context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .close(file)
        {
            SOURCE_ABI_OK
        } else {
            SOURCE_ABI_NOT_FOUND
        }
    })
}

#[no_mangle]
/// Reads from a Rust-owned file handle into caller-owned storage.
///
/// # Safety
/// A non-empty output slice must describe exclusively writable memory, and
/// `out_read` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_file_read(
    handle: SourceAbiHandle,
    file: u64,
    output: SourceAbiMutSlice,
    out_read: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_read.is_null() || file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_read, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        let result = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .read(file, output);
        match result {
            Ok(read) => {
                unsafe { ptr::write(out_read, read as u64) };
                SOURCE_ABI_OK
            }
            Err(error) => file_error_status(&error),
        }
    })
}

#[no_mangle]
/// Writes caller-owned bytes to a Rust-owned file handle.
///
/// # Safety
/// A non-empty input slice must describe readable memory, and `out_written`
/// must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_file_write(
    handle: SourceAbiHandle,
    file: u64,
    input: SourceAbiSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() || file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let input = match unsafe { read_slice(input) } {
            Ok(input) => input,
            Err(status) => return status,
        };
        let result = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write(file, input);
        match result {
            Ok(written) => {
                unsafe { ptr::write(out_written, written as u64) };
                SOURCE_ABI_OK
            }
            Err(error) => file_error_status(&error),
        }
    })
}

#[no_mangle]
pub extern "C" fn source_context_file_flush(handle: SourceAbiHandle, file: u64) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let result = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .flush(file);
        match result {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => file_error_status(&error),
        }
    })
}

#[no_mangle]
/// Seeks a Rust-owned file. Origins are 0=start, 1=current, and 2=end.
///
/// # Safety
/// `out_position` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_file_seek(
    handle: SourceAbiHandle,
    file: u64,
    offset: i64,
    origin: u32,
    out_position: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_position.is_null() || file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_position, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let result = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .seek(file, offset, origin);
        match result {
            Ok(position) => {
                unsafe { ptr::write(out_position, position) };
                SOURCE_ABI_OK
            }
            Err(error) => file_error_status(&error),
        }
    })
}

#[no_mangle]
/// Returns the current cursor of a Rust-owned file.
///
/// # Safety
/// `out_position` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_file_tell(
    handle: SourceAbiHandle,
    file: u64,
    out_position: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_position.is_null() || file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_position, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let result = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .tell(file);
        match result {
            Ok(position) => {
                unsafe { ptr::write(out_position, position) };
                SOURCE_ABI_OK
            }
            Err(error) => file_error_status(&error),
        }
    })
}

#[no_mangle]
/// Returns the current length of a Rust-owned file.
///
/// # Safety
/// `out_size` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_open_file_size(
    handle: SourceAbiHandle,
    file: u64,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_size.is_null() || file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_size, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let result = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .size(file);
        match result {
            Ok(size) => {
                unsafe { ptr::write(out_size, size) };
                SOURCE_ABI_OK
            }
            Err(error) => file_error_status(&error),
        }
    })
}

#[no_mangle]
/// Reports whether an opaque file ID is currently owned by this context.
///
/// # Safety
/// `out_open` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_file_is_open(
    handle: SourceAbiHandle,
    file: u64,
    out_open: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_open.is_null() || file == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_open, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let open = context
            .files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(file);
        unsafe { ptr::write(out_open, u32::from(open)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Reads a file through the context's ordered loose/VPK search paths.
///
/// An empty path-ID selects all mounts. `out_written` receives the required
/// byte count even when the output is too small.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. A non-empty output slice must describe exclusively writable memory,
/// and `out_written` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_read_file(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let contents = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.read(virtual_path, path_id) {
                Ok(contents) => contents,
                Err(error) => return search_error_status(&error),
            }
        };
        unsafe { ptr::write(out_written, contents.len() as u64) };
        if output.length < contents.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..contents.len()].copy_from_slice(&contents);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns a mounted file's size without reading its payload.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes, and `out_size` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_file_size(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_size.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_size, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let size = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.file_size(virtual_path, path_id) {
                Ok(size) => size,
                Err(error) => return search_error_status(&error),
            }
        };
        unsafe { ptr::write(out_size, size) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Resolves a mounted relative path to a canonical loose path or Source's
/// encoded `<base>.vpk/<virtual-path>` packed-path spelling.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. A non-empty output slice must describe exclusively writable memory,
/// and both output pointers must point to writable storage.
pub unsafe extern "C" fn source_context_resolve_read_path(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    out_kind: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() || out_kind.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_written, 0);
            ptr::write(out_kind, SOURCE_READ_PATH_DISK);
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let resolved = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.resolve_read_path(virtual_path, path_id) {
                Ok(resolved) => resolved,
                Err(error) => return search_error_status(&error),
            }
        };
        let resolved_path = match resolved.path.to_str() {
            Some(path) => path,
            None => return SOURCE_ABI_FORMAT_ERROR,
        };
        let kind = match resolved.kind {
            source_filesystem::ReadPathKind::Disk => SOURCE_READ_PATH_DISK,
            source_filesystem::ReadPathKind::Vpk => SOURCE_READ_PATH_VPK,
            source_filesystem::ReadPathKind::Pak => SOURCE_READ_PATH_PAK,
        };
        unsafe {
            ptr::write(out_written, resolved_path.len() as u64);
            ptr::write(out_kind, kind);
        }
        if output.length < resolved_path.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..resolved_path.len()].copy_from_slice(resolved_path.as_bytes());
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Reports whether a mounted relative path names a loose or VPK directory.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes, and `out_is_directory` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_path_is_directory(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_is_directory: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_is_directory.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_is_directory, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let is_directory = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.is_directory(virtual_path, path_id) {
                Ok(is_directory) => is_directory,
                Err(error) => return search_error_status(&error),
            }
        };
        unsafe { ptr::write(out_is_directory, u32::from(is_directory)) };
        SOURCE_ABI_OK
    })
}

fn copy_find_entry(
    entry: &source_filesystem::FindEntry,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    out_is_directory: *mut u32,
) -> SourceAbiStatus {
    unsafe {
        ptr::write(out_written, entry.name.len() as u64);
        ptr::write(out_is_directory, u32::from(entry.is_directory));
    }
    if output.length < entry.name.len() as u64 {
        return SOURCE_ABI_BUFFER_TOO_SMALL;
    }
    let output = match unsafe { write_slice(output) } {
        Ok(output) => output,
        Err(status) => return status,
    };
    output[..entry.name.len()].copy_from_slice(entry.name.as_bytes());
    SOURCE_ABI_OK
}

#[no_mangle]
/// Opens a context-owned cursor over selected loose/VPK/ZIP/BSP wildcard matches.
/// The first unqualified entry name is copied to caller-owned storage.
///
/// # Safety
/// Wildcard and non-empty path-ID slices must describe readable UTF-8 bytes.
/// Output pointers must point to writable storage and a non-empty output slice
/// must describe exclusively writable memory.
pub unsafe extern "C" fn source_context_find_first(
    handle: SourceAbiHandle,
    wildcard: SourceAbiSlice,
    path_id: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    out_is_directory: *mut u32,
    out_find: *mut u64,
) -> SourceAbiStatus {
    unsafe {
        source_context_find_first_bounded(
            handle,
            wildcard,
            path_id,
            u32::MAX,
            output,
            out_written,
            out_is_directory,
            out_find,
        )
    }
}

#[no_mangle]
/// Opens a cursor with names too long for a fixed-width native field omitted.
/// The limit excludes a trailing NUL. This is a query limit, independent of the
/// output buffer: short outputs still publish no cursor and never truncate.
///
/// # Safety
/// Same slice and output contracts as source_context_find_first.
pub unsafe extern "C" fn source_context_find_first_bounded(
    handle: SourceAbiHandle,
    wildcard: SourceAbiSlice,
    path_id: SourceAbiSlice,
    max_name_bytes: u32,
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
            ptr::write(out_written, 0);
            ptr::write(out_is_directory, 0);
            ptr::write(out_find, 0);
        }
        if max_name_bytes == 0
            || (output.length != 0 && output.data.is_null())
            || checked_len(output.length).is_err()
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let wildcard = match unsafe { read_utf8_slice(wildcard) } {
            Ok(wildcard) => wildcard,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let mut entries = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.find(wildcard, path_id) {
                Ok(entries) => entries,
                Err(error) => return search_error_status(&error),
            }
        };
        entries.retain(|entry| entry.name.len() <= max_name_bytes as usize);
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
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .open(entries);
        let Some((find, _)) = opened else {
            return SOURCE_ABI_NOT_FOUND;
        };
        unsafe { ptr::write(out_find, find) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Copies the next entry from a context-owned wildcard cursor.
///
/// # Safety
/// Output pointers must point to writable storage and a non-empty output slice
/// must describe exclusively writable memory.
pub unsafe extern "C" fn source_context_find_next(
    handle: SourceAbiHandle,
    find: u64,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    out_is_directory: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if find == 0 || out_written.is_null() || out_is_directory.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_written, 0);
            ptr::write(out_is_directory, 0);
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let mut finds = context
            .finds
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = match finds.peek(find) {
            Ok(Some(entry)) => entry.clone(),
            Ok(None) | Err(_) => return SOURCE_ABI_NOT_FOUND,
        };
        let status = copy_find_entry(&entry, output, out_written, out_is_directory);
        if status != SOURCE_ABI_OK {
            return status;
        }
        if finds.advance(find).is_err() {
            return SOURCE_ABI_NOT_FOUND;
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_find_close(handle: SourceAbiHandle, find: u64) -> SourceAbiStatus {
    ffi_status(|| {
        if find == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if context
            .finds
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .close(find)
        {
            SOURCE_ABI_OK
        } else {
            SOURCE_ABI_NOT_FOUND
        }
    })
}

#[no_mangle]
/// Registers context-owned sequence state for one live network channel.
///
/// # Safety
/// `out_channel_id` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_net_channel_create(
    handle: SourceAbiHandle,
    outgoing_sequence: i32,
    incoming_sequence: i32,
    outgoing_ack: i32,
    out_channel_id: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_channel_id.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_channel_id, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let channel_id = context
            .network
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .create(outgoing_sequence, incoming_sequence, outgoing_ack);
        unsafe { ptr::write(out_channel_id, channel_id) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_net_channel_remove(
    handle: SourceAbiHandle,
    channel_id: u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if channel_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if context
            .network
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(channel_id)
        {
            SOURCE_ABI_OK
        } else {
            SOURCE_ABI_NOT_FOUND
        }
    })
}

#[no_mangle]
pub extern "C" fn source_context_net_channel_reset(
    handle: SourceAbiHandle,
    channel_id: u64,
    outgoing_sequence: i32,
    incoming_sequence: i32,
    outgoing_ack: i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if channel_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let status = match context
            .network
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset(
                channel_id,
                outgoing_sequence,
                incoming_sequence,
                outgoing_ack,
            ) {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => network_error_status(error),
        };
        status
    })
}

#[no_mangle]
/// Computes duplicate, ordering, and packet-loss state without mutating the channel.
///
/// # Safety
/// `out_decision` must point to writable `SourceAbiNetPacketDecision` storage.
pub unsafe extern "C" fn source_context_net_channel_preview_incoming(
    handle: SourceAbiHandle,
    channel_id: u64,
    sequence: i32,
    outgoing_ack: i32,
    choked: u32,
    max_drop: i32,
    out_decision: *mut SourceAbiNetPacketDecision,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_decision.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_decision, SourceAbiNetPacketDecision::default()) };
        if channel_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let decision = match context
            .network
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .preview_incoming(channel_id, sequence, outgoing_ack, choked, max_drop)
        {
            Ok(decision) => decision,
            Err(error) => return network_error_status(error),
        };
        unsafe { ptr::write(out_decision, network_packet_decision(decision)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Commits an accepted incoming sequence after native reliable-fragment validation.
///
/// # Safety
/// `out_decision` must point to writable `SourceAbiNetPacketDecision` storage.
pub unsafe extern "C" fn source_context_net_channel_commit_incoming(
    handle: SourceAbiHandle,
    channel_id: u64,
    sequence: i32,
    outgoing_ack: i32,
    choked: u32,
    max_drop: i32,
    out_decision: *mut SourceAbiNetPacketDecision,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_decision.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_decision, SourceAbiNetPacketDecision::default()) };
        if channel_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let decision = match context
            .network
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .commit_incoming(channel_id, sequence, outgoing_ack, choked, max_drop)
        {
            Ok(decision) => decision,
            Err(error) => return network_error_status(error),
        };
        unsafe { ptr::write(out_decision, network_packet_decision(decision)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Advances the outgoing sequence with the same wrapping semantics as the native channel.
///
/// # Safety
/// `out_advance` must point to writable `SourceAbiNetSequenceAdvance` storage.
pub unsafe extern "C" fn source_context_net_channel_advance_outgoing(
    handle: SourceAbiHandle,
    channel_id: u64,
    out_advance: *mut SourceAbiNetSequenceAdvance,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_advance.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_advance, SourceAbiNetSequenceAdvance::default()) };
        if channel_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let advance = match context
            .network
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance_outgoing(channel_id)
        {
            Ok(advance) => advance,
            Err(error) => return network_error_status(error),
        };
        unsafe {
            ptr::write(
                out_advance,
                SourceAbiNetSequenceAdvance {
                    previous: advance.previous,
                    current: advance.current,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Encodes the fixed net-channel wire header before native payload assembly.
///
/// # Safety
/// `out_header` must point to writable `SourceAbiNetEncodedPacketHeader` storage.
pub unsafe extern "C" fn source_context_net_packet_encode_header(
    handle: SourceAbiHandle,
    sequence: i32,
    outgoing_ack: i32,
    reliable_state: u32,
    has_choked: u32,
    choked: u32,
    has_challenge: u32,
    challenge: u32,
    checksum_required: u32,
    out_header: *mut SourceAbiNetEncodedPacketHeader,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_header.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_header, SourceAbiNetEncodedPacketHeader::default()) };
        if reliable_state > u32::from(u8::MAX)
            || choked > u32::from(u8::MAX)
            || has_choked > 1
            || has_challenge > 1
            || checksum_required > 1
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        if let Err(status) = get_context(handle) {
            return status;
        }
        let encoded = source_net::encode_packet_header(
            sequence,
            outgoing_ack,
            reliable_state as u8,
            (has_choked != 0).then_some(choked as u8),
            (has_challenge != 0).then_some(challenge),
            checksum_required != 0,
        );
        unsafe { ptr::write(out_header, encoded_network_packet_header(encoded)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Computes the folded Source packet CRC over caller-owned bytes.
///
/// # Safety
/// `payload` must describe readable bytes and `out_checksum` must point to
/// writable `u16` storage.
pub unsafe extern "C" fn source_context_net_packet_checksum(
    handle: SourceAbiHandle,
    payload: SourceAbiSlice,
    out_checksum: *mut u16,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_checksum.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_checksum, 0) };
        if let Err(status) = get_context(handle) {
            return status;
        }
        let payload = match unsafe { read_slice(payload) } {
            Ok(payload) => payload,
            Err(status) => return status,
        };
        unsafe { ptr::write(out_checksum, source_net::short_packet_checksum(payload)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Finalizes the flags and optional checksum of a caller-owned packet.
///
/// # Safety
/// `packet` must describe exclusively writable bytes for the duration of this
/// call and `out_checksum` must point to writable `u16` storage.
pub unsafe extern "C" fn source_context_net_packet_finalize_header(
    handle: SourceAbiHandle,
    packet: SourceAbiMutSlice,
    flags: u32,
    checksum_required: u32,
    out_checksum: *mut u16,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_checksum.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_checksum, 0) };
        if flags > u32::from(u8::MAX) || checksum_required > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        if let Err(status) = get_context(handle) {
            return status;
        }
        let packet = match unsafe { write_slice(packet) } {
            Ok(packet) => packet,
            Err(status) => return status,
        };
        let checksum =
            match source_net::finalize_packet_header(packet, flags as u8, checksum_required != 0) {
                Ok(checksum) => checksum,
                Err(_) => return SOURCE_ABI_FORMAT_ERROR,
            };
        unsafe { ptr::write(out_checksum, checksum) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Parses and validates a complete Source net-channel packet header.
///
/// # Safety
/// `packet` must describe readable bytes and `out_header` must point to writable
/// `SourceAbiNetPacketHeader` storage.
pub unsafe extern "C" fn source_context_net_packet_header(
    handle: SourceAbiHandle,
    packet: SourceAbiSlice,
    checksum_required: u32,
    expects_challenge: u32,
    expected_challenge: u32,
    out_header: *mut SourceAbiNetPacketHeader,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_header.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_header, SourceAbiNetPacketHeader::default()) };
        if checksum_required > 1 || expects_challenge > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        if let Err(status) = get_context(handle) {
            return status;
        }
        let packet = match unsafe { read_slice(packet) } {
            Ok(packet) => packet,
            Err(status) => return status,
        };
        let decision = source_net::parse_packet_header(
            packet,
            checksum_required != 0,
            expects_challenge != 0,
            expected_challenge,
        );
        unsafe { ptr::write(out_header, network_packet_header(decision)) };
        SOURCE_ABI_OK
    })
}

/// Compresses `input` as LZSS, the codec the engine's saves and compressed
/// buffers use.
///
/// The encoder declines input it cannot shrink, reporting
/// `SOURCE_ABI_DECLINED` so a caller stores the original instead. The
/// required length is always reported, so an undersized buffer can be sized
/// and the call retried.
///
/// `window` is how far back a reference may reach and must be a power of
/// two; the save system uses a smaller one than the default, and the window
/// changes the bytes produced, so it is the caller's to choose.
///
/// # Safety
/// `input` must describe readable memory and `out_bytes` writable storage for
/// `capacity` bytes, not overlapping `input`. `out_length` must point to
/// writable `u64` storage.
#[no_mangle]
pub unsafe extern "C" fn source_compress_lzss_compress(
    input: SourceAbiSlice,
    window: u32,
    out_bytes: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        let Ok(bytes) = (unsafe { read_slice(input) }) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        let compressed = match source_compress::compress_with_window(bytes, window as usize) {
            Ok(compressed) => compressed,
            // Declining to grow a buffer is an outcome rather than a fault,
            // and the caller stores the original when it happens.
            Err(source_compress::CompressError::WouldGrow)
            | Err(source_compress::CompressError::TooSmall(_)) => return SOURCE_ABI_DECLINED,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        unsafe { write_bytes_out(&compressed, out_bytes, capacity, out_length) }
    })
}

/// Decompresses an LZSS buffer.
///
/// # Safety
/// As `source_compress_lzss_compress`.
#[no_mangle]
pub unsafe extern "C" fn source_compress_lzss_decompress(
    input: SourceAbiSlice,
    out_bytes: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        let Ok(bytes) = (unsafe { read_slice(input) }) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        let plain = match source_compress::decompress(bytes) {
            Ok(plain) => plain,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe { write_bytes_out(&plain, out_bytes, capacity, out_length) }
    })
}

/// Reports whether a buffer carries the `LZSS` tag and, if so, the
/// uncompressed length it declares.
///
/// # Safety
/// `input` must describe readable memory and `out_actual_size` writable `u64`
/// storage.
#[no_mangle]
pub unsafe extern "C" fn source_compress_lzss_actual_size(
    input: SourceAbiSlice,
    out_actual_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_actual_size.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_actual_size, 0) };
        let Ok(bytes) = (unsafe { read_slice(input) }) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        match source_compress::actual_size(bytes) {
            Some(size) => {
                unsafe { ptr::write(out_actual_size, size as u64) };
                SOURCE_ABI_OK
            }
            None => SOURCE_ABI_FORMAT_ERROR,
        }
    })
}

/// Copies a produced buffer out, reporting the length it needed either way.
///
/// # Safety
/// `out_bytes` must be writable for `capacity` bytes and `out_length` must
/// point to writable `u64` storage.
unsafe fn write_bytes_out(
    produced: &[u8],
    out_bytes: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> SourceAbiStatus {
    if out_length.is_null() {
        return SOURCE_ABI_INVALID_ARGUMENT;
    }
    unsafe { ptr::write(out_length, produced.len() as u64) };
    let Ok(capacity) = checked_len(capacity) else {
        return SOURCE_ABI_INVALID_ARGUMENT;
    };
    if capacity != 0 && out_bytes.is_null() {
        return SOURCE_ABI_INVALID_ARGUMENT;
    }
    if produced.len() > capacity {
        return SOURCE_ABI_BUFFER_TOO_SMALL;
    }
    // SAFETY: the caller guarantees `capacity` writable non-overlapping
    // bytes, and the length was checked to fit within them.
    if !produced.is_empty() {
        unsafe { ptr::copy_nonoverlapping(produced.as_ptr(), out_bytes, produced.len()) };
    }
    SOURCE_ABI_OK
}

/// A decoded `SPLITPACKET` header.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SourceAbiSplitPacketHeader {
    pub sequence: i32,
    pub packet_number: u32,
    pub packet_count: u32,
    pub split_size: u32,
}

/// Every split failure describes a datagram that cannot be placed, which is a
/// malformed stream rather than a caller mistake.
fn split_packet_error_status(_error: source_net::SplitPacketError) -> SourceAbiStatus {
    SOURCE_ABI_FORMAT_ERROR
}

#[no_mangle]
/// Encodes a `SPLITPACKET` header into `out_bytes`, which must hold twelve.
///
/// # Safety
/// `out_bytes` must point to writable storage for twelve bytes.
pub unsafe extern "C" fn source_net_split_packet_header_encode(
    sequence: i32,
    packet_number: u32,
    packet_count: u32,
    split_size: u32,
    out_bytes: *mut u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_bytes.is_null()
            || packet_number > u32::from(u8::MAX)
            || packet_count > u32::from(u8::MAX)
            || split_size > u32::from(u16::MAX)
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let header = source_net::SplitPacketHeader {
            sequence,
            packet_number: packet_number as u8,
            packet_count: packet_count as u8,
            split_size: split_size as u16,
        };
        let encoded = header.encode();
        // SAFETY: the caller guarantees twelve writable bytes, which is what
        // the header encodes to.
        unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), out_bytes, encoded.len()) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Decodes a `SPLITPACKET` header off the front of a datagram.
///
/// # Safety
/// `datagram` must describe readable memory and `out_header` writable storage.
pub unsafe extern "C" fn source_net_split_packet_header_decode(
    datagram: SourceAbiSlice,
    out_header: *mut SourceAbiSplitPacketHeader,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_header.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_header, SourceAbiSplitPacketHeader::default()) };
        let bytes = match unsafe { read_slice(datagram) } {
            Ok(bytes) => bytes,
            Err(status) => return status,
        };
        let header = match source_net::SplitPacketHeader::decode(bytes) {
            Ok(header) => header,
            Err(error) => return split_packet_error_status(error),
        };
        unsafe {
            ptr::write(
                out_header,
                SourceAbiSplitPacketHeader {
                    sequence: header.sequence,
                    packet_number: u32::from(header.packet_number),
                    packet_count: u32::from(header.packet_count),
                    split_size: u32::from(header.split_size),
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Starts tracking one peer's split datagrams.
///
/// # Safety
/// `out_peer_id` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_split_packet_create(
    handle: SourceAbiHandle,
    out_peer_id: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_peer_id.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_peer_id, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let peer_id = context
            .splits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .create();
        unsafe { ptr::write(out_peer_id, peer_id) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Stops tracking a peer, discarding any partial message it had.
pub extern "C" fn source_context_split_packet_remove(
    handle: SourceAbiHandle,
    peer_id: u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let removed = context
            .splits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(peer_id);
        if removed {
            SOURCE_ABI_OK
        } else {
            SOURCE_ABI_INVALID_ARGUMENT
        }
    })
}

#[no_mangle]
/// Forgets a peer's partial message, for one whose pieces have gone stale.
pub extern "C" fn source_context_split_packet_reset(
    handle: SourceAbiHandle,
    peer_id: u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let mut splits = context
            .splits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match splits.get_mut(peer_id) {
            Some(reassembler) => {
                reassembler.reset();
                SOURCE_ABI_OK
            }
            None => SOURCE_ABI_INVALID_ARGUMENT,
        }
    })
}

#[no_mangle]
/// Takes one split datagram from a peer, writing the whole message out once
/// its last piece arrives.
///
/// `out_complete` is set only when a message was both finished and copied
/// out. A message too large for `capacity` is reported through `out_length`
/// with `SOURCE_ABI_BUFFER_TOO_SMALL` and held for
/// `source_context_split_packet_collect`, so a caller that guessed its buffer
/// size wrong does not lose the message.
///
/// # Safety
/// `datagram` must describe readable memory, `out_bytes` writable storage for
/// `capacity` bytes, and `out_length` and `out_complete` writable storage.
pub unsafe extern "C" fn source_context_split_packet_accept(
    handle: SourceAbiHandle,
    peer_id: u64,
    datagram: SourceAbiSlice,
    out_bytes: *mut u8,
    capacity: u64,
    out_length: *mut u64,
    out_complete: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_length.is_null() || out_complete.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_length, 0);
            ptr::write(out_complete, 0);
        }
        let capacity = match checked_len(capacity) {
            Ok(capacity) => capacity,
            Err(status) => return status,
        };
        if capacity != 0 && out_bytes.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let bytes = match unsafe { read_slice(datagram) } {
            Ok(bytes) => bytes,
            Err(status) => return status,
        };

        let mut splits = context
            .splits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(reassembler) = splits.get_mut(peer_id) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        let message = match reassembler.accept(bytes) {
            Ok(message) => message,
            Err(error) => return split_packet_error_status(error),
        };
        let Some(message) = message else {
            return SOURCE_ABI_OK;
        };

        unsafe { ptr::write(out_length, message.len() as u64) };
        if message.len() > capacity {
            splits.hold(peer_id, message);
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        // SAFETY: the caller guarantees `capacity` writable bytes and the
        // message was checked to fit within them.
        unsafe {
            ptr::copy_nonoverlapping(message.as_ptr(), out_bytes, message.len());
            ptr::write(out_complete, 1);
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Takes a message that was finished but too large for the buffer offered.
///
/// # Safety
/// `out_bytes` must point to writable storage for `capacity` bytes and
/// `out_length` to writable `u64` storage.
pub unsafe extern "C" fn source_context_split_packet_collect(
    handle: SourceAbiHandle,
    peer_id: u64,
    out_bytes: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_length.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_length, 0) };
        let capacity = match checked_len(capacity) {
            Ok(capacity) => capacity,
            Err(status) => return status,
        };
        if capacity != 0 && out_bytes.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let mut splits = context
            .splits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(length) = splits.held_len(peer_id) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        unsafe { ptr::write(out_length, length as u64) };
        if length > capacity {
            // Left held, so a second undersized attempt is not fatal either.
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let message = splits.take_held(peer_id).unwrap_or_default();
        // SAFETY: the caller guarantees `capacity` writable bytes and the
        // message was checked to fit within them.
        unsafe { ptr::copy_nonoverlapping(message.as_ptr(), out_bytes, message.len()) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Creates a Rust-owned canonical network string table.
///
/// # Safety
/// `name` must describe readable bytes and `out_table_id` must point to writable
/// `u64` storage.
pub unsafe extern "C" fn source_context_string_table_create(
    handle: SourceAbiHandle,
    name: SourceAbiSlice,
    max_entries: u32,
    tick: i32,
    out_table_id: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_table_id.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_table_id, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let name = match unsafe { read_slice(name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let table_id = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .create(name, max_entries, tick)
        {
            Ok(table_id) => table_id,
            Err(error) => return string_table_error_status(error),
        };
        unsafe { ptr::write(out_table_id, table_id) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_string_table_remove(
    handle: SourceAbiHandle,
    table_id: u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(table_id)
        {
            SOURCE_ABI_OK
        } else {
            SOURCE_ABI_NOT_FOUND
        }
    })
}

#[no_mangle]
pub extern "C" fn source_context_string_table_clear(
    handle: SourceAbiHandle,
    table_id: u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let status = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear(table_id)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => string_table_error_status(error),
        };
        status
    })
}

#[no_mangle]
pub extern "C" fn source_context_string_table_enable_history(
    handle: SourceAbiHandle,
    table_id: u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let status = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .enable_history(table_id)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => string_table_error_status(error),
        };
        status
    })
}

#[no_mangle]
pub extern "C" fn source_context_string_table_set_tick(
    handle: SourceAbiHandle,
    table_id: u64,
    tick: i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let status = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_tick(table_id, tick)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => string_table_error_status(error),
        };
        status
    })
}

#[no_mangle]
/// Synchronizes a legacy rollback/copy cursor that may move between historical ticks.
pub extern "C" fn source_context_string_table_synchronize_tick(
    handle: SourceAbiHandle,
    table_id: u64,
    tick: i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let status = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .synchronize_tick(table_id, tick)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => string_table_error_status(error),
        };
        status
    })
}

#[no_mangle]
/// Inserts or updates one canonical networked string-table entry.
///
/// `update_user_data` must be zero to preserve existing data or one to apply
/// `user_data`, including an empty slice to clear it.
///
/// # Safety
/// Input slices must describe readable bytes and `out_result` must point to
/// writable `SourceAbiStringTableUpsert` storage.
pub unsafe extern "C" fn source_context_string_table_upsert(
    handle: SourceAbiHandle,
    table_id: u64,
    value: SourceAbiSlice,
    update_user_data: u32,
    user_data: SourceAbiSlice,
    out_result: *mut SourceAbiStringTableUpsert,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_result.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_result, SourceAbiStringTableUpsert::default()) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let value = match unsafe { read_slice(value) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let user_data = match update_user_data {
            0 if user_data.length == 0 => None,
            0 => return SOURCE_ABI_INVALID_ARGUMENT,
            1 => match unsafe { read_slice(user_data) } {
                Ok(user_data) => Some(user_data),
                Err(status) => return status,
            },
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let result = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .upsert(table_id, value, user_data)
        {
            Ok(result) => result,
            Err(error) => return string_table_error_status(error),
        };
        unsafe {
            ptr::write(
                out_result,
                SourceAbiStringTableUpsert {
                    index: result.index,
                    entry_count: result.entry_count,
                    created: u32::from(result.created),
                    user_data_changed: u32::from(result.user_data_changed),
                    tick_changed: result.tick_changed,
                    reserved: 0,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Replaces one canonical entry's user data.
///
/// # Safety
/// `user_data` must describe readable bytes and `out_change` must point to
/// writable `SourceAbiStringTableChange` storage.
pub unsafe extern "C" fn source_context_string_table_set_user_data(
    handle: SourceAbiHandle,
    table_id: u64,
    index: u32,
    user_data: SourceAbiSlice,
    out_change: *mut SourceAbiStringTableChange,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_change.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_change, SourceAbiStringTableChange::default()) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let user_data = match unsafe { read_slice(user_data) } {
            Ok(user_data) => user_data,
            Err(status) => return status,
        };
        let change = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_user_data(table_id, index, user_data)
        {
            Ok(change) => change,
            Err(error) => return string_table_error_status(error),
        };
        unsafe {
            ptr::write(
                out_change,
                SourceAbiStringTableChange {
                    tick_changed: change.tick_changed,
                    changed: u32::from(change.changed),
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Finds an entry using Source's case-insensitive string-table semantics.
///
/// # Safety
/// `value` must describe readable bytes and `out_index` must point to writable
/// `u32` storage.
pub unsafe extern "C" fn source_context_string_table_find(
    handle: SourceAbiHandle,
    table_id: u64,
    value: SourceAbiSlice,
    out_index: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_index.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_index, u32::MAX) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let value = match unsafe { read_slice(value) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let index = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .find(table_id, value)
        {
            Ok(index) => index,
            Err(error) => return string_table_error_status(error),
        };
        unsafe { ptr::write(out_index, index) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns whether the canonical table changed after `tick`.
///
/// # Safety
/// `out_changed` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_string_table_changed_since(
    handle: SourceAbiHandle,
    table_id: u64,
    tick: i32,
    out_changed: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_changed.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_changed, 0) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let changed = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .changed_since(table_id, tick)
        {
            Ok(changed) => changed,
            Err(error) => return string_table_error_status(error),
        };
        unsafe { ptr::write(out_changed, u32::from(changed)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Restores history-backed entry data and returns the last visible change tick.
///
/// # Safety
/// `out_last_changed_tick` must point to writable `i32` storage.
pub unsafe extern "C" fn source_context_string_table_restore_tick(
    handle: SourceAbiHandle,
    table_id: u64,
    tick: i32,
    out_last_changed_tick: *mut i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_last_changed_tick.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_last_changed_tick, 0) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let last_changed_tick = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .restore_tick(table_id, tick)
        {
            Ok(last_changed_tick) => last_changed_tick,
            Err(error) => return string_table_error_status(error),
        };
        unsafe { ptr::write(out_last_changed_tick, last_changed_tick) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns the canonical entry count.
///
/// # Safety
/// `out_entry_count` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_string_table_entry_count(
    handle: SourceAbiHandle,
    table_id: u64,
    out_entry_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_entry_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_entry_count, 0) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let entry_count = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry_count(table_id)
        {
            Ok(entry_count) => entry_count,
            Err(error) => return string_table_error_status(error),
        };
        unsafe { ptr::write(out_entry_count, entry_count) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Encodes a table's canonical entries in the layout demo and save containers
/// use, so the bytes a recording emits come from the same state the rest of the
/// engine reads.
///
/// The bits are packed least-significant-bit first within each byte, matching
/// the native bit writer, so a caller appends them to its buffer unchanged and
/// then writes its own client-side section after them. Reports the required bit
/// count even when the buffer is too small.
///
/// # Safety
/// `out_bytes` must point to writable storage for `capacity` bytes, and
/// `out_bit_count` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_string_table_encode_entries(
    handle: SourceAbiHandle,
    table_id: u64,
    out_bytes: *mut u8,
    capacity: u64,
    out_bit_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_bit_count.is_null() || (capacity != 0 && out_bytes.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_bit_count, 0) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let (bytes, bit_count) = match context
            .string_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .encode_entries(table_id)
        {
            Ok(encoded) => encoded,
            Err(error) => return string_table_error_status(error),
        };
        unsafe { ptr::write(out_bit_count, bit_count) };
        if bytes.len() as u64 > capacity {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), out_bytes, bytes.len()) };
        SOURCE_ABI_OK
    })
}

unsafe fn source_context_string_table_read_impl(
    handle: SourceAbiHandle,
    table_id: u64,
    index: u32,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    read_user_data: bool,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        if table_id == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let contents = {
            let tables = context
                .string_tables
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let contents = if read_user_data {
                tables.user_data(table_id, index)
            } else {
                tables.string(table_id, index)
            };
            match contents {
                Ok(contents) => contents.to_vec(),
                Err(error) => return string_table_error_status(error),
            }
        };
        unsafe { ptr::write(out_written, contents.len() as u64) };
        if output.length < contents.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..contents.len()].copy_from_slice(&contents);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Copies one canonical entry string to caller-owned storage.
///
/// # Safety
/// A non-empty output slice must describe writable memory and `out_written`
/// must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_string_table_read_string(
    handle: SourceAbiHandle,
    table_id: u64,
    index: u32,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    unsafe {
        source_context_string_table_read_impl(handle, table_id, index, output, out_written, false)
    }
}

#[no_mangle]
/// Copies one canonical entry's user data to caller-owned storage.
///
/// # Safety
/// A non-empty output slice must describe writable memory and `out_written`
/// must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_string_table_read_user_data(
    handle: SourceAbiHandle,
    table_id: u64,
    index: u32,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    unsafe {
        source_context_string_table_read_impl(handle, table_id, index, output, out_written, true)
    }
}

#[no_mangle]
/// Clears the context-owned canonical server datatable schema.
pub extern "C" fn source_context_data_table_clear(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Registers one named send table and its declared raw property count.
///
/// # Safety
/// `name` must describe readable bytes and `out_registration` must point to
/// writable `SourceAbiDataTableRegistration` storage.
pub unsafe extern "C" fn source_context_data_table_register(
    handle: SourceAbiHandle,
    name: SourceAbiSlice,
    property_count: u32,
    out_registration: *mut SourceAbiDataTableRegistration,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_registration.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_registration, SourceAbiDataTableRegistration::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let name = match unsafe { read_slice(name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let registration = match context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_table(name, property_count)
        {
            Ok(registration) => registration,
            Err(error) => return data_table_error_status(error),
        };
        unsafe {
            ptr::write(
                out_registration,
                SourceAbiDataTableRegistration {
                    table_id: registration.table_id,
                    created: u32::from(registration.created),
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Appends one exact wire-facing property descriptor to a registered table.
///
/// # Safety
/// `property` must point to a readable descriptor whose slices remain readable
/// for the duration of the call.
pub unsafe extern "C" fn source_context_data_table_register_property(
    handle: SourceAbiHandle,
    table_id: u32,
    property: *const SourceAbiDataTableProperty,
) -> SourceAbiStatus {
    ffi_status(|| {
        if property.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let property = unsafe { ptr::read(property) };
        let name = match unsafe { read_slice(property.name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let reference_name = match unsafe { read_slice(property.reference_name) } {
            Ok(reference_name) => reference_name,
            Err(status) => return status,
        };
        let descriptor = source_net::DataTablePropertyDescriptor {
            name: name.to_vec(),
            reference_name: reference_name.to_vec(),
            property_type: property.property_type,
            flags: property.flags,
            bit_count: property.bit_count,
            elements: property.elements,
            low_value: property.low_value,
            high_value: property.high_value,
        };
        let status = match context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_property(table_id, descriptor)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(error) => data_table_error_status(error),
        };
        status
    })
}

#[no_mangle]
/// Registers one server class and assigns its canonical contiguous class ID.
///
/// # Safety
/// `name` must describe readable bytes and `out_class_id` must point to writable
/// `u32` storage.
pub unsafe extern "C" fn source_context_server_class_register(
    handle: SourceAbiHandle,
    name: SourceAbiSlice,
    table_id: u32,
    out_class_id: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_class_id.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_class_id, u32::MAX) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let name = match unsafe { read_slice(name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let class_id = match context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_class(name, table_id)
        {
            Ok(class_id) => class_id,
            Err(error) => return data_table_error_status(error),
        };
        unsafe { ptr::write(out_class_id, class_id) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Finalizes and validates the schema against the native compatibility CRC.
///
/// # Safety
/// `out_summary` must point to writable `SourceAbiDataTableSummary` storage.
pub unsafe extern "C" fn source_context_data_table_finalize(
    handle: SourceAbiHandle,
    native_compatibility_crc: u32,
    out_summary: *mut SourceAbiDataTableSummary,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_summary.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_summary, SourceAbiDataTableSummary::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let summary = match context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .finalize(native_compatibility_crc)
        {
            Ok(summary) => summary,
            Err(error) => return data_table_error_status(error),
        };
        unsafe { ptr::write(out_summary, data_table_summary(summary)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns the finalized canonical schema counts and compatibility CRC.
///
/// # Safety
/// `out_summary` must point to writable `SourceAbiDataTableSummary` storage.
pub unsafe extern "C" fn source_context_data_table_summary(
    handle: SourceAbiHandle,
    out_summary: *mut SourceAbiDataTableSummary,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_summary.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_summary, SourceAbiDataTableSummary::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let summary = match context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .summary()
        {
            Ok(summary) => summary,
            Err(error) => return data_table_error_status(error),
        };
        unsafe { ptr::write(out_summary, data_table_summary(summary)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Finds a finalized server class using Source's case-insensitive semantics.
///
/// # Safety
/// `name` must describe readable bytes and `out_class_id` must point to writable
/// `u32` storage.
pub unsafe extern "C" fn source_context_server_class_find(
    handle: SourceAbiHandle,
    name: SourceAbiSlice,
    out_class_id: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_class_id.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_class_id, u32::MAX) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let name = match unsafe { read_slice(name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let class_id = match context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .class_id(name)
        {
            Ok(class_id) => class_id,
            Err(error) => return data_table_error_status(error),
        };
        unsafe { ptr::write(out_class_id, class_id) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Clears all context-owned snapshots and pending explicit-delete slots.
pub extern "C" fn source_context_snapshot_clear(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        context
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Queues one entity slot for explicit deletion in the next canonical snapshot.
///
/// # Safety
/// `out_queued` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_snapshot_queue_delete(
    handle: SourceAbiHandle,
    slot: u32,
    out_queued: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_queued.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_queued, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let queued = match context
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .queue_explicit_delete(slot)
        {
            Ok(queued) => queued,
            Err(error) => return snapshot_error_status(error),
        };
        unsafe { ptr::write(out_queued, u32::from(queued)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Creates an immutable canonical frame snapshot from an ordered entity array.
///
/// The pending explicit-delete set is transferred to the snapshot only after
/// all metadata has passed validation.
///
/// # Safety
/// A non-empty `entities` array must remain readable for `entity_count`
/// elements during the call. Both output pointers must point to writable
/// storage.
pub unsafe extern "C" fn source_context_snapshot_create(
    handle: SourceAbiHandle,
    tick: i32,
    max_entities: u32,
    entities: *const SourceAbiSnapshotEntity,
    entity_count: u64,
    out_snapshot_id: *mut u64,
    out_summary: *mut SourceAbiSnapshotSummary,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_snapshot_id.is_null() || out_summary.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_snapshot_id, 0);
            ptr::write(out_summary, SourceAbiSnapshotSummary::default());
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let entity_count = match checked_len(entity_count) {
            Ok(count) if count <= source_net::MAX_SNAPSHOT_ENTITIES => count,
            Ok(_) => return SOURCE_ABI_BUFFER_TOO_SMALL,
            Err(status) => return status,
        };
        if entity_count != 0 && entities.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let entities = if entity_count == 0 {
            &[]
        } else {
            // SAFETY: the caller contract guarantees a readable array for the call, and its
            // bounded element count and non-null pointer were checked above.
            unsafe { std::slice::from_raw_parts(entities, entity_count) }
        };
        let class_count = match context
            .data_tables
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .summary()
        {
            Ok(summary) => summary.class_count,
            Err(error) => return data_table_error_status(error),
        };
        let mut owned_entities = Vec::with_capacity(entity_count);
        for entity in entities {
            if entity.reserved != 0 || entity.class_id >= class_count {
                return SOURCE_ABI_INVALID_ARGUMENT;
            }
            owned_entities.push(source_net::SnapshotEntity {
                entity_index: entity.entity_index,
                serial_number: entity.serial_number,
                class_id: entity.class_id,
            });
        }
        let (snapshot_id, summary) = match context
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .create(tick, max_entities, &owned_entities)
        {
            Ok(snapshot) => snapshot,
            Err(error) => return snapshot_error_status(error),
        };
        unsafe {
            ptr::write(out_snapshot_id, snapshot_id);
            ptr::write(out_summary, snapshot_summary(summary));
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Removes one immutable canonical frame snapshot.
pub extern "C" fn source_context_snapshot_remove(
    handle: SourceAbiHandle,
    snapshot_id: u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        if snapshot_id == 0
            || !context
                .snapshots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(snapshot_id)
        {
            SOURCE_ABI_NOT_FOUND
        } else {
            SOURCE_ABI_OK
        }
    })
}

#[no_mangle]
/// Returns the immutable metadata counts for one canonical snapshot.
///
/// # Safety
/// `out_summary` must point to writable `SourceAbiSnapshotSummary` storage.
pub unsafe extern "C" fn source_context_snapshot_summary(
    handle: SourceAbiHandle,
    snapshot_id: u64,
    out_summary: *mut SourceAbiSnapshotSummary,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_summary.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_summary, SourceAbiSnapshotSummary::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let summary = match context
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .summary(snapshot_id)
        {
            Ok(summary) => summary,
            Err(error) => return snapshot_error_status(error),
        };
        unsafe { ptr::write(out_summary, snapshot_summary(summary)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns one canonical snapshot entity by its stable ascending ordinal.
///
/// # Safety
/// `out_entity` must point to writable `SourceAbiSnapshotEntity` storage.
pub unsafe extern "C" fn source_context_snapshot_entity_at(
    handle: SourceAbiHandle,
    snapshot_id: u64,
    ordinal: u32,
    out_entity: *mut SourceAbiSnapshotEntity,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_entity.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_entity, SourceAbiSnapshotEntity::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let entity = match context
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entity(snapshot_id, ordinal)
        {
            Ok(entity) => entity,
            Err(error) => return snapshot_error_status(error),
        };
        unsafe { ptr::write(out_entity, snapshot_entity(entity)) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns one canonical explicit-delete slot by its stable insertion ordinal.
///
/// # Safety
/// `out_slot` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_snapshot_delete_at(
    handle: SourceAbiHandle,
    snapshot_id: u64,
    ordinal: u32,
    out_slot: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_slot.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_slot, u32::MAX) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let slot = match context
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .explicit_delete(snapshot_id, ordinal)
        {
            Ok(slot) => slot,
            Err(error) => return snapshot_error_status(error),
        };
        unsafe { ptr::write(out_slot, slot) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Classifies every entity index that differs between two canonical snapshots.
///
/// A `from_snapshot_id` of zero requests a full update, where every entity in
/// the newer snapshot has to be created. Each side may carry an ascending,
/// duplicate-free visibility set; a null pointer compares the snapshot's whole
/// entity list instead. Entries are written in ascending entity-index order,
/// which is the order the wire format requires. The required count is always
/// reported, so a caller that passes a short buffer receives
/// `SOURCE_ABI_BUFFER_TOO_SMALL` together with the size it needs.
///
/// # Safety
/// Each non-null visibility pointer must address that many readable `u32`
/// values. `out_deltas` must point to writable storage for `capacity` entries,
/// and `out_count` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_snapshot_delta(
    handle: SourceAbiHandle,
    from_snapshot_id: u64,
    from_visible: *const u32,
    from_visible_count: u64,
    to_snapshot_id: u64,
    to_visible: *const u32,
    to_visible_count: u64,
    out_deltas: *mut SourceAbiSnapshotDelta,
    capacity: u64,
    out_count: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_count.is_null() || (capacity != 0 && out_deltas.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_count, 0) };
        let from_visible = match unsafe { read_index_slice(from_visible, from_visible_count) } {
            Ok(slice) => slice,
            Err(status) => return status,
        };
        let to_visible = match unsafe { read_index_slice(to_visible, to_visible_count) } {
            Ok(slice) => slice,
            Err(status) => return status,
        };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let entries = match context
            .snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .delta(
                (from_snapshot_id != 0).then_some(source_net::SnapshotDeltaSide {
                    snapshot_id: from_snapshot_id,
                    transmit: from_visible,
                }),
                source_net::SnapshotDeltaSide {
                    snapshot_id: to_snapshot_id,
                    transmit: to_visible,
                },
            ) {
            Ok(entries) => entries,
            Err(error) => return snapshot_error_status(error),
        };
        unsafe { ptr::write(out_count, entries.len() as u64) };
        if entries.len() as u64 > capacity {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        for (ordinal, entry) in entries.into_iter().enumerate() {
            unsafe { ptr::write(out_deltas.add(ordinal), snapshot_delta(entry)) };
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Encodes a demo file header into the exact bytes a demo starts with.
///
/// Each name slice is a UTF-8 relative name that has to fit the fixed field
/// with room for its terminator. The encoder rejects a header the Rust parser
/// would refuse, so a recorder cannot emit a demo it could not read back.
///
/// # Safety
/// Every name slice must describe readable UTF-8 bytes. `out_bytes` must point
/// to writable storage for `capacity` bytes, and `out_size` must point to
/// writable `u64` storage.
pub unsafe extern "C" fn source_demo_header_encode(
    demo_protocol: i32,
    network_protocol: i32,
    server_name: SourceAbiSlice,
    client_name: SourceAbiSlice,
    map_name: SourceAbiSlice,
    game_directory: SourceAbiSlice,
    playback_time: f32,
    playback_ticks: i32,
    playback_frames: i32,
    signon_length: i32,
    out_bytes: *mut u8,
    capacity: u64,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_size.is_null() || (capacity != 0 && out_bytes.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_size, 0) };
        // Names may legitimately be empty before the client knows them.
        let names = [server_name, client_name, map_name, game_directory].map(|slice| unsafe {
            read_slice(slice).and_then(|bytes| {
                std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT)
            })
        });
        let [server_name, client_name, map_name, game_directory] = match names {
            [Ok(server), Ok(client), Ok(map), Ok(directory)] => [server, client, map, directory],
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let header = source_demo::Header {
            demo_protocol,
            network_protocol,
            server_name,
            client_name,
            map_name,
            game_directory,
            playback_time,
            playback_ticks,
            playback_frames,
            signon_length,
        };
        let bytes = match header.encode() {
            Ok(bytes) => bytes,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        unsafe { ptr::write(out_size, bytes.len() as u64) };
        if bytes.len() as u64 > capacity {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), out_bytes, bytes.len()) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Encodes one packet-entity header into protocol-25 wire bits.
///
/// `header_base` is the index of the previously written entity, or -1 before
/// the first one, because the header only carries the gap since that entity.
/// The bits are packed least-significant-bit first within each byte and low
/// byte first, so a caller can append them to a packet unchanged.
///
/// # Safety
/// `out_bytes` must point to writable storage for `capacity` bytes, and
/// `out_bit_count` must point to writable `u32` storage.
pub unsafe extern "C" fn source_delta_header_encode(
    entity_index: u32,
    header_base: i32,
    leave_pvs: u32,
    delete_entity: u32,
    enter_pvs: u32,
    out_bytes: *mut u8,
    capacity: u64,
    out_bit_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_bit_count.is_null() || (capacity != 0 && out_bytes.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_bit_count, 0) };
        let header = source_net::DeltaHeader {
            entity_index,
            header_base,
            leave_pvs: leave_pvs != 0,
            delete_entity: delete_entity != 0,
            enter_pvs: enter_pvs != 0,
        };
        let (bytes, bit_count) = match header.encode() {
            Ok(encoded) => encoded,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        unsafe { ptr::write(out_bit_count, bit_count) };
        if bytes.len() as u64 > capacity {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), out_bytes, bytes.len()) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Reads and parses a scene-image cache through the ordered content mounts.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. Both output pointers must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_probe_scene(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_scene_count: *mut u64,
    out_string_count: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_scene_count.is_null() || out_string_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_scene_count, 0);
            ptr::write(out_string_count, 0);
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let contents = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.read(virtual_path, path_id) {
                Ok(contents) => contents,
                Err(error) => return search_error_status(&error),
            }
        };
        let image = match source_scene::SceneImage::parse(&contents) {
            Ok(image) => image,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe {
            ptr::write(out_scene_count, image.entries().len() as u64);
            ptr::write(out_string_count, image.strings().len() as u64);
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Reads and structurally validates a demo through the ordered content mounts.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. `out_info` must point to writable `SourceAbiDemoInfo` storage.
pub unsafe extern "C" fn source_context_demo_validate(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_info: *mut SourceAbiDemoInfo,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_info.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_info, SourceAbiDemoInfo::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let contents = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.read(virtual_path, path_id) {
                Ok(contents) => contents,
                Err(error) => return search_error_status(&error),
            }
        };
        let demo = match source_demo::Demo::parse_with_options(
            &contents,
            source_demo::ParseOptions {
                required_network_protocol: None,
                ..source_demo::ParseOptions::default()
            },
        ) {
            Ok(demo) => demo,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let packet_count = demo
            .commands()
            .iter()
            .filter(|command| {
                matches!(
                    &command.data,
                    source_demo::CommandData::Signon(_) | source_demo::CommandData::Packet(_)
                )
            })
            .count();
        // The string-table sections a recording carries are decoded here rather
        // than left opaque, so a demo written by the Rust encoder is only
        // reported valid when it reads back as tables and entries.
        let mut string_table_section_count = 0u64;
        let mut string_table_entry_count = 0u64;
        for command in demo.commands() {
            let source_demo::CommandData::StringTables(payload) = &command.data else {
                continue;
            };
            let sections = match source_net::decode_string_table_container(payload) {
                Ok(sections) => sections,
                Err(_) => return SOURCE_ABI_FORMAT_ERROR,
            };
            string_table_section_count += sections.len() as u64;
            for section in &sections {
                string_table_entry_count += section.entries.len() as u64;
                if let Some(client_side) = &section.client_side {
                    string_table_entry_count += client_side.len() as u64;
                }
            }
        }
        unsafe {
            ptr::write(
                out_info,
                SourceAbiDemoInfo {
                    command_count: demo.commands().len() as u64,
                    packet_count: packet_count as u64,
                    playback_ticks: demo.header().playback_ticks,
                    network_protocol: demo.header().network_protocol,
                    string_table_section_count,
                    string_table_entry_count,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Composes a map-state save container from its token table and two data
/// sections, so the bytes written to disk come from the same code that reads
/// them back.
///
/// `token_count` must match the NUL-terminated names in `tokens`, because the
/// reader sizes its symbol table from the count alone. Reports the required
/// size even when the buffer is too small, and refuses input it would not
/// itself accept.
///
/// # Safety
/// Each input slice must describe readable bytes for its length. `out_bytes`
/// must point to writable storage for `capacity` bytes, and `out_size` must
/// point to writable `u64` storage.
pub unsafe extern "C" fn source_save_map_state_encode(
    tokens: SourceAbiSlice,
    token_count: u64,
    data_headers: SourceAbiSlice,
    data: SourceAbiSlice,
    out_bytes: *mut u8,
    capacity: u64,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_size.is_null() || (capacity != 0 && out_bytes.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_size, 0) };
        let tokens = match unsafe { read_slice(tokens) } {
            Ok(tokens) => tokens,
            Err(status) => return status,
        };
        let data_headers = match unsafe { read_slice(data_headers) } {
            Ok(headers) => headers,
            Err(status) => return status,
        };
        let data = match unsafe { read_slice(data) } {
            Ok(data) => data,
            Err(status) => return status,
        };
        let token_count = match usize::try_from(token_count) {
            Ok(count) => count,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let bytes = match source_save::encode_map_state(tokens, token_count, data_headers, data) {
            Ok(bytes) => bytes,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe { ptr::write(out_size, bytes.len() as u64) };
        if bytes.len() as u64 > capacity {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), out_bytes, bytes.len()) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Composes the leading save container: magic, version, sizes, token table and
/// game data. The embedded map states are appended by the caller afterwards.
///
/// # Safety
/// Each input slice must describe readable bytes for its length. `out_bytes`
/// must point to writable storage for `capacity` bytes, and `out_size` must
/// point to writable `u64` storage.
pub unsafe extern "C" fn source_save_container_header_encode(
    tokens: SourceAbiSlice,
    token_count: u64,
    game_data: SourceAbiSlice,
    out_bytes: *mut u8,
    capacity: u64,
    out_size: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_size.is_null() || (capacity != 0 && out_bytes.is_null()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_size, 0) };
        let tokens = match unsafe { read_slice(tokens) } {
            Ok(tokens) => tokens,
            Err(status) => return status,
        };
        let game_data = match unsafe { read_slice(game_data) } {
            Ok(data) => data,
            Err(status) => return status,
        };
        let token_count = match usize::try_from(token_count) {
            Ok(count) => count,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let bytes = match source_save::encode_save_container_header(tokens, token_count, game_data)
        {
            Ok(bytes) => bytes,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe { ptr::write(out_size, bytes.len() as u64) };
        if bytes.len() as u64 > capacity {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), out_bytes, bytes.len()) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Reads and structurally validates a save plus every embedded map state.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. `out_info` must point to writable `SourceAbiSaveInfo` storage.
pub unsafe extern "C" fn source_context_save_validate(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_info: *mut SourceAbiSaveInfo,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_info.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_info, SourceAbiSaveInfo::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let contents = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.read(virtual_path, path_id) {
                Ok(contents) => contents,
                Err(error) => return search_error_status(&error),
            }
        };
        let save = match source_save::SaveContainer::parse(&contents) {
            Ok(save) => save,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let mut map_state_count = 0u64;
        for file in &save.files {
            if file.name.len() >= 4
                && file.name[file.name.len() - 4..].eq_ignore_ascii_case(b".hl1")
            {
                if source_save::MapState::parse(file.data).is_err() {
                    return SOURCE_ABI_FORMAT_ERROR;
                }
                map_state_count += 1;
            }
        }
        unsafe {
            ptr::write(
                out_info,
                SourceAbiSaveInfo {
                    embedded_file_count: save.files.len() as u64,
                    embedded_map_state_count: map_state_count,
                    token_count: save.token_count as u64,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Loads an owned BSP world through the ordered content mounts.
///
/// # Safety
/// Virtual-path and non-empty path-ID slices must describe readable UTF-8
/// bytes. `out_info` must point to writable `SourceAbiWorldInfo` storage.
pub unsafe extern "C" fn source_context_world_load(
    handle: SourceAbiHandle,
    virtual_path: SourceAbiSlice,
    path_id: SourceAbiSlice,
    out_info: *mut SourceAbiWorldInfo,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_info.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_info, SourceAbiWorldInfo::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let virtual_path = match unsafe { read_utf8_slice(virtual_path) } {
            Ok(path) => path,
            Err(status) => return status,
        };
        let path_id = match unsafe { read_optional_utf8_slice(path_id) } {
            Ok(path_id) => path_id,
            Err(status) => return status,
        };
        let contents = {
            let filesystem = context
                .filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match filesystem.read(virtual_path, path_id) {
                Ok(contents) => contents,
                Err(error) => return search_error_status(&error),
            }
        };
        let bsp = match source_bsp::Bsp::parse(&contents) {
            Ok(bsp) => bsp,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let world = match source_bsp::World::parse(&bsp) {
            Ok(world) => world,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let info = SourceAbiWorldInfo {
            plane_count: world.planes().len() as u64,
            node_count: world.nodes().len() as u64,
            leaf_count: world.leaves().len() as u64,
            cluster_count: world.cluster_count() as u64,
        };
        *context
            .world
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(world);
        unsafe { ptr::write(out_info, info) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_world_clear(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        *context
            .world
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Resolves a point through the loaded BSP tree and returns its leaf metadata.
///
/// # Safety
/// `out_leaf` must point to writable `SourceAbiWorldLeaf` storage.
pub unsafe extern "C" fn source_context_world_point_leaf(
    handle: SourceAbiHandle,
    x: f32,
    y: f32,
    z: f32,
    out_leaf: *mut SourceAbiWorldLeaf,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_leaf.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_leaf, SourceAbiWorldLeaf::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let world = context
            .world
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(world) = world.as_ref() else {
            return SOURCE_ABI_NOT_FOUND;
        };
        let leaf_index = match world.point_leaf([x, y, z]) {
            Ok(index) => index,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let leaf = &world.leaves()[leaf_index];
        unsafe {
            ptr::write(
                out_leaf,
                SourceAbiWorldLeaf {
                    leaf_index: leaf_index as u64,
                    contents: leaf.contents,
                    cluster: i32::from(leaf.cluster),
                    area: u32::from(leaf.area),
                    flags: u32::from(leaf.flags),
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Decompresses one PVS/PAS row from the loaded BSP into caller-owned storage.
///
/// # Safety
/// A non-empty output slice must describe exclusively writable memory and
/// `out_written` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_world_visibility(
    handle: SourceAbiHandle,
    cluster: u32,
    kind: u32,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let kind = match source_bsp::VisibilityKind::try_from(kind) {
            Ok(kind) => kind,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let world = context
            .world
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(world) = world.as_ref() else {
            return SOURCE_ABI_NOT_FOUND;
        };
        let row = match world.visibility(cluster as usize, kind) {
            Ok(row) => row,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        unsafe { ptr::write(out_written, row.len() as u64) };
        if output.length < row.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..row.len()].copy_from_slice(&row);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_host_transition(
    handle: SourceAbiHandle,
    phase: u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let phase = match source_host::Phase::try_from(phase) {
            Ok(phase) => phase,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let status = match context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .transition(phase)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(_) => SOURCE_ABI_FORMAT_ERROR,
        };
        status
    })
}

#[no_mangle]
/// Returns the context's current host lifecycle phase.
///
/// # Safety
/// `out_phase` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_host_phase(
    handle: SourceAbiHandle,
    out_phase: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_phase.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_phase, source_host::Phase::Created as u32) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let phase = context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .phase();
        unsafe { ptr::write(out_phase, phase as u32) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_host_configure_scheduler(
    handle: SourceAbiHandle,
    tick_interval_ns: u64,
    max_catch_up_ticks: u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let status = match context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .configure_scheduler(tick_interval_ns, max_catch_up_ticks)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(_) => SOURCE_ABI_INVALID_ARGUMENT,
        };
        status
    })
}

#[no_mangle]
/// Advances the deterministic fixed-step scheduler with monotonic nanoseconds.
///
/// # Safety
/// `out_plan` must point to writable `SourceAbiFramePlan` storage.
pub unsafe extern "C" fn source_context_host_advance(
    handle: SourceAbiHandle,
    now_ns: u64,
    out_plan: *mut SourceAbiFramePlan,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_plan.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_plan, SourceAbiFramePlan::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let plan = match context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .advance(now_ns)
        {
            Ok(plan) => plan,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe {
            ptr::write(
                out_plan,
                SourceAbiFramePlan {
                    interpolation_numerator_ns: plan.interpolation_numerator_ns,
                    interpolation_denominator_ns: plan.interpolation_denominator_ns,
                    dropped_ns: plan.dropped_ns,
                    tick_count: plan.ticks,
                    reserved: 0,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Applies the Rust-owned frame pacing policy to a monotonic timestamp.
/// Sleeping remains a platform-adapter responsibility.
///
/// # Safety
/// `out_pace` must point to writable `SourceAbiFramePace` storage.
pub unsafe extern "C" fn source_context_host_pace(
    handle: SourceAbiHandle,
    now_ns: u64,
    minimum_frame_ns: u64,
    out_pace: *mut SourceAbiFramePace,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_pace.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_pace, SourceAbiFramePace::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let pace = context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pace_frame(now_ns, minimum_frame_ns);
        unsafe {
            ptr::write(
                out_pace,
                SourceAbiFramePace {
                    elapsed_ns: pace.elapsed_ns,
                    wait_ns: pace.wait_ns,
                    ready: u32::from(pace.ready),
                    reserved: 0,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Advances the active legacy-compatible simulation tick accumulator in Rust.
///
/// # Safety
/// `out_plan` must point to writable `SourceAbiTickPlan` storage.
pub unsafe extern "C" fn source_context_host_schedule_ticks(
    handle: SourceAbiHandle,
    frame_time: f64,
    tick_interval: f64,
    start_tick: i32,
    accumulate: u32,
    alternate_ticks: u32,
    out_plan: *mut SourceAbiTickPlan,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_plan.is_null() || accumulate > 1 || alternate_ticks > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_plan, SourceAbiTickPlan::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let plan = match context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .schedule_ticks(
                frame_time,
                tick_interval,
                start_tick,
                accumulate != 0,
                alternate_ticks != 0,
            ) {
            Ok(plan) => plan,
            Err(source_host::Error::WrongPhase(_)) => return SOURCE_ABI_FORMAT_ERROR,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        unsafe {
            ptr::write(
                out_plan,
                SourceAbiTickPlan {
                    previous_remainder: plan.previous_remainder,
                    remainder: plan.remainder,
                    next_tick: plan.next_tick,
                    tick_count: plan.ticks,
                    reserved: 0,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Stores the host operation that Rust will hand to the legacy executor at a
/// frame boundary. A newer unconsumed request replaces the previous request.
///
/// # Safety
/// Target and landmark slices must describe readable UTF-8 for the duration of
/// the call. Empty slices are valid where permitted by the operation kind.
pub unsafe extern "C" fn source_context_host_request_operation(
    handle: SourceAbiHandle,
    kind: u32,
    target: SourceAbiSlice,
    landmark: SourceAbiSlice,
    flags: u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let kind = match source_host::HostOperationKind::try_from(kind) {
            Ok(kind) => kind,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let target = match unsafe { read_slice(target) }
            .and_then(|bytes| std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
        {
            Ok(target) => target,
            Err(status) => return status,
        };
        let landmark = match unsafe { read_slice(landmark) }
            .and_then(|bytes| std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
        {
            Ok(landmark) => landmark,
            Err(status) => return status,
        };
        let operation = match source_host::HostOperation::new(kind, target, landmark, flags) {
            Ok(operation) => operation,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let status = match context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .request_operation(operation)
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(_) => SOURCE_ABI_FORMAT_ERROR,
        };
        status
    })
}

#[no_mangle]
/// Copies and consumes the pending Rust-owned host operation.
///
/// On `SOURCE_ABI_BUFFER_TOO_SMALL`, `out_info` reports the required byte
/// lengths and the operation remains pending.
///
/// # Safety
/// Output slices must describe exclusively writable memory, and `out_info`
/// must point to writable `SourceAbiHostOperationInfo` storage.
pub unsafe extern "C" fn source_context_host_take_operation(
    handle: SourceAbiHandle,
    target_output: SourceAbiMutSlice,
    landmark_output: SourceAbiMutSlice,
    out_info: *mut SourceAbiHostOperationInfo,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_info.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_info, SourceAbiHostOperationInfo::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let mut host = context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let operation = match host.pending_operation() {
            Ok(Some(operation)) => operation.clone(),
            Ok(None) => return SOURCE_ABI_NOT_FOUND,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let info = SourceAbiHostOperationInfo {
            target_length: operation.target.len() as u64,
            landmark_length: operation.landmark.len() as u64,
            kind: operation.kind as u32,
            flags: operation.flags,
        };
        unsafe { ptr::write(out_info, info) };
        if target_output.length < info.target_length
            || landmark_output.length < info.landmark_length
        {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let target_output = match unsafe { write_slice(target_output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        let landmark_output = match unsafe { write_slice(landmark_output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        target_output[..operation.target.len()].copy_from_slice(operation.target.as_bytes());
        landmark_output[..operation.landmark.len()].copy_from_slice(operation.landmark.as_bytes());
        match host.take_operation() {
            Ok(Some(_)) => SOURCE_ABI_OK,
            _ => SOURCE_ABI_INTERNAL_ERROR,
        }
    })
}

#[no_mangle]
/// Reports the pending host operation kind without consuming it.
///
/// # Safety
/// `out_kind` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_host_pending_operation(
    handle: SourceAbiHandle,
    out_kind: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_kind.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_kind, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let host = context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match host.pending_operation() {
            Ok(Some(operation)) => {
                unsafe { ptr::write(out_kind, operation.kind as u32) };
                SOURCE_ABI_OK
            }
            Ok(None) => SOURCE_ABI_NOT_FOUND,
            Err(_) => SOURCE_ABI_FORMAT_ERROR,
        }
    })
}

#[no_mangle]
pub extern "C" fn source_context_host_clear_operation(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let status = match context
            .host
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear_operation()
        {
            Ok(()) => SOURCE_ABI_OK,
            Err(_) => SOURCE_ABI_FORMAT_ERROR,
        };
        status
    })
}

#[no_mangle]
/// Runs legacy engine sessions under the Rust host's bounded restart loop.
///
/// # Safety
/// `run_session` must remain callable for the duration of this call, must not
/// unwind across the C ABI, and must honor the session result constants.
/// `out_session_count` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_host_run_sessions(
    handle: SourceAbiHandle,
    run_session: Option<SourceAbiSessionFn>,
    user_data: *mut c_void,
    max_sessions: u32,
    out_session_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if run_session.is_none() || out_session_count.is_null() || max_sessions == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_session_count, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let run_session = run_session.expect("checked callback");
        {
            let host = context
                .host
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if host.ensure_legacy_running().is_err() {
                return SOURCE_ABI_FORMAT_ERROR;
            }
        }
        let result = source_host::run_legacy_sessions(max_sessions, || {
            // SAFETY: the caller guarantees that the callback and user data remain valid.
            match unsafe { run_session(user_data) } {
                0 => source_host::SessionAction::Stop,
                1 => source_host::SessionAction::Restart,
                _ => source_host::SessionAction::Failed,
            }
        });
        match result {
            Ok(session_count) => {
                unsafe { ptr::write(out_session_count, session_count) };
                SOURCE_ABI_OK
            }
            Err(_) => SOURCE_ABI_FORMAT_ERROR,
        }
    })
}

#[no_mangle]
/// Runs the active engine frame/message iterations under Rust-owned control.
/// A zero `max_iterations` means unbounded operation until the callback asks
/// to stop or restart.
///
/// # Safety
/// `run_frame` must remain callable for the duration of this call and must not
/// unwind across the C ABI. `out_info` must point to writable storage.
pub unsafe extern "C" fn source_context_host_run_frames(
    handle: SourceAbiHandle,
    run_frame: Option<SourceAbiFrameFn>,
    user_data: *mut c_void,
    max_iterations: u64,
    out_info: *mut SourceAbiFrameLoopInfo,
) -> SourceAbiStatus {
    ffi_status(|| {
        if run_frame.is_none() || out_info.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_info, SourceAbiFrameLoopInfo::default()) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        {
            let host = context
                .host
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if host.ensure_legacy_running().is_err() {
                return SOURCE_ABI_FORMAT_ERROR;
            }
        }
        let run_frame = run_frame.expect("checked callback");
        let report = match source_host::run_legacy_frames(max_iterations, || {
            // SAFETY: the caller guarantees callback and user-data validity.
            match unsafe { run_frame(user_data) } {
                0 => source_host::FrameAction::Continue,
                1 => source_host::FrameAction::Stop,
                2 => source_host::FrameAction::Restart,
                _ => source_host::FrameAction::Failed,
            }
        }) {
            Ok(report) => report,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let exit_reason = match report.exit {
            source_host::FrameLoopExit::Stop => 1,
            source_host::FrameLoopExit::Restart => 2,
        };
        unsafe {
            ptr::write(
                out_info,
                SourceAbiFrameLoopInfo {
                    iteration_count: report.iterations,
                    exit_reason,
                    reserved: 0,
                },
            )
        };
        SOURCE_ABI_OK
    })
}

fn console_error_status(error: &source_console::Error) -> SourceAbiStatus {
    match error {
        source_console::Error::NotFound => SOURCE_ABI_NOT_FOUND,
        source_console::Error::EmptyName
        | source_console::Error::InvalidName
        | source_console::Error::NameTooLong
        | source_console::Error::ValueTooLong
        | source_console::Error::UnterminatedQuote
        | source_console::Error::InvalidEscape
        | source_console::Error::CommandTooLong
        | source_console::Error::TooManyArguments => SOURCE_ABI_INVALID_ARGUMENT,
        source_console::Error::AlreadyRegistered
        | source_console::Error::ReadOnly
        | source_console::Error::QueueFull => SOURCE_ABI_FORMAT_ERROR,
    }
}

#[no_mangle]
/// Registers an idempotent, context-owned console variable.
///
/// # Safety
/// Name and default-value slices must describe readable UTF-8 bytes for the
/// duration of the call. The default value may be empty.
pub unsafe extern "C" fn source_context_cvar_register(
    handle: SourceAbiHandle,
    name: SourceAbiSlice,
    default_value: SourceAbiSlice,
    flags: u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let name = match unsafe { read_utf8_slice(name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let default_value = match unsafe { read_slice(default_value) }
            .and_then(|bytes| std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
        {
            Ok(value) => value,
            Err(status) => return status,
        };
        let status = match context
            .console
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register(name, default_value, flags)
        {
            Ok(_) => SOURCE_ABI_OK,
            Err(error) => console_error_status(&error),
        };
        status
    })
}

#[no_mangle]
/// Updates a registered console variable. Read-only variables reject writes.
///
/// # Safety
/// Name and value slices must describe readable UTF-8 bytes for the duration
/// of the call. The value may be empty.
pub unsafe extern "C" fn source_context_cvar_set(
    handle: SourceAbiHandle,
    name: SourceAbiSlice,
    value: SourceAbiSlice,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let name = match unsafe { read_utf8_slice(name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let value = match unsafe { read_slice(value) }
            .and_then(|bytes| std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
        {
            Ok(value) => value,
            Err(status) => return status,
        };
        let status = match context
            .console
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set(name, value)
        {
            Ok(_) => SOURCE_ABI_OK,
            Err(error) => console_error_status(&error),
        };
        status
    })
}

#[no_mangle]
/// Reads a console variable into caller-owned storage without a trailing NUL.
/// An empty output slice performs a size query.
///
/// # Safety
/// Name and output slices must remain valid for the call. `out_written` and
/// `out_info` must point to writable storage.
pub unsafe extern "C" fn source_context_cvar_get(
    handle: SourceAbiHandle,
    name: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
    out_info: *mut SourceAbiCvarInfo,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() || out_info.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_written, 0);
            ptr::write(out_info, SourceAbiCvarInfo::default());
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let name = match unsafe { read_utf8_slice(name) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let console = context
            .console
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let variable = match console.variable(name) {
            Ok(variable) => variable,
            Err(error) => return console_error_status(&error),
        };
        let value = variable.value().as_bytes();
        unsafe {
            ptr::write(out_written, value.len() as u64);
            ptr::write(
                out_info,
                SourceAbiCvarInfo {
                    generation: variable.generation(),
                    flags: variable.flags(),
                    reserved: 0,
                },
            );
        }
        if output.length < value.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..value.len()].copy_from_slice(value);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Tokenizes and queues one or more bounded Source-style command lines.
///
/// # Safety
/// The script slice must describe readable UTF-8 bytes for the call and
/// `out_command_count` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_command_enqueue(
    handle: SourceAbiHandle,
    script: SourceAbiSlice,
    out_command_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_command_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_command_count, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let script = match unsafe { read_slice(script) }
            .and_then(|bytes| std::str::from_utf8(bytes).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
        {
            Ok(script) => script,
            Err(status) => return status,
        };
        let count = match context
            .console
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .enqueue(script)
        {
            Ok(count) => count,
            Err(error) => return console_error_status(&error),
        };
        let count = match u32::try_from(count) {
            Ok(count) => count,
            Err(_) => return SOURCE_ABI_INTERNAL_ERROR,
        };
        unsafe { ptr::write(out_command_count, count) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns and removes the next canonical command using caller-owned storage.
/// A size query does not remove the command.
///
/// # Safety
/// A non-empty output slice must be exclusively writable for the call and
/// `out_written` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_command_pop(
    handle: SourceAbiHandle,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let mut console = context
            .console
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let command = match console.front() {
            Some(command) => command.canonical(),
            None => return SOURCE_ABI_NOT_FOUND,
        };
        unsafe { ptr::write(out_written, command.len() as u64) };
        if output.length < command.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..command.len()].copy_from_slice(command.as_bytes());
        console.pop();
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Updates one physical modifier key and returns the collapsed Source mask.
///
/// # Safety
/// `out_modifier_mask` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_input_modifier(
    handle: SourceAbiHandle,
    modifier: u32,
    pressed: u8,
    out_modifier_mask: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_modifier_mask.is_null() || pressed > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_modifier_mask, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let mut input = context
            .input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match input.update_modifier(modifier, pressed != 0) {
            Ok(mask) => {
                unsafe { ptr::write(out_modifier_mask, mask) };
                SOURCE_ABI_OK
            }
            Err(_) => SOURCE_ABI_INVALID_ARGUMENT,
        }
    })
}

#[no_mangle]
/// Updates one Source mouse-button bit and returns the complete button mask.
///
/// # Safety
/// `out_button_mask` must point to writable `u32` storage.
pub unsafe extern "C" fn source_context_input_mouse_button(
    handle: SourceAbiHandle,
    button: u32,
    pressed: u8,
    out_button_mask: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_button_mask.is_null() || pressed > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_button_mask, 0) };
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let mut input = context
            .input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match input.update_mouse_button(button, pressed != 0) {
            Ok(mask) => {
                unsafe { ptr::write(out_button_mask, mask) };
                SOURCE_ABI_OK
            }
            Err(_) => SOURCE_ABI_INVALID_ARGUMENT,
        }
    })
}

#[no_mangle]
pub extern "C" fn source_context_input_mouse_motion(
    handle: SourceAbiHandle,
    delta_x: i32,
    delta_y: i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        context
            .input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .add_mouse_delta(delta_x, delta_y);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns and clears the accumulated relative mouse delta.
///
/// # Safety
/// Both output pointers must point to writable `i32` storage.
pub unsafe extern "C" fn source_context_input_take_mouse_delta(
    handle: SourceAbiHandle,
    out_delta_x: *mut i32,
    out_delta_y: *mut i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_delta_x.is_null() || out_delta_y.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_delta_x, 0);
            ptr::write(out_delta_y, 0);
        }
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let delta = context
            .input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take_mouse_delta();
        unsafe {
            ptr::write(out_delta_x, delta.0);
            ptr::write(out_delta_y, delta.1);
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_input_reset(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        let context = match get_context(handle) {
            Ok(context) => context,
            Err(status) => return status,
        };
        context
            .input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset();
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Reads and validates a VPK directory file using the Rust asset implementation.
///
/// # Safety
/// A non-empty path must describe readable UTF-8 bytes for the duration of the
/// call. Both output pointers must point to writable storage.
pub unsafe extern "C" fn source_context_probe_vpk(
    handle: SourceAbiHandle,
    path: SourceAbiSlice,
    out_entry_count: *mut u64,
    out_version: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_entry_count.is_null() || out_version.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_entry_count, 0);
            ptr::write(out_version, 0);
        }
        if get_context(handle).is_err() {
            return SOURCE_ABI_INVALID_HANDLE;
        }
        let path = match unsafe { read_slice(path) } {
            Ok(path) if !path.is_empty() => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let path = match std::str::from_utf8(path) {
            Ok(path) => path,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => return SOURCE_ABI_IO_ERROR,
        };
        let archive = match source_vpk::Archive::parse(&bytes) {
            Ok(archive) => archive,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe {
            ptr::write(out_entry_count, archive.entries().len() as u64);
            ptr::write(out_version, archive.header().version);
        }
        SOURCE_ABI_OK
    })
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

#[no_mangle]
/// Reads and CRC-validates one VPK member into caller-owned storage.
///
/// `out_written` always receives the required size after a successful archive
/// lookup, including when `output` is too small. Passing an empty output slice
/// therefore performs a bounded size query without transferring ownership.
///
/// # Safety
/// Non-empty directory and entry paths must describe readable UTF-8 bytes for
/// the duration of the call. A non-empty output slice must describe exclusively
/// writable memory and `out_written` must point to writable `u64` storage.
pub unsafe extern "C" fn source_context_read_vpk_file(
    handle: SourceAbiHandle,
    directory_path: SourceAbiSlice,
    entry_path: SourceAbiSlice,
    output: SourceAbiMutSlice,
    out_written: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_written.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { ptr::write(out_written, 0) };
        if get_context(handle).is_err() {
            return SOURCE_ABI_INVALID_HANDLE;
        }

        let directory_path = match unsafe { read_slice(directory_path) } {
            Ok(path) if !path.is_empty() => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let directory_path = match std::str::from_utf8(directory_path) {
            Ok(path) => Path::new(path),
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let entry_path = match unsafe { read_slice(entry_path) } {
            Ok(path) if !path.is_empty() => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let entry_path = match std::str::from_utf8(entry_path) {
            Ok(path) => path,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };

        let directory_bytes = match std::fs::read(directory_path) {
            Ok(bytes) => bytes,
            Err(_) => return SOURCE_ABI_IO_ERROR,
        };
        let archive = match source_vpk::Archive::parse(&directory_bytes) {
            Ok(archive) => archive,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let contents = match archive.read_file(entry_path, |index| {
            std::fs::read(vpk_chunk_path(directory_path, index))
                .map_err(|_| source_vpk::Error::MissingArchive(index))
        }) {
            Ok(contents) => contents,
            Err(source_vpk::Error::MissingArchive(_)) => return SOURCE_ABI_IO_ERROR,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe { ptr::write(out_written, contents.len() as u64) };
        if output.length < contents.len() as u64 {
            return SOURCE_ABI_BUFFER_TOO_SMALL;
        }
        let output = match unsafe { write_slice(output) } {
            Ok(output) => output,
            Err(status) => return status,
        };
        output[..contents.len()].copy_from_slice(&contents);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Reads, CRC-validates, and parses a scene-image cache stored in a VPK.
///
/// # Safety
/// Non-empty directory and entry paths must describe readable UTF-8 bytes for
/// the duration of the call. Both output pointers must point to writable `u64`
/// storage.
pub unsafe extern "C" fn source_context_probe_vpk_scene(
    handle: SourceAbiHandle,
    directory_path: SourceAbiSlice,
    entry_path: SourceAbiSlice,
    out_scene_count: *mut u64,
    out_string_count: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_scene_count.is_null() || out_string_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            ptr::write(out_scene_count, 0);
            ptr::write(out_string_count, 0);
        }
        if get_context(handle).is_err() {
            return SOURCE_ABI_INVALID_HANDLE;
        }

        let directory_path = match unsafe { read_slice(directory_path) } {
            Ok(path) if !path.is_empty() => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let directory_path = match std::str::from_utf8(directory_path) {
            Ok(path) => Path::new(path),
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let entry_path = match unsafe { read_slice(entry_path) } {
            Ok(path) if !path.is_empty() => path,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let entry_path = match std::str::from_utf8(entry_path) {
            Ok(path) => path,
            Err(_) => return SOURCE_ABI_INVALID_ARGUMENT,
        };

        let directory_bytes = match std::fs::read(directory_path) {
            Ok(bytes) => bytes,
            Err(_) => return SOURCE_ABI_IO_ERROR,
        };
        let archive = match source_vpk::Archive::parse(&directory_bytes) {
            Ok(archive) => archive,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let contents = match archive.read_file(entry_path, |index| {
            std::fs::read(vpk_chunk_path(directory_path, index))
                .map_err(|_| source_vpk::Error::MissingArchive(index))
        }) {
            Ok(contents) => contents,
            Err(source_vpk::Error::MissingArchive(_)) => return SOURCE_ABI_IO_ERROR,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        let image = match source_scene::SceneImage::parse(&contents) {
            Ok(image) => image,
            Err(_) => return SOURCE_ABI_FORMAT_ERROR,
        };
        unsafe {
            ptr::write(out_scene_count, image.entries().len() as u64);
            ptr::write(out_string_count, image.strings().len() as u64);
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
pub extern "C" fn source_context_live_count() -> u64 {
    lock_contexts().len() as u64
}

#[no_mangle]
pub extern "C" fn source_context_force_panic_for_test(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        if get_context(handle).is_err() {
            return SOURCE_ABI_INVALID_HANDLE;
        }
        panic!("intentional ABI containment test")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn test_log(_user: *mut c_void, level: i32, message: SourceAbiSlice) {
        // SAFETY: the ABI call keeps the message alive for this callback.
        let message = unsafe { read_slice(message) }.unwrap();
        assert_eq!(level, 7);
        assert_eq!(message, b"hello");
        CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    fn config() -> SourceAbiContextConfig {
        SourceAbiContextConfig {
            struct_size: std::mem::size_of::<SourceAbiContextConfig>() as u32,
            abi_version: SOURCE_ABI_VERSION,
            log: Some(test_log),
            user_data: ptr::null_mut(),
        }
    }

    fn synthetic_world_bsp() -> Vec<u8> {
        let mut planes = Vec::new();
        for value in [1.0f32, 0.0, 0.0, 0.0] {
            planes.extend_from_slice(&value.to_le_bytes());
        }
        planes.extend_from_slice(&0i32.to_le_bytes());

        let mut nodes = Vec::new();
        for value in [0i32, -1, -2] {
            nodes.extend_from_slice(&value.to_le_bytes());
        }
        nodes.resize(32, 0);

        let mut leaves = Vec::new();
        for cluster in [0i16, 1] {
            let start = leaves.len();
            leaves.extend_from_slice(&0i32.to_le_bytes());
            leaves.extend_from_slice(&cluster.to_le_bytes());
            leaves.resize(start + 28, 0);
            leaves.extend_from_slice(&(-1i16).to_le_bytes());
            leaves.resize(start + 32, 0);
        }

        let mut visibility = Vec::new();
        visibility.extend_from_slice(&2i32.to_le_bytes());
        for offset in [20i32, 21, 22, 23] {
            visibility.extend_from_slice(&offset.to_le_bytes());
        }
        visibility.extend_from_slice(&[0b01, 0b11, 0b10, 0b11]);

        let mut builder = source_bsp::Builder::new(20, 1);
        builder.set_lump(source_bsp::LUMP_PLANES, 0, 0, planes);
        builder.set_lump(source_bsp::LUMP_NODES, 0, 0, nodes);
        builder.set_lump(source_bsp::LUMP_LEAVES, 1, 0, leaves);
        builder.set_lump(source_bsp::LUMP_VISIBILITY, 0, 0, visibility);
        builder.build().unwrap()
    }

    fn synthetic_demo() -> Vec<u8> {
        let mut bytes = vec![0u8; source_demo::HEADER_SIZE];
        bytes[..8].copy_from_slice(source_demo::DEMO_STAMP);
        bytes[8..12].copy_from_slice(&source_demo::DEMO_PROTOCOL.to_le_bytes());
        bytes[12..16].copy_from_slice(&source_demo::NETWORK_PROTOCOL.to_le_bytes());
        bytes[1060..1064].copy_from_slice(&42i32.to_le_bytes());
        bytes.push(7);
        bytes.extend_from_slice(&42i32.to_le_bytes());
        bytes
    }

    fn synthetic_save() -> Vec<u8> {
        let mut map_state = Vec::new();
        map_state.extend_from_slice(&source_save::VALV_MAGIC.to_le_bytes());
        map_state.extend_from_slice(&source_save::SAVEGAME_VERSION.to_le_bytes());
        map_state.extend_from_slice(&0i32.to_le_bytes());
        map_state.extend_from_slice(&0i32.to_le_bytes());
        map_state.extend_from_slice(&0i32.to_le_bytes());
        map_state.extend_from_slice(&0i32.to_le_bytes());

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&source_save::JSAV_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&source_save::SAVEGAME_VERSION.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        let mut name = [0u8; source_save::EMBEDDED_NAME_SIZE];
        name[..8].copy_from_slice(b"map.hl1\0");
        bytes.extend_from_slice(&name);
        bytes.extend_from_slice(&(map_state.len() as i32).to_le_bytes());
        bytes.extend_from_slice(&map_state);
        bytes
    }

    fn slice(bytes: &[u8]) -> SourceAbiSlice {
        SourceAbiSlice {
            data: bytes.as_ptr(),
            length: bytes.len() as u64,
        }
    }

    #[test]
    fn gameinfo_startup_mounts_selected_world_and_preserves_mounts_on_failure() {
        let _guard = TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("source-abi-gameinfo-{}", std::process::id()));
        std::fs::create_dir_all(root.join("episode/maps")).unwrap();
        std::fs::write(
            root.join("episode/gameinfo.txt"),
            b"GameInfo { FileSystem { SearchPaths { game |gameinfo_path|. } } }",
        )
        .unwrap();
        std::fs::write(root.join("episode/maps/episode.bsp"), synthetic_world_bsp()).unwrap();
        let config = SourceAbiContextConfig {
            log: None,
            ..config()
        };
        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config, &mut handle) },
            SOURCE_ABI_OK
        );
        let base = slice(root.to_str().unwrap().as_bytes());
        let mut count = 99;
        assert_eq!(
            unsafe {
                source_context_mount_gameinfo(
                    handle,
                    base,
                    slice(b"episode"),
                    slice(b""),
                    ptr::null_mut(),
                )
            },
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(
            unsafe {
                source_context_mount_gameinfo(
                    handle,
                    base,
                    slice(b"episode"),
                    slice(b""),
                    &mut count,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(count, 1);
        assert_eq!(
            unsafe {
                source_context_mount_gameinfo(
                    handle,
                    base,
                    slice(b"missing"),
                    slice(b""),
                    &mut count,
                )
            },
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(count, 0);
        let mut world = SourceAbiWorldInfo::default();
        assert_eq!(
            unsafe {
                source_context_world_load(
                    handle,
                    slice(b"maps/episode.bsp"),
                    slice(b"GAME"),
                    &mut world,
                )
            },
            SOURCE_ABI_OK
        );
        assert!(world.leaf_count > 0);
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn owns_cvars_and_a_bounded_command_queue() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );

        assert_eq!(
            unsafe {
                source_context_cvar_register(handle, slice(b"Host_Name"), slice(b"lambda"), 0)
            },
            SOURCE_ABI_OK
        );
        let mut written = 0;
        let mut info = SourceAbiCvarInfo::default();
        assert_eq!(
            unsafe {
                source_context_cvar_get(
                    handle,
                    slice(b"host_name"),
                    SourceAbiMutSlice {
                        data: ptr::null_mut(),
                        length: 0,
                    },
                    &mut written,
                    &mut info,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(written, 6);
        assert_eq!(info.generation, 0);
        assert_eq!(
            unsafe { source_context_cvar_set(handle, slice(b"HOST_NAME"), slice(b"rust host")) },
            SOURCE_ABI_OK
        );
        let mut value = [0u8; 32];
        assert_eq!(
            unsafe {
                source_context_cvar_get(
                    handle,
                    slice(b"host_name"),
                    SourceAbiMutSlice {
                        data: value.as_mut_ptr(),
                        length: value.len() as u64,
                    },
                    &mut written,
                    &mut info,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&value[..written as usize], b"rust host");
        assert_eq!(info.generation, 1);

        assert_eq!(
            unsafe {
                source_context_cvar_register(
                    handle,
                    slice(b"build_id"),
                    slice(b"1"),
                    source_console::CVAR_READ_ONLY,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(
            unsafe { source_context_cvar_set(handle, slice(b"build_id"), slice(b"2")) },
            SOURCE_ABI_FORMAT_ERROR
        );

        let mut command_count = 0;
        assert_eq!(
            unsafe {
                source_context_command_enqueue(
                    handle,
                    slice(b"echo \"hello world\"; map d1_trainstation_01"),
                    &mut command_count,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(command_count, 2);
        assert_eq!(
            unsafe {
                source_context_command_pop(
                    handle,
                    SourceAbiMutSlice {
                        data: ptr::null_mut(),
                        length: 0,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(written, 18);
        let mut command = [0u8; 64];
        assert_eq!(
            unsafe {
                source_context_command_pop(
                    handle,
                    SourceAbiMutSlice {
                        data: command.as_mut_ptr(),
                        length: command.len() as u64,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&command[..written as usize], b"echo \"hello world\"");
        assert_eq!(
            unsafe {
                source_context_command_pop(
                    handle,
                    SourceAbiMutSlice {
                        data: command.as_mut_ptr(),
                        length: command.len() as u64,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&command[..written as usize], b"map d1_trainstation_01");
        assert_eq!(
            unsafe {
                source_context_command_pop(
                    handle,
                    SourceAbiMutSlice {
                        data: command.as_mut_ptr(),
                        length: command.len() as u64,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
    }

    #[test]
    fn owns_network_channel_sequence_and_drop_decisions() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );

        assert_eq!(
            unsafe { source_context_net_channel_create(handle, 1, 0, 0, ptr::null_mut()) },
            SOURCE_ABI_INVALID_ARGUMENT
        );
        let mut channel = 0;
        assert_eq!(
            unsafe { source_context_net_channel_create(handle, 1, 0, 0, &mut channel) },
            SOURCE_ABI_OK
        );
        assert_ne!(channel, 0);

        let mut advance = SourceAbiNetSequenceAdvance::default();
        assert_eq!(
            unsafe { source_context_net_channel_advance_outgoing(handle, channel, &mut advance) },
            SOURCE_ABI_OK
        );
        assert_eq!(
            advance,
            SourceAbiNetSequenceAdvance {
                previous: 1,
                current: 2,
            }
        );

        let challenge = 0xa1b2_c3d4u32;
        let mut encoded = SourceAbiNetEncodedPacketHeader::default();
        assert_eq!(
            unsafe {
                source_context_net_packet_encode_header(
                    handle,
                    17,
                    11,
                    0xa5,
                    1,
                    3,
                    1,
                    challenge,
                    1,
                    &mut encoded,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(encoded.length, 17);
        assert_eq!(encoded.flags_offset, 8);
        assert_eq!(encoded.checksum_offset, 9);
        assert_eq!(encoded.checksum_start, 11);
        let mut encoded_packet = encoded.bytes[..encoded.length as usize].to_vec();
        encoded_packet.extend_from_slice(b"payload");
        let mut finalized_checksum = 0u16;
        assert_eq!(
            unsafe {
                source_context_net_packet_finalize_header(
                    handle,
                    SourceAbiMutSlice {
                        data: encoded_packet.as_mut_ptr(),
                        length: encoded_packet.len() as u64,
                    },
                    encoded.base_flags | 1,
                    1,
                    &mut finalized_checksum,
                )
            },
            SOURCE_ABI_OK
        );
        assert_ne!(finalized_checksum, 0);
        assert_eq!(encoded_packet[8], (encoded.base_flags | 1) as u8);

        let mut packet = Vec::new();
        packet.extend_from_slice(&17i32.to_le_bytes());
        packet.extend_from_slice(&11i32.to_le_bytes());
        packet.push(source_net::PACKET_FLAG_CHOKED | source_net::PACKET_FLAG_CHALLENGE | 1);
        packet.extend_from_slice(&0u16.to_le_bytes());
        packet.push(0xa5);
        packet.push(3);
        packet.extend_from_slice(&challenge.to_le_bytes());
        packet.extend_from_slice(b"payload");
        let payload = SourceAbiSlice {
            data: packet[11..].as_ptr(),
            length: (packet.len() - 11) as u64,
        };
        let mut checksum = 0u16;
        assert_eq!(
            unsafe { source_context_net_packet_checksum(handle, payload, &mut checksum) },
            SOURCE_ABI_OK
        );
        packet[9..11].copy_from_slice(&checksum.to_le_bytes());
        let packet_slice = SourceAbiSlice {
            data: packet.as_ptr(),
            length: packet.len() as u64,
        };
        let mut header = SourceAbiNetPacketHeader::default();
        assert_eq!(
            unsafe {
                source_context_net_packet_header(handle, packet_slice, 1, 0, challenge, &mut header)
            },
            SOURCE_ABI_OK
        );
        assert_eq!(header.sequence, 17);
        assert_eq!(header.outgoing_ack, 11);
        assert_eq!(header.reliable_state, 0xa5);
        assert_eq!(header.choked, 3);
        assert_eq!(header.challenge, challenge);
        assert_eq!(header.header_bytes, 17);
        assert_eq!(header.accepted, 1);
        assert_eq!(header.reason, source_net::PACKET_HEADER_ACCEPTED);
        *packet.last_mut().unwrap() ^= 1;
        assert_eq!(
            unsafe {
                source_context_net_packet_header(
                    handle,
                    SourceAbiSlice {
                        data: packet.as_ptr(),
                        length: packet.len() as u64,
                    },
                    1,
                    0,
                    challenge,
                    &mut header,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(header.accepted, 0);
        assert_eq!(header.reason, source_net::PACKET_HEADER_CHECKSUM_MISMATCH);
        header.sequence = 99;
        assert_eq!(
            unsafe {
                source_context_net_packet_header(
                    handle,
                    SourceAbiSlice {
                        data: packet.as_ptr(),
                        length: packet.len() as u64,
                    },
                    2,
                    0,
                    challenge,
                    &mut header,
                )
            },
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(header, SourceAbiNetPacketHeader::default());

        let mut decision = SourceAbiNetPacketDecision::default();
        assert_eq!(
            unsafe {
                source_context_net_channel_preview_incoming(
                    handle,
                    channel,
                    5,
                    3,
                    1,
                    100,
                    &mut decision,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(decision.accepted, 1);
        assert_eq!(decision.dropped, 3);
        assert_eq!(decision.reason, source_net::PACKET_ACCEPTED);

        assert_eq!(
            unsafe {
                source_context_net_channel_commit_incoming(
                    handle,
                    channel,
                    5,
                    3,
                    1,
                    100,
                    &mut decision,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(decision.incoming_sequence, 5);
        assert_eq!(decision.outgoing_ack, 3);

        assert_eq!(
            unsafe {
                source_context_net_channel_preview_incoming(
                    handle,
                    channel,
                    5,
                    4,
                    0,
                    100,
                    &mut decision,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(decision.accepted, 0);
        assert_eq!(decision.reason, source_net::PACKET_DUPLICATE);

        assert_eq!(
            source_context_net_channel_reset(handle, channel, 20, 10, 9),
            SOURCE_ABI_OK
        );
        assert_eq!(
            source_context_net_channel_remove(handle, channel),
            SOURCE_ABI_OK
        );
        assert_eq!(
            source_context_net_channel_remove(handle, channel),
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(
            unsafe { source_context_net_channel_advance_outgoing(handle, channel, &mut advance) },
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(advance, SourceAbiNetSequenceAdvance::default());
        advance = SourceAbiNetSequenceAdvance {
            previous: 7,
            current: 8,
        };
        assert_eq!(
            unsafe { source_context_net_channel_advance_outgoing(handle, 0, &mut advance) },
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(advance, SourceAbiNetSequenceAdvance::default());
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
    }

    #[test]
    fn owns_network_string_table_contents_and_ticks() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );

        let mut table = 77;
        assert_eq!(
            unsafe {
                source_context_string_table_create(
                    handle,
                    slice(b"modelprecache"),
                    3,
                    1,
                    &mut table,
                )
            },
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(table, 0);
        assert_eq!(
            unsafe {
                source_context_string_table_create(
                    handle,
                    slice(b"modelprecache"),
                    4,
                    1,
                    &mut table,
                )
            },
            SOURCE_ABI_OK
        );
        assert_ne!(table, 0);
        assert_eq!(
            source_context_string_table_enable_history(handle, table),
            SOURCE_ABI_OK
        );

        let mut upsert = SourceAbiStringTableUpsert::default();
        assert_eq!(
            unsafe {
                source_context_string_table_upsert(
                    handle,
                    table,
                    slice(b"models/gman_high.mdl"),
                    1,
                    slice(b"one"),
                    &mut upsert,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(upsert.index, 0);
        assert_eq!(upsert.entry_count, 1);
        assert_eq!(upsert.created, 1);

        let mut found = u32::MAX;
        assert_eq!(
            unsafe {
                source_context_string_table_find(
                    handle,
                    table,
                    slice(b"MODELS/GMAN_HIGH.MDL"),
                    &mut found,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(found, 0);
        let mut written = 0;
        assert_eq!(
            unsafe {
                source_context_string_table_read_string(
                    handle,
                    table,
                    0,
                    SourceAbiMutSlice {
                        data: ptr::null_mut(),
                        length: 0,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(written, 20);
        let mut value = [0u8; 32];
        assert_eq!(
            unsafe {
                source_context_string_table_read_string(
                    handle,
                    table,
                    0,
                    SourceAbiMutSlice {
                        data: value.as_mut_ptr(),
                        length: value.len() as u64,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&value[..written as usize], b"models/gman_high.mdl");

        assert_eq!(
            source_context_string_table_set_tick(handle, table, 2),
            SOURCE_ABI_OK
        );
        let mut change = SourceAbiStringTableChange::default();
        assert_eq!(
            unsafe {
                source_context_string_table_set_user_data(
                    handle,
                    table,
                    0,
                    slice(b"two"),
                    &mut change,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(change.changed, 0);
        assert_eq!(change.tick_changed, 1);
        let mut changed = 0;
        assert_eq!(
            unsafe { source_context_string_table_changed_since(handle, table, 1, &mut changed) },
            SOURCE_ABI_OK
        );
        assert_eq!(changed, 0);
        let mut last_changed_tick = 0;
        assert_eq!(
            unsafe {
                source_context_string_table_restore_tick(handle, table, 1, &mut last_changed_tick)
            },
            SOURCE_ABI_OK
        );
        assert_eq!(last_changed_tick, 1);
        let mut user_data = [0u8; 8];
        assert_eq!(
            unsafe {
                source_context_string_table_read_user_data(
                    handle,
                    table,
                    0,
                    SourceAbiMutSlice {
                        data: user_data.as_mut_ptr(),
                        length: user_data.len() as u64,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&user_data[..written as usize], b"one");

        assert_eq!(
            source_context_string_table_clear(handle, table),
            SOURCE_ABI_OK
        );
        let mut entry_count = 99;
        assert_eq!(
            unsafe { source_context_string_table_entry_count(handle, table, &mut entry_count) },
            SOURCE_ABI_OK
        );
        assert_eq!(entry_count, 0);
        assert_eq!(
            source_context_string_table_remove(handle, table),
            SOURCE_ABI_OK
        );
        assert_eq!(
            source_context_string_table_remove(handle, table),
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
    }

    #[test]
    fn owns_server_datatable_schema_and_class_ids() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );

        let mut base = SourceAbiDataTableRegistration::default();
        assert_eq!(
            unsafe {
                source_context_data_table_register(handle, slice(b"DT_BaseEntity"), 1, &mut base)
            },
            SOURCE_ABI_OK
        );
        assert_eq!(base.created, 1);
        let base_property = SourceAbiDataTableProperty {
            name: slice(b"m_iTeamNum"),
            reference_name: slice(b""),
            property_type: 0,
            flags: 1,
            bit_count: 8,
            elements: 1,
            low_value: 0.0,
            high_value: 255.0,
        };
        assert_eq!(
            unsafe {
                source_context_data_table_register_property(handle, base.table_id, &base_property)
            },
            SOURCE_ABI_OK
        );

        let mut derived = SourceAbiDataTableRegistration::default();
        assert_eq!(
            unsafe {
                source_context_data_table_register(handle, slice(b"DT_TestEntity"), 1, &mut derived)
            },
            SOURCE_ABI_OK
        );
        let derived_property = SourceAbiDataTableProperty {
            name: slice(b"baseclass"),
            reference_name: slice(b"DT_BaseEntity"),
            property_type: source_net::SEND_PROP_DATA_TABLE,
            flags: 1 << 12,
            bit_count: 0,
            elements: 1,
            low_value: 0.0,
            high_value: 0.0,
        };
        assert_eq!(
            unsafe {
                source_context_data_table_register_property(
                    handle,
                    derived.table_id,
                    &derived_property,
                )
            },
            SOURCE_ABI_OK
        );

        let mut class_id = u32::MAX;
        assert_eq!(
            unsafe {
                source_context_server_class_register(
                    handle,
                    slice(b"CTestEntity"),
                    derived.table_id,
                    &mut class_id,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(class_id, 0);
        let mut summary = SourceAbiDataTableSummary::default();
        assert_eq!(
            unsafe { source_context_data_table_finalize(handle, 0xf7bb_8e46, &mut summary) },
            SOURCE_ABI_OK
        );
        assert_eq!(summary.table_count, 2);
        assert_eq!(summary.property_count, 2);
        assert_eq!(summary.class_count, 1);
        assert_eq!(summary.compatibility_crc, 0xf7bb_8e46);

        class_id = u32::MAX;
        assert_eq!(
            unsafe {
                source_context_server_class_find(handle, slice(b"ctestentity"), &mut class_id)
            },
            SOURCE_ABI_OK
        );
        assert_eq!(class_id, 0);

        let mut queued = 0;
        assert_eq!(
            unsafe { source_context_snapshot_queue_delete(handle, 3, &mut queued) },
            SOURCE_ABI_OK
        );
        assert_eq!(queued, 1);
        assert_eq!(
            unsafe { source_context_snapshot_queue_delete(handle, 3, &mut queued) },
            SOURCE_ABI_OK
        );
        assert_eq!(queued, 0);
        let entities = [
            SourceAbiSnapshotEntity {
                entity_index: 0,
                serial_number: 11,
                class_id: 0,
                reserved: 0,
            },
            SourceAbiSnapshotEntity {
                entity_index: 7,
                serial_number: 23,
                class_id: 0,
                reserved: 0,
            },
        ];
        let mut snapshot_id = 0;
        let mut snapshot_summary = SourceAbiSnapshotSummary::default();
        assert_eq!(
            unsafe {
                source_context_snapshot_create(
                    handle,
                    42,
                    8,
                    entities.as_ptr(),
                    entities.len() as u64,
                    &mut snapshot_id,
                    &mut snapshot_summary,
                )
            },
            SOURCE_ABI_OK
        );
        assert_ne!(snapshot_id, 0);
        assert_eq!(
            snapshot_summary,
            SourceAbiSnapshotSummary {
                tick: 42,
                max_entities: 8,
                valid_entity_count: 2,
                explicit_delete_count: 1,
            }
        );
        let mut entity = SourceAbiSnapshotEntity::default();
        assert_eq!(
            unsafe { source_context_snapshot_entity_at(handle, snapshot_id, 1, &mut entity) },
            SOURCE_ABI_OK
        );
        assert_eq!(entity, entities[1]);
        let mut delete_slot = u32::MAX;
        assert_eq!(
            unsafe { source_context_snapshot_delete_at(handle, snapshot_id, 0, &mut delete_slot) },
            SOURCE_ABI_OK
        );
        assert_eq!(delete_slot, 3);
        snapshot_summary = SourceAbiSnapshotSummary::default();
        assert_eq!(
            unsafe { source_context_snapshot_summary(handle, snapshot_id, &mut snapshot_summary) },
            SOURCE_ABI_OK
        );
        assert_eq!(snapshot_summary.valid_entity_count, 2);

        let invalid_class = SourceAbiSnapshotEntity {
            entity_index: 0,
            serial_number: 1,
            class_id: 1,
            reserved: 0,
        };
        let mut invalid_id = 99;
        assert_eq!(
            unsafe {
                source_context_snapshot_create(
                    handle,
                    43,
                    8,
                    &invalid_class,
                    1,
                    &mut invalid_id,
                    &mut snapshot_summary,
                )
            },
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(invalid_id, 0);
        assert_eq!(
            source_context_snapshot_remove(handle, snapshot_id),
            SOURCE_ABI_OK
        );
        assert_eq!(
            source_context_snapshot_remove(handle, snapshot_id),
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(source_context_snapshot_clear(handle), SOURCE_ABI_OK);
        assert_eq!(source_context_data_table_clear(handle), SOURCE_ABI_OK);
        assert_eq!(
            unsafe { source_context_data_table_summary(handle, &mut summary) },
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(summary, SourceAbiDataTableSummary::default());
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
    }

    #[test]
    fn ten_thousand_full_lifecycles_and_panic_containment() {
        let _guard = TEST_LOCK.lock().unwrap();
        let before = source_context_live_count();
        CALLBACK_COUNT.store(0, Ordering::Relaxed);

        for iteration in 0u32..10_000 {
            let mut handle = 0;
            assert_eq!(
                unsafe { source_context_create(&config(), &mut handle) },
                SOURCE_ABI_OK
            );
            assert_ne!(handle, 0);

            let input = iteration.to_le_bytes();
            let mut output = [0u8; 4];
            let mut written = 0;
            assert_eq!(
                unsafe {
                    source_context_echo(
                        handle,
                        SourceAbiSlice {
                            data: input.as_ptr(),
                            length: input.len() as u64,
                        },
                        SourceAbiMutSlice {
                            data: output.as_mut_ptr(),
                            length: output.len() as u64,
                        },
                        &mut written,
                    )
                },
                SOURCE_ABI_OK
            );
            assert_eq!(written, 4);
            assert_eq!(output, input);

            if iteration == 0 {
                assert_eq!(
                    unsafe {
                        source_context_emit_log(
                            handle,
                            7,
                            SourceAbiSlice {
                                data: b"hello".as_ptr(),
                                length: 5,
                            },
                        )
                    },
                    SOURCE_ABI_OK
                );
                assert_eq!(
                    source_context_force_panic_for_test(handle),
                    SOURCE_ABI_PANIC
                );
                // The context remains valid after the contained panic.
                assert_eq!(source_context_live_count(), before + 1);
            }

            assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
            assert_eq!(source_context_destroy(handle), SOURCE_ABI_INVALID_HANDLE);
        }

        assert_eq!(source_context_live_count(), before);
        assert_eq!(CALLBACK_COUNT.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn rejects_versions_nulls_and_short_buffers() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut handle = 123;
        let mut bad_config = config();
        bad_config.abi_version += 1;
        assert_eq!(
            unsafe { source_context_create(&bad_config, &mut handle) },
            SOURCE_ABI_UNSUPPORTED_VERSION
        );
        assert_eq!(handle, 0);
        assert_eq!(
            unsafe { source_context_create(ptr::null(), &mut handle) },
            SOURCE_ABI_INVALID_ARGUMENT
        );

        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );
        let mut output = [0u8; 2];
        let mut written = 0;
        assert_eq!(
            unsafe {
                source_context_echo(
                    handle,
                    SourceAbiSlice {
                        data: b"long".as_ptr(),
                        length: 4,
                    },
                    SourceAbiMutSlice {
                        data: output.as_mut_ptr(),
                        length: output.len() as u64,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(written, 4);

        let root = b"/tmp/game";
        let virtual_path = b"hl2/../escape";
        assert_eq!(
            unsafe {
                source_context_resolve_content_path(
                    handle,
                    SourceAbiSlice {
                        data: root.as_ptr(),
                        length: root.len() as u64,
                    },
                    SourceAbiSlice {
                        data: virtual_path.as_ptr(),
                        length: virtual_path.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: ptr::null_mut(),
                        length: 0,
                    },
                    &mut written,
                )
            },
            SOURCE_ABI_FORMAT_ERROR
        );
        assert_eq!(written, 0);

        let mut phase = u32::MAX;
        assert_eq!(
            unsafe { source_context_host_phase(handle, &mut phase) },
            SOURCE_ABI_OK
        );
        assert_eq!(phase, source_host::Phase::Created as u32);
        assert_eq!(
            source_context_host_transition(handle, source_host::Phase::ContentReady as u32),
            SOURCE_ABI_FORMAT_ERROR
        );
        assert_eq!(
            source_context_host_transition(handle, u32::MAX),
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(
            source_context_host_transition(handle, source_host::Phase::LauncherReady as u32),
            SOURCE_ABI_OK
        );
        assert_eq!(
            source_context_host_configure_scheduler(handle, 0, 4),
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(
            source_context_host_configure_scheduler(handle, 15_000_000, 4),
            SOURCE_ABI_OK
        );
        let mut plan = SourceAbiFramePlan::default();
        assert_eq!(
            unsafe { source_context_host_advance(handle, 1_000, &mut plan) },
            SOURCE_ABI_OK
        );
        assert_eq!(plan.tick_count, 0);
        assert_eq!(
            unsafe { source_context_host_advance(handle, 22_501_000, &mut plan) },
            SOURCE_ABI_OK
        );
        assert_eq!(plan.tick_count, 1);
        assert_eq!(plan.interpolation_numerator_ns, 7_500_000);
        assert_eq!(plan.interpolation_denominator_ns, 15_000_000);
        assert_eq!(plan.dropped_ns, 0);

        let mut pace = SourceAbiFramePace::default();
        assert_eq!(
            unsafe { source_context_host_pace(handle, 1_000, 15_000_000, &mut pace) },
            SOURCE_ABI_OK
        );
        assert_eq!(
            (pace.ready, pace.elapsed_ns, pace.wait_ns),
            (1, 15_000_000, 0)
        );
        assert_eq!(
            unsafe { source_context_host_pace(handle, 10_001_000, 15_000_000, &mut pace) },
            SOURCE_ABI_OK
        );
        assert_eq!(
            (pace.ready, pace.elapsed_ns, pace.wait_ns),
            (0, 10_000_000, 5_000_000)
        );
        let target = b"d1_trainstation_01";
        assert_eq!(
            unsafe {
                source_context_host_request_operation(
                    handle,
                    source_host::HostOperationKind::NewGame as u32,
                    SourceAbiSlice {
                        data: target.as_ptr(),
                        length: target.len() as u64,
                    },
                    SourceAbiSlice {
                        data: ptr::null(),
                        length: 0,
                    },
                    0,
                )
            },
            SOURCE_ABI_FORMAT_ERROR
        );
        assert_eq!(
            source_context_host_transition(handle, source_host::Phase::ContentReady as u32),
            SOURCE_ABI_OK
        );
        assert_eq!(
            source_context_host_transition(handle, source_host::Phase::LegacyRunning as u32),
            SOURCE_ABI_OK
        );
        assert_eq!(
            unsafe {
                source_context_host_request_operation(
                    handle,
                    source_host::HostOperationKind::NewGame as u32,
                    SourceAbiSlice {
                        data: target.as_ptr(),
                        length: target.len() as u64,
                    },
                    SourceAbiSlice {
                        data: ptr::null(),
                        length: 0,
                    },
                    0,
                )
            },
            SOURCE_ABI_OK
        );
        let mut pending_kind = 0;
        assert_eq!(
            unsafe { source_context_host_pending_operation(handle, &mut pending_kind) },
            SOURCE_ABI_OK
        );
        assert_eq!(pending_kind, source_host::HostOperationKind::NewGame as u32);
        let mut target_output = [0u8; 64];
        let mut landmark_output = [0u8; 64];
        let mut operation_info = SourceAbiHostOperationInfo::default();
        assert_eq!(
            unsafe {
                source_context_host_take_operation(
                    handle,
                    SourceAbiMutSlice {
                        data: target_output.as_mut_ptr(),
                        length: target_output.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: landmark_output.as_mut_ptr(),
                        length: landmark_output.len() as u64,
                    },
                    &mut operation_info,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(operation_info.target_length, target.len() as u64);
        assert_eq!(&target_output[..target.len()], target);
        assert_eq!(
            unsafe { source_context_host_pending_operation(handle, &mut pending_kind) },
            SOURCE_ABI_NOT_FOUND
        );
        let mut tick_plan = SourceAbiTickPlan::default();
        assert_eq!(
            unsafe {
                source_context_host_schedule_ticks(handle, 0.010, 0.015, 100, 1, 0, &mut tick_plan)
            },
            SOURCE_ABI_OK
        );
        assert_eq!(tick_plan.tick_count, 0);
        assert!((tick_plan.remainder - 0.010).abs() < 1.0e-12);
        assert_eq!(
            unsafe {
                source_context_host_schedule_ticks(handle, 0.020, 0.015, 100, 1, 1, &mut tick_plan)
            },
            SOURCE_ABI_OK
        );
        assert_eq!(tick_plan.tick_count, 2);
        assert!(tick_plan.remainder.abs() < 1.0e-12);

        let mut modifier_mask = 0;
        assert_eq!(
            unsafe {
                source_context_input_modifier(
                    handle,
                    source_input::MOD_LEFT_SHIFT,
                    1,
                    &mut modifier_mask,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(modifier_mask, 1 << 1);
        let mut button_mask = 0;
        assert_eq!(
            unsafe { source_context_input_mouse_button(handle, 1 << 2, 1, &mut button_mask) },
            SOURCE_ABI_OK
        );
        assert_eq!(button_mask, 1 << 2);
        assert_eq!(
            source_context_input_mouse_motion(handle, 12, -7),
            SOURCE_ABI_OK
        );
        let (mut delta_x, mut delta_y) = (0, 0);
        assert_eq!(
            unsafe { source_context_input_take_mouse_delta(handle, &mut delta_x, &mut delta_y) },
            SOURCE_ABI_OK
        );
        assert_eq!((delta_x, delta_y), (12, -7));
        assert_eq!(source_context_input_reset(handle), SOURCE_ABI_OK);
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
    }

    #[test]
    fn probes_and_reads_vpk_without_crossing_allocator_ownership() {
        let _guard = TEST_LOCK.lock().unwrap();
        let mut scene = vec![0u8; 58];
        scene[0..4].copy_from_slice(b"VSIF");
        scene[4..8].copy_from_slice(&2i32.to_le_bytes());
        scene[8..12].copy_from_slice(&1i32.to_le_bytes());
        scene[12..16].copy_from_slice(&1i32.to_le_bytes());
        scene[16..20].copy_from_slice(&30i32.to_le_bytes());
        scene[20..24].copy_from_slice(&24u32.to_le_bytes());
        scene[24..30].copy_from_slice(b"sound\0");
        scene[30..34].copy_from_slice(&10u32.to_le_bytes());
        scene[34..38].copy_from_slice(&58i32.to_le_bytes());
        scene[38..42].copy_from_slice(&9i32.to_le_bytes());
        scene[42..46].copy_from_slice(&46i32.to_le_bytes());
        scene[46..50].copy_from_slice(&1000u32.to_le_bytes());
        scene[50..54].copy_from_slice(&1i32.to_le_bytes());
        scene[54..58].copy_from_slice(&0u32.to_le_bytes());
        scene.extend_from_slice(b"bvcd\x04\x7b\0\0\0");
        let mut builder = source_vpk::Builder::new();
        builder
            .add_file("test/hello.txt", b"hello".to_vec())
            .unwrap();
        builder.add_file("scenes/scenes.image", scene).unwrap();
        let bytes = builder.build_v1_embedded().unwrap();
        let path =
            std::env::temp_dir().join(format!("source-abi-vpk-probe-{}.vpk", std::process::id()));
        std::fs::write(&path, bytes).unwrap();

        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );
        let path_bytes = path.to_str().unwrap().as_bytes();
        let mut entry_count = 0;
        let mut version = 0;
        let status = unsafe {
            source_context_probe_vpk(
                handle,
                SourceAbiSlice {
                    data: path_bytes.as_ptr(),
                    length: path_bytes.len() as u64,
                },
                &mut entry_count,
                &mut version,
            )
        };
        assert_eq!(status, SOURCE_ABI_OK);
        assert_eq!(entry_count, 2);
        assert_eq!(version, source_vpk::VPK_VERSION_1);

        let entry = b"test/hello.txt";
        let mut required = 0;
        assert_eq!(
            unsafe {
                source_context_read_vpk_file(
                    handle,
                    SourceAbiSlice {
                        data: path_bytes.as_ptr(),
                        length: path_bytes.len() as u64,
                    },
                    SourceAbiSlice {
                        data: entry.as_ptr(),
                        length: entry.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: ptr::null_mut(),
                        length: 0,
                    },
                    &mut required,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(required, 5);
        let mut contents = [0u8; 5];
        assert_eq!(
            unsafe {
                source_context_read_vpk_file(
                    handle,
                    SourceAbiSlice {
                        data: path_bytes.as_ptr(),
                        length: path_bytes.len() as u64,
                    },
                    SourceAbiSlice {
                        data: entry.as_ptr(),
                        length: entry.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: contents.as_mut_ptr(),
                        length: contents.len() as u64,
                    },
                    &mut required,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&contents, b"hello");

        let scene_entry = b"scenes/scenes.image";
        let mut scene_count = 0;
        let mut string_count = 0;
        assert_eq!(
            unsafe {
                source_context_probe_vpk_scene(
                    handle,
                    SourceAbiSlice {
                        data: path_bytes.as_ptr(),
                        length: path_bytes.len() as u64,
                    },
                    SourceAbiSlice {
                        data: scene_entry.as_ptr(),
                        length: scene_entry.len() as u64,
                    },
                    &mut scene_count,
                    &mut string_count,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(scene_count, 1);
        assert_eq!(string_count, 1);

        let path_id = b"GAME";
        assert_eq!(
            unsafe {
                source_context_mount_vpk(
                    handle,
                    SourceAbiSlice {
                        data: path_bytes.as_ptr(),
                        length: path_bytes.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    0,
                )
            },
            SOURCE_ABI_OK
        );
        contents.fill(0);
        let mut mounted_size = 0;
        assert_eq!(
            unsafe {
                source_context_file_size(
                    handle,
                    SourceAbiSlice {
                        data: entry.as_ptr(),
                        length: entry.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    &mut mounted_size,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(mounted_size, 5);
        // Rooted where the archive was mounted rather than at its real path,
        // which is what lets the engine match the result to a search path.
        let expected_resolved = format!("{}/test/hello.txt", path.to_str().unwrap());
        let mut resolved_kind = u32::MAX;
        let mut resolved_size = 0;
        assert_eq!(
            unsafe {
                source_context_resolve_read_path(
                    handle,
                    SourceAbiSlice {
                        data: entry.as_ptr(),
                        length: entry.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: ptr::null_mut(),
                        length: 0,
                    },
                    &mut resolved_size,
                    &mut resolved_kind,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(resolved_size, expected_resolved.len() as u64);
        assert_eq!(resolved_kind, SOURCE_READ_PATH_VPK);
        let mut resolved_output = vec![0u8; resolved_size as usize];
        assert_eq!(
            unsafe {
                source_context_resolve_read_path(
                    handle,
                    SourceAbiSlice {
                        data: entry.as_ptr(),
                        length: entry.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: resolved_output.as_mut_ptr(),
                        length: resolved_output.len() as u64,
                    },
                    &mut resolved_size,
                    &mut resolved_kind,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(resolved_output, expected_resolved.as_bytes());
        assert_eq!(
            unsafe {
                source_context_read_file(
                    handle,
                    SourceAbiSlice {
                        data: entry.as_ptr(),
                        length: entry.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: contents.as_mut_ptr(),
                        length: contents.len() as u64,
                    },
                    &mut required,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&contents, b"hello");
        let mut mounted_file = 0;
        let mut mounted_file_size = 0;
        assert_eq!(
            unsafe {
                source_context_file_open_read(
                    handle,
                    SourceAbiSlice {
                        data: entry.as_ptr(),
                        length: entry.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    &mut mounted_file,
                    &mut mounted_file_size,
                )
            },
            SOURCE_ABI_OK
        );
        assert_ne!(mounted_file, 0);
        assert_eq!(mounted_file_size, 5);
        contents.fill(0);
        required = 0;
        assert_eq!(
            unsafe {
                source_context_file_read(
                    handle,
                    mounted_file,
                    SourceAbiMutSlice {
                        data: contents.as_mut_ptr(),
                        length: contents.len() as u64,
                    },
                    &mut required,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(required, 5);
        assert_eq!(&contents, b"hello");
        assert_eq!(
            source_context_file_close(handle, mounted_file),
            SOURCE_ABI_OK
        );
        let wildcard = b"test/*";
        let mut find = 99;
        let mut is_directory = 99;
        let mut find_name_size = 99;
        let mut tiny_name = [0u8; 1];
        assert_eq!(
            unsafe {
                source_context_find_first(
                    handle,
                    SourceAbiSlice {
                        data: wildcard.as_ptr(),
                        length: wildcard.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: tiny_name.as_mut_ptr(),
                        length: tiny_name.len() as u64,
                    },
                    &mut find_name_size,
                    &mut is_directory,
                    &mut find,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(find, 0);
        assert_eq!(find_name_size, 9);
        let mut find_name = [0u8; 32];
        assert_eq!(
            unsafe {
                source_context_find_first(
                    handle,
                    SourceAbiSlice {
                        data: wildcard.as_ptr(),
                        length: wildcard.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: find_name.as_mut_ptr(),
                        length: find_name.len() as u64,
                    },
                    &mut find_name_size,
                    &mut is_directory,
                    &mut find,
                )
            },
            SOURCE_ABI_OK
        );
        assert_ne!(find, 0);
        assert_eq!(&find_name[..find_name_size as usize], b"hello.txt");
        assert_eq!(is_directory, 0);
        assert_eq!(
            unsafe {
                source_context_find_next(
                    handle,
                    find,
                    SourceAbiMutSlice {
                        data: find_name.as_mut_ptr(),
                        length: find_name.len() as u64,
                    },
                    &mut find_name_size,
                    &mut is_directory,
                )
            },
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(find_name_size, 0);
        assert_eq!(source_context_find_close(handle, find), SOURCE_ABI_OK);
        assert_eq!(
            source_context_find_close(handle, find),
            SOURCE_ABI_NOT_FOUND
        );
        let root_wildcard = b"*";
        assert_eq!(
            unsafe {
                source_context_find_first(
                    handle,
                    SourceAbiSlice {
                        data: root_wildcard.as_ptr(),
                        length: root_wildcard.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    SourceAbiMutSlice {
                        data: find_name.as_mut_ptr(),
                        length: find_name.len() as u64,
                    },
                    &mut find_name_size,
                    &mut is_directory,
                    &mut find,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&find_name[..find_name_size as usize], b"scenes");
        assert_eq!(is_directory, 1);
        assert_eq!(
            unsafe {
                source_context_find_next(
                    handle,
                    find,
                    SourceAbiMutSlice {
                        data: tiny_name.as_mut_ptr(),
                        length: tiny_name.len() as u64,
                    },
                    &mut find_name_size,
                    &mut is_directory,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(find_name_size, 4);
        assert_eq!(
            unsafe {
                source_context_find_next(
                    handle,
                    find,
                    SourceAbiMutSlice {
                        data: find_name.as_mut_ptr(),
                        length: find_name.len() as u64,
                    },
                    &mut find_name_size,
                    &mut is_directory,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(&find_name[..find_name_size as usize], b"test");
        assert_eq!(is_directory, 1);
        assert_eq!(source_context_find_close(handle, find), SOURCE_ABI_OK);
        scene_count = 0;
        string_count = 0;
        assert_eq!(
            unsafe {
                source_context_probe_scene(
                    handle,
                    SourceAbiSlice {
                        data: scene_entry.as_ptr(),
                        length: scene_entry.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    &mut scene_count,
                    &mut string_count,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!((scene_count, string_count), (1, 1));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
    }

    #[test]
    fn validates_mounted_demo_and_save_artifacts() {
        let _guard = TEST_LOCK.lock().unwrap();
        let root =
            std::env::temp_dir().join(format!("source-abi-serialization-{}", std::process::id()));
        let save_directory = root.join("save");
        std::fs::create_dir_all(&save_directory).unwrap();
        let demo_path = root.join("sample.dem");
        std::fs::write(&demo_path, synthetic_demo()).unwrap();
        std::fs::write(save_directory.join("sample.sav"), synthetic_save()).unwrap();

        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );
        let root_bytes = root.to_str().unwrap().as_bytes();
        let path_id = b"GAME";
        assert_eq!(
            unsafe { source_context_mount_directory(handle, slice(root_bytes), slice(path_id), 1) },
            SOURCE_ABI_OK
        );

        let mut demo_info = SourceAbiDemoInfo::default();
        assert_eq!(
            unsafe {
                source_context_demo_validate(
                    handle,
                    slice(b"sample.dem"),
                    slice(path_id),
                    &mut demo_info,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(
            (
                demo_info.command_count,
                demo_info.packet_count,
                demo_info.playback_ticks,
                demo_info.network_protocol,
            ),
            (1, 0, 42, source_demo::NETWORK_PROTOCOL)
        );

        let mut save_info = SourceAbiSaveInfo::default();
        assert_eq!(
            unsafe {
                source_context_save_validate(
                    handle,
                    slice(b"save/sample.sav"),
                    slice(path_id),
                    &mut save_info,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(
            (
                save_info.embedded_file_count,
                save_info.embedded_map_state_count,
                save_info.token_count,
            ),
            (1, 1, 0)
        );

        std::fs::write(&demo_path, b"truncated").unwrap();
        assert_eq!(
            unsafe {
                source_context_demo_validate(
                    handle,
                    slice(b"sample.dem"),
                    slice(path_id),
                    &mut demo_info,
                )
            },
            SOURCE_ABI_FORMAT_ERROR
        );
        assert_eq!(demo_info.command_count, 0);

        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loads_and_queries_owned_bsp_world() {
        let _guard = TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!("source-abi-world-{}", std::process::id()));
        let maps = root.join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::write(maps.join("test.bsp"), synthetic_world_bsp()).unwrap();

        let mut handle = 0;
        assert_eq!(
            unsafe { source_context_create(&config(), &mut handle) },
            SOURCE_ABI_OK
        );
        let mut leaf = SourceAbiWorldLeaf::default();
        assert_eq!(
            unsafe { source_context_world_point_leaf(handle, 1.0, 0.0, 0.0, &mut leaf) },
            SOURCE_ABI_NOT_FOUND
        );
        let root_bytes = root.to_str().unwrap().as_bytes();
        let path_id = b"GAME";
        assert_eq!(
            unsafe {
                source_context_mount_directory(
                    handle,
                    SourceAbiSlice {
                        data: root_bytes.as_ptr(),
                        length: root_bytes.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    1,
                )
            },
            SOURCE_ABI_OK
        );
        let virtual_path = b"maps/test.bsp";
        let mut info = SourceAbiWorldInfo::default();
        assert_eq!(
            unsafe {
                source_context_world_load(
                    handle,
                    SourceAbiSlice {
                        data: virtual_path.as_ptr(),
                        length: virtual_path.len() as u64,
                    },
                    SourceAbiSlice {
                        data: path_id.as_ptr(),
                        length: path_id.len() as u64,
                    },
                    &mut info,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(
            (
                info.plane_count,
                info.node_count,
                info.leaf_count,
                info.cluster_count
            ),
            (1, 1, 2, 2)
        );
        assert_eq!(
            unsafe { source_context_world_point_leaf(handle, -1.0, 0.0, 0.0, &mut leaf) },
            SOURCE_ABI_OK
        );
        assert_eq!((leaf.leaf_index, leaf.cluster), (1, 1));

        let mut required = 0;
        assert_eq!(
            unsafe {
                source_context_world_visibility(
                    handle,
                    0,
                    source_bsp::VisibilityKind::PotentiallyVisible as u32,
                    SourceAbiMutSlice {
                        data: ptr::null_mut(),
                        length: 0,
                    },
                    &mut required,
                )
            },
            SOURCE_ABI_BUFFER_TOO_SMALL
        );
        assert_eq!(required, 1);
        let mut row = [0u8; 1];
        assert_eq!(
            unsafe {
                source_context_world_visibility(
                    handle,
                    0,
                    source_bsp::VisibilityKind::PotentiallyVisible as u32,
                    SourceAbiMutSlice {
                        data: row.as_mut_ptr(),
                        length: row.len() as u64,
                    },
                    &mut required,
                )
            },
            SOURCE_ABI_OK
        );
        assert_eq!(row, [0b01]);
        assert_eq!(source_context_world_clear(handle), SOURCE_ABI_OK);
        assert_eq!(
            unsafe { source_context_world_point_leaf(handle, 1.0, 0.0, 0.0, &mut leaf) },
            SOURCE_ABI_NOT_FOUND
        );
        assert_eq!(source_context_destroy(handle), SOURCE_ABI_OK);
        std::fs::remove_dir_all(root).unwrap();
    }
}
