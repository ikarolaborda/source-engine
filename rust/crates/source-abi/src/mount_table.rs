//! Owns mount order and foreign resource leases behind a context-free ABI.
//! Callback execution must never occur under either registry/table mutex.
use super::*;
use source_filesystem::mount_table::{MountTable, StoreIds, MAX_MOUNTS};

type DropFn = unsafe extern "C" fn(*mut c_void);
type CloneFn = unsafe extern "C" fn(*const c_void) -> *mut c_void;

struct Resource {
    // An opaque host context, never dereferenced by Rust. The ABI requires
    // callbacks/data to support calls from the thread performing the operation.
    data: usize,
    drop_fn: DropFn,
}
impl Drop for Resource {
    fn drop(&mut self) {
        unsafe { (self.drop_fn)(self.data as *mut c_void) };
    }
}
struct Registry {
    entries: MountTable<Arc<Resource>>,
    drop_fn: DropFn,
    clone_fn: CloneFn,
}
type Owned = Arc<Mutex<Registry>>;

fn tables() -> &'static Mutex<HashMap<u64, Owned>> {
    static TABLES: OnceLock<Mutex<HashMap<u64, Owned>>> = OnceLock::new();
    TABLES.get_or_init(|| Mutex::new(HashMap::new()))
}
fn get(handle: u64) -> Result<Owned, SourceAbiStatus> {
    tables()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&handle)
        .cloned()
        .ok_or(SOURCE_ABI_INVALID_HANDLE)
}
fn publish(registry: Registry) -> Result<u64, SourceAbiStatus> {
    let handle = NEXT_HANDLE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| SOURCE_ABI_INTERNAL_ERROR)?;
    tables()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(handle, Arc::new(Mutex::new(registry)));
    Ok(handle)
}

#[no_mangle]
/// Creates an empty ordered table. Callbacks/data must remain valid until every
/// owning table/snapshot is destroyed. Callbacks must not unwind, and must be
/// safe on any thread invoking an owning operation. No active context required.
/// # Safety
/// Callbacks obey those contracts; out_handle is writable.
pub unsafe extern "C" fn source_mount_table_create(
    drop_fn: Option<DropFn>,
    clone_fn: Option<CloneFn>,
    out_handle: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_handle.write(0) };
        let (Some(drop_fn), Some(clone_fn)) = (drop_fn, clone_fn) else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        match publish(Registry {
            entries: MountTable::default(),
            drop_fn,
            clone_fn,
        }) {
            Ok(handle) => {
                unsafe { out_handle.write(handle) };
                SOURCE_ABI_OK
            }
            Err(status) => status,
        }
    })
}

#[no_mangle]
/// Transfers a non-null opaque resource on success only; caller retains it on
/// error. Index may equal count (append); at most 65536 mounts.
/// # Safety
/// Resource supports the table's callbacks until released. Mutations must be
/// externally serialized against native borrows returned by get.
pub unsafe extern "C" fn source_mount_table_insert(
    handle: u64,
    index: u32,
    resource: *mut c_void,
) -> SourceAbiStatus {
    ffi_status(|| {
        if resource.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let table = match get(handle) {
            Ok(t) => t,
            Err(s) => return s,
        };
        let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
        if index as usize > table.entries.entries().len()
            || table.entries.entries().len() >= MAX_MOUNTS
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let resource = Arc::new(Resource {
            data: resource as usize,
            drop_fn: table.drop_fn,
        });
        // Bounds checked under the same lock, so this cannot return an owner.
        assert!(table.entries.insert(index as usize, resource).is_ok());
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// # Safety
/// out_count points to writable storage.
pub unsafe extern "C" fn source_mount_table_count(
    handle: u64,
    out_count: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_count.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_count.write(0) };
        let table = match get(handle) {
            Ok(t) => t,
            Err(s) => return s,
        };
        let table = table.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { out_count.write(table.entries.entries().len() as u32) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Returns a borrowed opaque resource, not an ownership transfer. The caller
/// must prevent concurrent removal/clear/destruction while using the borrow.
/// # Safety
/// out_resource is writable; native use obeys the borrow/lifetime contract.
pub unsafe extern "C" fn source_mount_table_get(
    handle: u64,
    index: u32,
    out_resource: *mut *mut c_void,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_resource.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_resource.write(ptr::null_mut()) };
        let table = match get(handle) {
            Ok(t) => t,
            Err(s) => return s,
        };
        let table = table.lock().unwrap_or_else(|e| e.into_inner());
        let Some(resource) = table.entries.entries().get(index as usize) else {
            return SOURCE_ABI_NOT_FOUND;
        };
        unsafe { out_resource.write(resource.data as *mut c_void) };
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Removes one entry, invoking its drop callback outside locks. fast=1 swaps
/// the last entry into the gap; fast=0 preserves remaining order.
pub extern "C" fn source_mount_table_remove(handle: u64, index: u32, fast: u8) -> SourceAbiStatus {
    ffi_status(|| {
        if fast > 1 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let table = match get(handle) {
            Ok(t) => t,
            Err(s) => return s,
        };
        let removed = {
            table
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entries
                .remove(index as usize, fast != 0)
        };
        if removed.is_none() {
            return SOURCE_ABI_NOT_FOUND;
        }
        drop(removed);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Clears order before dropping resources outside locks; leaves a reusable table.
pub extern "C" fn source_mount_table_clear(handle: u64) -> SourceAbiStatus {
    ffi_status(|| {
        let table = match get(handle) {
            Ok(t) => t,
            Err(s) => return s,
        };
        let removed = {
            table
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entries
                .take_all()
        };
        drop(removed);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Clones native resources in order, outside locks, retaining source leases
/// through callbacks. A null clone aborts atomically, dropping earlier copies.
/// The new independent table preserves native header-copy/AddRef semantics.
/// # Safety
/// out_handle is writable; clone callbacks follow the create contract.
pub unsafe extern "C" fn source_mount_table_snapshot(
    handle: u64,
    out_handle: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_handle.write(0) };
        let table = match get(handle) {
            Ok(t) => t,
            Err(s) => return s,
        };
        let (source, drop_fn, clone_fn) = {
            let table = table.lock().unwrap_or_else(|e| e.into_inner());
            (
                table.entries.entries().to_vec(),
                table.drop_fn,
                table.clone_fn,
            )
        };
        let mut snapshot = Registry {
            entries: MountTable::default(),
            drop_fn,
            clone_fn,
        };
        for resource in &source {
            let data = unsafe { clone_fn(resource.data as *const c_void) };
            if data.is_null() {
                return SOURCE_ABI_INTERNAL_ERROR;
            }
            let copy = Arc::new(Resource {
                data: data as usize,
                drop_fn,
            });
            assert!(snapshot
                .entries
                .insert(snapshot.entries.entries().len(), copy)
                .is_ok());
        }
        match publish(snapshot) {
            Ok(handle) => {
                unsafe { out_handle.write(handle) };
                SOURCE_ABI_OK
            }
            Err(status) => status,
        }
    })
}

#[no_mangle]
/// Removes the handle before releasing resources, outside locks. Callbacks may
/// reenter the ABI; the destroyed handle is already invalid.
pub extern "C" fn source_mount_table_destroy(handle: u64) -> SourceAbiStatus {
    ffi_status(|| {
        let removed = {
            tables()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&handle)
        };
        if removed.is_none() {
            return SOURCE_ABI_INVALID_HANDLE;
        }
        drop(removed);
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Allocates a process-wide positive signed store identity without wrapping.
/// Zero and negative IDs remain reserved; identities are never reused.
/// # Safety
/// out_id points to writable storage.
pub unsafe extern "C" fn source_mount_store_id_next(out_id: *mut i32) -> SourceAbiStatus {
    ffi_status(|| {
        if out_id.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_id.write(0) };
        static NEXT: StoreIds = StoreIds::new();
        match NEXT.allocate() {
            Some(id) => {
                unsafe { out_id.write(id) };
                SOURCE_ABI_OK
            }
            None => SOURCE_ABI_INTERNAL_ERROR,
        }
    })
}
