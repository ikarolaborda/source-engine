//! `IAppSystem`, the base of every interface an app-system group manages.

use crate::CreateInterfaceFn;
use std::ffi::{c_char, c_void};

/// `InitReturnVal_t`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitReturnVal {
    Failed = 0,
    Ok = 1,
}

/// `IAppSystem`'s five virtual functions in declaration order, which an
/// interface deriving from it lists before its own.
#[repr(C)]
pub struct AppSystemMethods<T> {
    pub connect: unsafe extern "C" fn(this: *mut T, factory: Option<CreateInterfaceFn>) -> bool,
    pub disconnect: unsafe extern "C" fn(this: *mut T),
    pub query_interface: unsafe extern "C" fn(this: *mut T, name: *const c_char) -> *mut c_void,
    pub init: unsafe extern "C" fn(this: *mut T) -> InitReturnVal,
    pub shutdown: unsafe extern "C" fn(this: *mut T),
}
