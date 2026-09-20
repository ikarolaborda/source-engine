use crate::{
    ffi_status, SourceAbiStatus, SOURCE_ABI_INTERNAL_ERROR, SOURCE_ABI_INVALID_ARGUMENT,
    SOURCE_ABI_INVALID_HANDLE, SOURCE_ABI_OK,
};
use source_host::app_system::StartedGroup;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::ThreadId;

type AppSystemFn = unsafe extern "C" fn(*mut c_void, u32, u32) -> i32;

enum Phase {
    Starting,
    Ready(StartedGroup),
    Stopping,
}

struct Group {
    // Identity only: these addresses are never dereferenced from the registry.
    identity: (usize, usize),
    owner: ThreadId,
    phase: Phase,
}

fn groups() -> &'static Mutex<HashMap<u64, Group>> {
    static GROUPS: OnceLock<Mutex<HashMap<u64, Group>>> = OnceLock::new();
    GROUPS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct Reservation {
    handle: u64,
    keep: bool,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.keep {
            groups()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.handle);
        }
    }
}

fn reserve(callback: AppSystemFn, user_data: *mut c_void) -> Result<Reservation, SourceAbiStatus> {
    let identity = (callback as usize, user_data as usize);
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let handle = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            n.checked_add(1).filter(|next| *next < u64::MAX)
        })
        .map_err(|_| SOURCE_ABI_INTERNAL_ERROR)?;
    let mut table = groups().lock().unwrap_or_else(|e| e.into_inner());
    if table.values().any(|group| group.identity == identity) {
        return Err(SOURCE_ABI_INVALID_ARGUMENT);
    }
    table.insert(
        handle,
        Group {
            identity,
            owner: std::thread::current().id(),
            phase: Phase::Starting,
        },
    );
    Ok(Reservation {
        handle,
        keep: false,
    })
}

#[no_mangle]
/// Starts a group without running Main; success returns a live handle and
/// result zero. Startup failure rolls back and returns handle zero/result -1.
/// Duplicate callback/user identities are rejected until shutdown completes.
///
/// # Safety
/// Callback/data must remain callable during this call, with no unwinding.
/// Outputs must point to distinct writable u64/i32 storage. Keep native systems
/// alive until shutdown on this thread with the same callback/data identity.
pub unsafe extern "C" fn source_host_app_group_startup(
    callback: Option<AppSystemFn>,
    user_data: *mut c_void,
    out_handle: *mut u64,
    out_result: *mut i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        let Some(callback) = callback else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        if out_handle.is_null() || out_result.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe {
            out_handle.write(0);
            out_result.write(-1);
        }
        let mut reservation = match reserve(callback, user_data) {
            Ok(r) => r,
            Err(status) => return status,
        };
        let handle = reservation.handle;
        let started = StartedGroup::startup(&mut |op, index| unsafe {
            callback(user_data, op as u32, index)
        });
        if let Some(started) = started {
            groups()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&handle)
                .expect("starting reservation cannot be removed by a callback")
                .phase = Phase::Ready(started);
            reservation.keep = true;
            unsafe {
                out_handle.write(handle);
                out_result.write(0);
            }
        }
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Shuts down a live split-phase group exactly once. No registry lock spans
/// native callbacks. A stopped/stale handle, mismatched identity, wrong thread
/// or reentrant shutdown is rejected without dispatching a callback.
///
/// # Safety
/// Callback/data must be the still-valid pair from startup and must not unwind.
pub unsafe extern "C" fn source_host_app_group_shutdown(
    handle: u64,
    callback: Option<AppSystemFn>,
    user_data: *mut c_void,
) -> SourceAbiStatus {
    ffi_status(|| {
        let Some(callback) = callback else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        let mut state = {
            let mut table = groups().lock().unwrap_or_else(|e| e.into_inner());
            let Some(group) = table.get_mut(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            if group.identity != (callback as usize, user_data as usize)
                || group.owner != std::thread::current().id()
                || !matches!(group.phase, Phase::Ready(_))
            {
                return SOURCE_ABI_INVALID_ARGUMENT;
            }
            match std::mem::replace(&mut group.phase, Phase::Stopping) {
                Phase::Ready(state) => state,
                _ => unreachable!("ready phase checked under lock"),
            }
        };
        let _reservation = Reservation {
            handle,
            keep: false,
        };
        state.shutdown(&mut |op, index| unsafe { callback(user_data, op as u32, index) });
        SOURCE_ABI_OK
    })
}

#[no_mangle]
/// Runs synchronous app-system startup, Main and reverse-order cleanup.
/// Independent groups may reenter this function from their callbacks.
///
/// # Safety
/// The callback/user data must remain valid throughout the call and must not
/// unwind across the ABI. See source_host::app_system::run for its contract.
/// `out_result` must point to writable i32 storage; no pointers are retained.
pub unsafe extern "C" fn source_host_run_app_system_group(
    callback: Option<AppSystemFn>,
    user_data: *mut c_void,
    out_result: *mut i32,
) -> SourceAbiStatus {
    ffi_status(|| {
        let Some(callback) = callback else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        if out_result.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { out_result.write(-1) };
        let _reservation = match reserve(callback, user_data) {
            Ok(r) => r,
            Err(status) => return status,
        };
        let result = source_host::app_system::run(|operation, index| {
            // SAFETY: borrowed callback and user-data lifetime is the caller's contract.
            unsafe { callback(user_data, operation as u32, index) }
        });
        unsafe { out_result.write(result) };
        SOURCE_ABI_OK
    })
}
