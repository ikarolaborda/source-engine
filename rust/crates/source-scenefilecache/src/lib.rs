//! The `scenefilecache` engine module, with no C++ in it.
//!
//! The game server loads this library by name, asks its `CreateInterface` for
//! `SceneFileCache002`, and from then on calls it as an `ISceneFileCache`. It
//! replaces `scenefilecache/SceneFileCache.cpp` whole: the object handed out
//! is a [`source_cppabi`] table of the functions below, the image is read
//! through the filesystem module's own interface, and every answer comes from
//! [`source_scene::cache`], which mirrors the native lookups.
//!
//! The app-system group resolves names by asking every system it holds,
//! this one included, so `QueryInterface` can be entered while `Connect` is
//! still inside the factory. Nothing here holds the state lock across a call
//! out of the module.

use source_cppabi::appsystem::{AppSystemMethods, InitReturnVal};
use source_cppabi::filesystem::{BaseFileSystem, BASE_FILESYSTEM_INTERFACE, FILESYSTEM_INTERFACE};
use source_cppabi::{create_interface, guard, tier0, CreateInterfaceFn, Object, VTable};
use source_scene::cache::SceneCache;
use std::ffi::{c_char, c_int, c_short, c_uint, c_void, CStr};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// `SCENE_FILE_CACHE_INTERFACE_VERSION`.
pub const INTERFACE_VERSION: &CStr = c"SceneFileCache002";
const IMAGE_NAME: &CStr = c"scenes/scenes.image";
const IMAGE_PATH_ID: &CStr = c"GAME";

/// `SceneCachedData_t`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneCachedData {
    pub msecs: c_uint,
    pub num_sounds: c_int,
    pub scene_id: c_int,
}

type This = Object<Methods>;

/// `ISceneFileCache`: `IAppSystem`, then its own six in declaration order.
#[repr(C)]
struct Methods {
    app_system: AppSystemMethods<This>,
    get_scene_buffer_size: unsafe extern "C" fn(*mut This, *const c_char) -> usize,
    get_scene_data: unsafe extern "C" fn(*mut This, *const c_char, *mut u8, usize) -> bool,
    get_scene_cached_data:
        unsafe extern "C" fn(*mut This, *const c_char, *mut SceneCachedData) -> bool,
    get_scene_cached_sound: unsafe extern "C" fn(*mut This, c_int, c_int) -> c_short,
    get_scene_string: unsafe extern "C" fn(*mut This, c_short) -> *const c_char,
    reload: unsafe extern "C" fn(*mut This),
}

static TABLE: VTable<Methods> = VTable::new(Methods {
    app_system: AppSystemMethods {
        connect,
        disconnect,
        query_interface,
        init,
        shutdown,
    },
    get_scene_buffer_size,
    get_scene_data,
    get_scene_cached_data,
    get_scene_cached_sound,
    get_scene_string,
    reload,
});
static SCENE_FILE_CACHE: This = Object::new(&TABLE);

struct State {
    filesystem: Option<BaseFileSystem>,
    cache: Option<SceneCache>,
}

static STATE: Mutex<State> = Mutex::new(State {
    filesystem: None,
    cache: None,
});

fn state() -> MutexGuard<'static, State> {
    // A panic elsewhere leaves the state as it last was, which is still valid.
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The module's one export, which the engine resolves by this name.
///
/// # Safety
///
/// `name` must be null or NUL-terminated and `return_code` null or writable.
#[no_mangle]
pub unsafe extern "C" fn CreateInterface(
    name: *const c_char,
    return_code: *mut c_int,
) -> *mut c_void {
    let interfaces = [(INTERFACE_VERSION, SCENE_FILE_CACHE.as_interface())];
    // SAFETY: forwarded under the caller's contract.
    unsafe { create_interface(&interfaces, name, return_code) }
}

unsafe extern "C" fn connect(_: *mut This, factory: Option<CreateInterfaceFn>) -> bool {
    guard(false, || {
        let Some(factory) = factory else {
            return false;
        };
        // The name the C++ module asked for comes first: the group holds the
        // filesystem under it, where the base interface's name is answered
        // only by a sweep of every system.
        // SAFETY: the factory is the engine's, and each name yields the
        // interface type its binding assumes.
        let filesystem = unsafe {
            BaseFileSystem::from_filesystem(factory(
                FILESYSTEM_INTERFACE.as_ptr(),
                std::ptr::null_mut(),
            ))
            .or_else(|| {
                BaseFileSystem::from_raw(factory(
                    BASE_FILESYSTEM_INTERFACE.as_ptr(),
                    std::ptr::null_mut(),
                ))
            })
        };
        state().filesystem = filesystem;
        filesystem.is_some()
    })
}

unsafe extern "C" fn disconnect(_: *mut This) {
    // The group is rebuilt for each mod and connects again with a new factory.
    guard((), || state().filesystem = None);
}

/// Called by the group while it resolves any name, possibly from inside
/// `connect`, so it must not take the state lock. There is nothing to offer.
unsafe extern "C" fn query_interface(_: *mut This, _: *const c_char) -> *mut c_void {
    std::ptr::null_mut()
}

unsafe extern "C" fn init(_: *mut This) -> InitReturnVal {
    guard(InitReturnVal::Ok, || {
        load();
        InitReturnVal::Ok
    })
}

unsafe extern "C" fn shutdown(_: *mut This) {
    guard((), || state().cache = None);
}

unsafe extern "C" fn reload(_: *mut This) {
    guard((), || {
        state().cache = None;
        load();
    });
}

/// `Init`: mounts the image unless one is already resident. A missing image
/// is not an error; the game runs without choreographed scenes. An image with
/// the wrong tag or version is the engine's fatal error, as it was natively.
fn load() {
    let filesystem = {
        let state = state();
        if state.cache.is_some() {
            return;
        }
        state.filesystem
    };
    // Read with the lock released: this is a call into another module.
    let Some(image) =
        filesystem.and_then(|filesystem| filesystem.read_file(IMAGE_NAME, IMAGE_PATH_ID))
    else {
        return;
    };
    match SceneCache::load(image) {
        Ok(cache) => {
            // Another thread's Init may have finished first; the image it
            // mounted may already have had pointers handed out of it.
            let mut state = state();
            if state.cache.is_none() {
                state.cache = Some(cache);
            }
        }
        Err(error) => tier0::error(&format!(
            "CSceneFileCache: Bad scene image file scenes/scenes.image ({error})\n"
        )),
    }
}

/// # Safety
///
/// `name` must be null or NUL-terminated.
unsafe fn name_bytes<'a>(name: *const c_char) -> &'a [u8] {
    if name.is_null() {
        return &[];
    }
    // SAFETY: non-null and terminated per the contract.
    unsafe { CStr::from_ptr(name) }.to_bytes()
}

unsafe extern "C" fn get_scene_buffer_size(_: *mut This, filename: *const c_char) -> usize {
    guard(0, || {
        // SAFETY: the engine passes a terminated name.
        let name = unsafe { name_bytes(filename) };
        let state = state();
        state
            .cache
            .as_ref()
            .and_then(|cache| cache.scene_size(cache.find(name)?))
            .unwrap_or(0)
    })
}

unsafe extern "C" fn get_scene_data(
    _: *mut This,
    filename: *const c_char,
    buffer: *mut u8,
    buffer_size: usize,
) -> bool {
    guard(false, || {
        // SAFETY: the engine passes a terminated name.
        let name = unsafe { name_bytes(filename) };
        let output: &mut [u8] = if buffer.is_null() {
            &mut []
        } else {
            // SAFETY: the engine owns `buffer_size` writable bytes there.
            unsafe { std::slice::from_raw_parts_mut(buffer, buffer_size.min(isize::MAX as usize)) }
        };
        let state = state();
        let copied = state
            .cache
            .as_ref()
            .and_then(|cache| cache.copy_scene(cache.find(name)?, output));
        if copied.is_none() {
            // The native miss path stores a null through the byte pointer.
            if let Some(first) = output.first_mut() {
                *first = 0;
            }
        }
        copied.is_some()
    })
}

unsafe extern "C" fn get_scene_cached_data(
    _: *mut This,
    filename: *const c_char,
    data: *mut SceneCachedData,
) -> bool {
    guard(false, || {
        if data.is_null() {
            return false;
        }
        // SAFETY: the engine passes a terminated name.
        let name = unsafe { name_bytes(filename) };
        let state = state();
        let found = state.cache.as_ref().and_then(|cache| {
            let scene = cache.find(name)?;
            Some((scene, cache.cached_data(scene)?))
        });
        let result = match found {
            Some((scene, summary)) => SceneCachedData {
                msecs: summary.milliseconds,
                num_sounds: summary.sound_count,
                scene_id: scene as c_int,
            },
            None => SceneCachedData {
                msecs: 0,
                num_sounds: 0,
                scene_id: -1,
            },
        };
        // SAFETY: non-null, and the engine owns the struct it points at.
        unsafe { data.write(result) };
        found.is_some()
    })
}

unsafe extern "C" fn get_scene_cached_sound(_: *mut This, scene: c_int, sound: c_int) -> c_short {
    guard(-1, || {
        state()
            .cache
            .as_ref()
            .and_then(|cache| cache.cached_sound(scene, sound))
            .unwrap_or(-1)
    })
}

unsafe extern "C" fn get_scene_string(_: *mut This, string_id: c_short) -> *const c_char {
    guard(std::ptr::null(), || {
        // The string lives in the image, so the pointer outlasts the lock and
        // stays good until the image is dropped, as the native one does.
        state()
            .cache
            .as_ref()
            .and_then(|cache| cache.string(string_id))
            .map_or(std::ptr::null(), CStr::as_ptr)
    })
}
