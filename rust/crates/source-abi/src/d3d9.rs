//! The C ABI of the Direct3D 9 on Metal device, declared to C++ in
//! `public/rust/source_d3d9.h` and implemented in the `source-d3d9` crate.
//!
//! Every entry point here is a thin unwrapping of handles and pointers. A
//! handle is a pointer to a boxed Rust object that the C++ class holding it
//! owns; the shader API calls in from one thread at a time, which is the
//! contract ToGL put on it, so nothing here locks. Panics are stopped at the
//! boundary and reported, because unwinding into C++ is undefined.

#![cfg(target_os = "macos")]

use source_d3d9::device::{self, Buffer, Device, Query, Shader, Texture, VertexDeclaration};
use source_d3d9::format;
use source_d3d9::state::{
    DisplayInfo, Handle, Rect, State, SurfaceRef, VertexElement, SHADER_VERTEX,
};
use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn guard<T>(fallback: T, operation: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("d3d9metal: a panic was stopped at the ABI boundary");
            fallback
        }
    }
}

/// # Safety
/// `handle` must be zero or a device from `source_d3d9_device_create` that
/// has not been destroyed, used by one thread at a time.
unsafe fn device<'a>(handle: Handle) -> Option<&'a mut Device> {
    // SAFETY: guaranteed by the caller.
    unsafe { (handle as *mut Device).as_mut() }
}

fn handle_of<T>(value: Option<Box<T>>) -> Handle {
    value.map_or(0, |boxed| Box::into_raw(boxed) as Handle)
}

/// # Safety
/// `text` must be null or a NUL-terminated string.
unsafe fn text<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() {
        return None;
    }
    // SAFETY: guaranteed by the caller.
    unsafe { CStr::from_ptr(text) }.to_str().ok()
}

/// # Safety
/// `ns_window` must be null or a live `NSWindow` that outlives the device.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_set_window(ns_window: *mut c_void) {
    device::set_window(ns_window);
}

/// # Safety
/// `info` must be null or point to a writable `SourceD3D9DisplayInfo`.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_display_info(info: *mut DisplayInfo) -> i32 {
    guard(0, || {
        // SAFETY: guaranteed by the caller.
        let Some(info) = (unsafe { info.as_mut() }) else {
            return 0;
        };
        let Some((width, height, refresh, scale, memory, name)) = device::display_info() else {
            return 0;
        };
        info.pixel_width = width;
        info.pixel_height = height;
        info.refresh_hz = refresh;
        info.backing_scale = scale;
        info.recommended_memory = memory;
        info.name = [0; 128];
        let bytes = name.as_bytes();
        let length = bytes.len().min(info.name.len() - 1);
        info.name[..length].copy_from_slice(&bytes[..length]);
        1
    })
}

#[no_mangle]
pub extern "C" fn source_d3d9_format_supported(d3d_format: u32, _usage: u32, _kind: u32) -> i32 {
    i32::from(format::lookup(d3d_format).is_some())
}

#[no_mangle]
pub extern "C" fn source_d3d9_device_create(width: u32, height: u32) -> Handle {
    guard(0, || handle_of(Device::create(width, height)))
}

/// # Safety
/// `handle` must be zero or a live device, which this destroys.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_device_destroy(handle: Handle) {
    guard((), || {
        if handle != 0 {
            // SAFETY: guaranteed by the caller.
            drop(unsafe { Box::from_raw(handle as *mut Device) });
        }
    });
}

/// # Safety
/// `handle` must be zero or a live device.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_device_reset(handle: Handle, width: u32, height: u32) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        if let Some(device) = unsafe { device(handle) } {
            device.reset(width, height);
        }
    });
}

/// # Safety
/// `handle` must be zero or a live device and `debug_label` null or a
/// NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_texture_create(
    handle: Handle,
    kind: u32,
    width: u32,
    height: u32,
    depth: u32,
    levels: u32,
    usage: u32,
    d3d_format: u32,
    debug_label: *const c_char,
) -> Handle {
    guard(0, || {
        // SAFETY: guaranteed by the caller.
        let (Some(device), label) = (unsafe { device(handle) }, unsafe { text(debug_label) })
        else {
            return 0;
        };
        handle_of(
            device.create_texture(kind, width, height, depth, levels, usage, d3d_format, label),
        )
    })
}

/// # Safety
/// `handle` must be zero or a live device and `texture` zero or a texture it
/// created, which this destroys.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_texture_destroy(handle: Handle, texture: Handle) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        match unsafe { device(handle) } {
            Some(device) => device.destroy_texture(texture),
            None if texture != 0 => drop(unsafe { Box::from_raw(texture as *mut Texture) }),
            None => {}
        }
    });
}

/// # Safety
/// `handle` and `texture` as above; `rect` null or a readable rectangle; the
/// three out-pointers writable.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_texture_lock(
    handle: Handle,
    texture: Handle,
    face: u32,
    level: u32,
    rect: *const Rect,
    front: u32,
    back: u32,
    readback: i32,
    bits: *mut *mut c_void,
    pitch: *mut i32,
    slice_pitch: *mut i32,
) -> i32 {
    guard(0, || {
        if bits.is_null() || pitch.is_null() || slice_pitch.is_null() {
            return 0;
        }
        // SAFETY: guaranteed by the caller.
        let Some(device) = (unsafe { device(handle) }) else {
            return 0;
        };
        // SAFETY: guaranteed by the caller.
        let rect = unsafe { rect.as_ref() }.copied();
        match device.lock_texture(texture, face, level, rect, front, back, readback != 0) {
            Some((memory, row, slice)) => {
                // SAFETY: the out-pointers were checked non-null above.
                unsafe {
                    *bits = memory.cast();
                    *pitch = row as i32;
                    *slice_pitch = slice as i32;
                }
                1
            }
            None => 0,
        }
    })
}

/// # Safety
/// `handle` and `texture` as above.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_texture_unlock(
    handle: Handle,
    texture: Handle,
    face: u32,
    level: u32,
) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        if let Some(device) = unsafe { device(handle) } {
            device.unlock_texture(texture, face, level);
        }
    });
}

/// # Safety
/// `handle` must be zero or a live device.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_buffer_create(
    handle: Handle,
    size: u32,
    _usage: u32,
    is_index: i32,
    index_size: i32,
) -> Handle {
    guard(0, || {
        // SAFETY: guaranteed by the caller.
        let Some(device) = (unsafe { device(handle) }) else {
            return 0;
        };
        let index_size = if is_index != 0 {
            index_size.max(2) as u32
        } else {
            0
        };
        handle_of(device.create_buffer(size, index_size))
    })
}

/// # Safety
/// `handle` as above; `buffer` zero or a buffer it created, destroyed here.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_buffer_destroy(handle: Handle, buffer: Handle) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        match unsafe { device(handle) } {
            Some(device) => device.destroy_buffer(buffer),
            None if buffer != 0 => drop(unsafe { Box::from_raw(buffer as *mut Buffer) }),
            None => {}
        }
    });
}

/// # Safety
/// `handle` and `buffer` as above.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_buffer_lock(
    handle: Handle,
    buffer: Handle,
    offset: u32,
    size: u32,
    flags: u32,
) -> *mut c_void {
    guard(std::ptr::null_mut(), || {
        // SAFETY: guaranteed by the caller.
        match unsafe { device(handle) } {
            Some(device) => device.lock_buffer(buffer, offset, size, flags).cast(),
            None => std::ptr::null_mut(),
        }
    })
}

/// The lock handed out the buffer's own shared memory, so there is nothing
/// to copy back.
#[no_mangle]
pub extern "C" fn source_d3d9_buffer_unlock(_handle: Handle, _buffer: Handle) {}

/// # Safety
/// `handle` as above; `elements` must point to `count` elements.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_vertex_declaration_create(
    handle: Handle,
    elements: *const VertexElement,
    count: u32,
) -> Handle {
    guard(0, || {
        // SAFETY: guaranteed by the caller.
        let Some(device) = (unsafe { device(handle) }) else {
            return 0;
        };
        let elements = if elements.is_null() || count == 0 {
            &[][..]
        } else {
            // SAFETY: guaranteed by the caller.
            unsafe { std::slice::from_raw_parts(elements, count as usize) }
        };
        Box::into_raw(device.create_vertex_declaration(elements)) as Handle
    })
}

/// # Safety
/// `declaration` must be zero or a live declaration, destroyed here.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_vertex_declaration_destroy(
    _handle: Handle,
    declaration: Handle,
) {
    guard((), || {
        if declaration != 0 {
            // SAFETY: guaranteed by the caller.
            drop(unsafe { Box::from_raw(declaration as *mut VertexDeclaration) });
        }
    });
}

/// Reads a shader's token stream up to and including its END token. The
/// caller passes no length, as Direct3D's own `CreateVertexShader` does not.
///
/// # Safety
/// `bytecode` must point to a Direct3D 9 shader token stream that ends in an
/// END token.
unsafe fn shader_tokens<'a>(bytecode: *const u32) -> &'a [u32] {
    const END: u32 = 0x0000_FFFF;
    const COMMENT: u32 = 0xFFFE;
    const LIMIT: usize = 1 << 20;
    let mut length = 1;
    // SAFETY: the stream is walked token by token and stops at END, which the
    // caller guarantees is present; comments are skipped whole because their
    // payload is arbitrary data that may contain the END pattern.
    unsafe {
        while length < LIMIT {
            let token = *bytecode.add(length);
            length += 1;
            if token == END {
                break;
            }
            if token & 0xFFFF == COMMENT {
                length += ((token >> 16) & 0x7FFF) as usize;
            }
        }
        std::slice::from_raw_parts(bytecode, length.min(LIMIT))
    }
}

/// # Safety
/// `handle` as above; `bytecode` a complete shader token stream; `name` null
/// or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_shader_create(
    handle: Handle,
    stage: i32,
    bytecode: *const u32,
    name: *const c_char,
) -> Handle {
    guard(0, || {
        if bytecode.is_null() {
            return 0;
        }
        // SAFETY: guaranteed by the caller.
        let Some(device) = (unsafe { device(handle) }) else {
            return 0;
        };
        // SAFETY: guaranteed by the caller.
        let (tokens, name) = unsafe { (shader_tokens(bytecode), text(name)) };
        let fallback = if stage == SHADER_VERTEX {
            "vertex shader"
        } else {
            "pixel shader"
        };
        handle_of(device.create_shader(stage, tokens, name.unwrap_or(fallback)))
    })
}

/// # Safety
/// `shader` must be zero or a live shader, destroyed here.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_shader_destroy(_handle: Handle, shader: Handle) {
    guard((), || {
        if shader != 0 {
            // SAFETY: guaranteed by the caller.
            drop(unsafe { Box::from_raw(shader as *mut Shader) });
        }
    });
}

/// # Safety
/// `handle` as above; `state` must point to a state block whose handles are
/// all live or zero.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_draw(
    handle: Handle,
    state: *const State,
    primitive_type: u32,
    start_vertex: u32,
    primitive_count: u32,
) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        if let (Some(device), Some(state)) = (unsafe { device(handle) }, unsafe { state.as_ref() })
        {
            device.draw(state, primitive_type, start_vertex, primitive_count);
        }
    });
}

/// # Safety
/// As `source_d3d9_draw`.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_draw_indexed(
    handle: Handle,
    state: *const State,
    primitive_type: u32,
    base_vertex: i32,
    _min_index: u32,
    _vertex_count: u32,
    start_index: u32,
    primitive_count: u32,
) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        if let (Some(device), Some(state)) = (unsafe { device(handle) }, unsafe { state.as_ref() })
        {
            device.draw_indexed(
                state,
                primitive_type,
                base_vertex,
                start_index,
                primitive_count,
            );
        }
    });
}

/// # Safety
/// As `source_d3d9_draw`; `rects` must point to `rect_count` rectangles.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_clear(
    handle: Handle,
    state: *const State,
    rect_count: u32,
    rects: *const Rect,
    flags: u32,
    color: u32,
    z: f32,
    stencil: u32,
) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        let (Some(device), Some(state)) = (unsafe { device(handle) }, unsafe { state.as_ref() })
        else {
            return;
        };
        let rects = if rects.is_null() || rect_count == 0 {
            &[][..]
        } else {
            // SAFETY: guaranteed by the caller.
            unsafe { std::slice::from_raw_parts(rects, rect_count as usize) }
        };
        device.clear(state, rects, flags, color, z, stencil);
    });
}

/// # Safety
/// `handle` as above; the surface pointers must be readable and name live
/// textures; the rectangles null or readable.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_stretch_rect(
    handle: Handle,
    source: *const SurfaceRef,
    source_rect: *const Rect,
    destination: *const SurfaceRef,
    destination_rect: *const Rect,
    filter: u32,
) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        unsafe {
            if let (Some(device), Some(source), Some(destination)) =
                (device(handle), source.as_ref(), destination.as_ref())
            {
                device.stretch_rect(
                    *source,
                    source_rect.as_ref().copied(),
                    *destination,
                    destination_rect.as_ref().copied(),
                    filter,
                );
            }
        }
    });
}

/// # Safety
/// As `source_d3d9_stretch_rect`.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_read_render_target(
    handle: Handle,
    source: *const SurfaceRef,
    destination: *const SurfaceRef,
) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        unsafe {
            if let (Some(device), Some(source), Some(destination)) =
                (device(handle), source.as_ref(), destination.as_ref())
            {
                device.read_render_target(*source, *destination);
            }
        }
    });
}

/// # Safety
/// `handle` as above; `back_buffer` readable and naming a live texture.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_present(handle: Handle, back_buffer: *const SurfaceRef) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        unsafe {
            if let (Some(device), Some(back_buffer)) = (device(handle), back_buffer.as_ref()) {
                device.present(*back_buffer);
            }
        }
    });
}

/// # Safety
/// `handle` must be zero or a live device.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_query_create(handle: Handle, _kind: u32) -> Handle {
    guard(0, || {
        // SAFETY: guaranteed by the caller.
        match unsafe { device(handle) } {
            Some(device) => Box::into_raw(device.create_query()) as Handle,
            None => 0,
        }
    })
}

/// # Safety
/// `query` must be zero or a live query, destroyed here.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_query_destroy(_handle: Handle, query: Handle) {
    guard((), || {
        if query != 0 {
            // SAFETY: guaranteed by the caller.
            drop(unsafe { Box::from_raw(query as *mut Query) });
        }
    });
}

/// # Safety
/// `handle` and `query` as above.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_query_issue(handle: Handle, query: Handle, flags: u32) {
    guard((), || {
        // SAFETY: guaranteed by the caller.
        if let Some(device) = unsafe { device(handle) } {
            device.issue_query(query, flags);
        }
    });
}

/// # Safety
/// `handle` and `query` as above; `pixels` null or writable.
#[no_mangle]
pub unsafe extern "C" fn source_d3d9_query_get_data(
    handle: Handle,
    query: Handle,
    flush: i32,
    pixels: *mut u32,
) -> i32 {
    guard(0, || {
        // SAFETY: guaranteed by the caller.
        let Some(device) = (unsafe { device(handle) }) else {
            return 0;
        };
        match device.query_data(query, flush != 0) {
            Some(count) => {
                // SAFETY: guaranteed by the caller.
                if let Some(pixels) = unsafe { pixels.as_mut() } {
                    *pixels = count;
                }
                1
            }
            None => 0,
        }
    })
}
