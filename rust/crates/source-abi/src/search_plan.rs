//! Context-independent ownership for native read iterators, including startup.
use super::*;
use source_filesystem::search_plan::{Candidate, Filter, SearchPlan, StoreVisits};

#[repr(C)]
pub struct SourceAbiSearchPath {
    pub path_id: SourceAbiSlice,
    pub store_id: i32,
    /// 1 pack, 2 map, 4 by-request-only, 8 platform-excluded.
    pub flags: u32,
}

enum State {
    Plan(SearchPlan),
    Visits(StoreVisits),
}

fn states() -> &'static Mutex<HashMap<u64, State>> {
    static STATES: OnceLock<Mutex<HashMap<u64, State>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn insert(state: State) -> Result<u64, SourceAbiStatus> {
    let handle = NEXT_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| SOURCE_ABI_INTERNAL_ERROR)?;
    states()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(handle, state);
    Ok(handle)
}

#[no_mangle]
/// Snapshots selected, deduplicated source indices; no native memory retained.
/// Maximum 65536 paths, 4096 UTF-8 bytes per ID. Filter: 0 all, 1 cull ZIP/BSP,
/// 2 cull non-ZIP/BSP (VPK counts as non-pack, matching the native interface).
///
/// # Safety
/// Input array/slices are readable for the call; out_handle is writable.
pub unsafe extern "C" fn source_search_plan_create(
    paths: *const SourceAbiSearchPath,
    count: u32,
    requested: SourceAbiSlice,
    has_requested: u8,
    filter: u32,
    out_handle: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_handle.write(0) };
        if count > 65536
            || (count != 0 && paths.is_null())
            || has_requested > 1
            || requested.length > 4096
            || (has_requested == 0 && requested.length != 0)
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let filter = match filter {
            0 => Filter::All,
            1 => Filter::CullPack,
            2 => Filter::CullNonPack,
            _ => return SOURCE_ABI_INVALID_ARGUMENT,
        };
        let requested = if has_requested == 0 {
            None
        } else {
            match unsafe { read_slice(requested) }
                .and_then(|b| std::str::from_utf8(b).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
            {
                Ok(id) => Some(id),
                Err(status) => return status,
            }
        };
        let paths = if count == 0 {
            &[]
        } else {
            unsafe { slice::from_raw_parts(paths, count as usize) }
        };
        let mut candidates = Vec::with_capacity(paths.len());
        for path in paths {
            if path.flags & !15 != 0
                || (path.flags & 2 != 0 && path.flags & 1 == 0)
                || path.path_id.length > 4096
            {
                return SOURCE_ABI_INVALID_ARGUMENT;
            }
            let path_id = match unsafe { read_slice(path.path_id) }
                .and_then(|b| std::str::from_utf8(b).map_err(|_| SOURCE_ABI_INVALID_ARGUMENT))
            {
                Ok(id) => id,
                Err(status) => return status,
            };
            candidates.push(Candidate {
                path_id,
                store_id: path.store_id,
                pack: path.flags & 1 != 0,
                map: path.flags & 2 != 0,
                by_request_only: path.flags & 4 != 0,
                excluded: path.flags & 8 != 0,
            });
        }
        match insert(State::Plan(SearchPlan::new(&candidates, requested, filter))) {
            Ok(handle) => {
                unsafe { out_handle.write(handle) };
                SOURCE_ABI_OK
            }
            Err(status) => status,
        }
    })
}

#[no_mangle]
/// Creates an independent empty store-ID visit set.
/// # Safety
/// out_handle points to writable storage.
pub unsafe extern "C" fn source_search_visits_create(out_handle: *mut u64) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_handle.write(0) };
        match insert(State::Visits(StoreVisits::default())) {
            Ok(handle) => {
                unsafe { out_handle.write(handle) };
                SOURCE_ABI_OK
            }
            Err(status) => status,
        }
    })
}

#[no_mangle]
/// Returns a source index, or NOT_FOUND with u32::MAX after exhaustion.
/// # Safety
/// out_index points to writable storage.
pub unsafe extern "C" fn source_search_plan_next(
    handle: u64,
    out_index: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_index.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_index.write(u32::MAX) };
        let mut states = states().lock().unwrap_or_else(|e| e.into_inner());
        let Some(State::Plan(plan)) = states.get_mut(&handle) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };
        match plan.next() {
            Some(index) => {
                unsafe { out_index.write(index) };
                SOURCE_ABI_OK
            }
            None => SOURCE_ABI_NOT_FOUND,
        }
    })
}

#[no_mangle]
/// Marks a signed opaque store ID; returns 1 if already visited, 0 if new.
/// Invalid input fails closed (out_seen=1). Only accepts a visits handle.
/// # Safety
/// out_seen points to writable storage.
pub unsafe extern "C" fn source_search_visits_mark(
    handle: u64,
    store: i32,
    out_seen: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_seen.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_seen.write(1) };
        let mut states = states().lock().unwrap_or_else(|e| e.into_inner());
        let Some(State::Visits(visits)) = states.get_mut(&handle) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };
        unsafe { out_seen.write(u32::from(visits.mark(store))) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Rewinds an immutable plan or clears an independent store visit set.
pub extern "C" fn source_search_state_reset(handle: u64) -> SourceAbiStatus {
    ffi_status(|| {
        let mut states = states().lock().unwrap_or_else(|e| e.into_inner());
        match states.get_mut(&handle) {
            Some(State::Plan(plan)) => plan.reset(),
            Some(State::Visits(visits)) => visits.reset(),
            None => return SOURCE_ABI_INVALID_HANDLE,
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Releases either state exactly once. Missing/stale handles are invalid.
pub extern "C" fn source_search_state_destroy(handle: u64) -> SourceAbiStatus {
    ffi_status(|| {
        if states()
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
