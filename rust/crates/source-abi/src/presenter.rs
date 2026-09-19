//! Presenting Metal frames into the window the engine already owns.
//!
//! The renderer has been able to draw into a `CAMetalLayer` for a while,
//! but only into one it made for itself and read back offscreen. This is
//! the seam that lets it draw into the window a player is looking at: the
//! C++ side creates the window without an OpenGL context, hands the view
//! across, and gets back a handle it presents through.
//!
//! Presenters are kept per thread rather than in a shared map, because
//! AppKit requires view changes on the main thread and Metal's objects are
//! not `Send`. A presenter used from the thread that did not create it is
//! reported as an invalid handle rather than being made to work, since the
//! alternative is a crash somewhere else later.

use crate::{
    ffi_status, SourceAbiHandle, SourceAbiStatus, NEXT_HANDLE, SOURCE_ABI_INTERNAL_ERROR,
    SOURCE_ABI_INVALID_ARGUMENT, SOURCE_ABI_INVALID_HANDLE, SOURCE_ABI_OK,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::Ordering;

#[cfg(target_os = "macos")]
struct Presenter {
    device: source_render::Device,
    swapchain: source_render::Swapchain,
    /// Kept so a resize can re-read the window's backing scale, which
    /// changes when the window is dragged between displays without its
    /// size in points changing at all.
    window: *mut c_void,
    /// The map being drawn, once one has been loaded onto this presenter's
    /// device. A presenter without one still presents, as a clear, which
    /// is what the frames before a map loads are.
    scene: Option<source_materialsystem::Scene>,
    /// The engine's two-dimensional output for the frame being built. It
    /// outlives a frame because the textures it holds do: the engine
    /// rasterises a font once and names it for as long as it runs.
    overlay: Option<source_materialsystem::Overlay>,
}

#[cfg(target_os = "macos")]
thread_local! {
    static PRESENTERS: RefCell<HashMap<SourceAbiHandle, Presenter>> =
        RefCell::new(HashMap::new());
}

/// Opens a Metal device, makes a layer the size of the window, and hands
/// the layer to the window's own view, so that what is drawn is what the
/// window shows.
///
/// # Safety
/// `window` must be a live `NSWindow` that outlives the presenter, and
/// this must be called on the main thread. `out_handle` must point at
/// writable storage for one handle.
#[no_mangle]
pub unsafe extern "C" fn source_render_presenter_create(
    window: *mut c_void,
    width: u32,
    height: u32,
    scale: f64,
    out_handle: *mut SourceAbiHandle,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_handle.is_null() || window.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        // A zero-sized layer has no drawable to present into, and a
        // non-finite scale reaches Core Animation as a size it cannot
        // make, so both are refused here rather than deeper in.
        // A scale of zero asks the window for its own backing scale,
        // which is how the caller avoids having to know whether the
        // display is Retina.
        if width == 0 || height == 0 || !scale.is_finite() || scale < 0.0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        unsafe { std::ptr::write(out_handle, 0) };

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (window, width, height, scale);
            SOURCE_ABI_INTERNAL_ERROR
        }

        #[cfg(target_os = "macos")]
        {
            let Ok(device) = source_render::Device::open() else {
                return SOURCE_ABI_INTERNAL_ERROR;
            };
            // SAFETY: the caller guarantees a live window on the main
            // thread, which is this function's own contract.
            let scale = if scale > 0.0 {
                scale
            } else {
                unsafe { source_render::backing_scale_of(window) }.unwrap_or(1.0)
            };
            let Ok(swapchain) = device.create_swapchain(width, height, scale) else {
                return SOURCE_ABI_INTERNAL_ERROR;
            };
            // SAFETY: the caller guarantees a live window on the main
            // thread, which is this function's own contract.
            if unsafe { swapchain.attach_to_window(window) }.is_err() {
                return SOURCE_ABI_INTERNAL_ERROR;
            }
            let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            PRESENTERS.with(|presenters| {
                presenters.borrow_mut().insert(
                    handle,
                    Presenter {
                        device,
                        swapchain,
                        window,
                        scene: None,
                        overlay: None,
                    },
                )
            });
            unsafe { std::ptr::write(out_handle, handle) };
            SOURCE_ABI_OK
        }
    })
}

/// Releases the layer and the device behind a presenter.
///
/// # Safety
/// Must be called on the thread that created the presenter, and before the
/// view it was attached to goes away.
#[no_mangle]
pub unsafe extern "C" fn source_render_presenter_destroy(
    handle: SourceAbiHandle,
) -> SourceAbiStatus {
    ffi_status(|| {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = handle;
            SOURCE_ABI_INVALID_HANDLE
        }

        #[cfg(target_os = "macos")]
        {
            let removed =
                PRESENTERS.with(|presenters| presenters.borrow_mut().remove(&handle).is_some());
            if removed {
                SOURCE_ABI_OK
            } else {
                SOURCE_ABI_INVALID_HANDLE
            }
        }
    })
}

/// Resizes the layer to a new window size, in points.
///
/// A `scale` of zero re-reads the window's own backing scale, which is
/// what a caller wants on an ordinary resize: a window moved between a
/// Retina display and an external one changes scale without changing its
/// size in points, and a layer that tracked only the size draws at the
/// wrong resolution on the other display.
///
/// # Safety
/// Must be called on the thread that created the presenter.
#[no_mangle]
pub unsafe extern "C" fn source_render_presenter_resize(
    handle: SourceAbiHandle,
    width: u32,
    height: u32,
    scale: f64,
) -> SourceAbiStatus {
    ffi_status(|| {
        if width == 0 || height == 0 || !scale.is_finite() || scale < 0.0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = handle;
            SOURCE_ABI_INVALID_HANDLE
        }

        #[cfg(target_os = "macos")]
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(presenter) = presenters.get_mut(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let scale = if scale > 0.0 {
                scale
            } else {
                // SAFETY: the window outlives the presenter by the
                // contract on `source_render_presenter_create`, and this
                // is the main thread by the contract on this function.
                unsafe { source_render::backing_scale_of(presenter.window) }.unwrap_or(1.0)
            };
            if presenter.swapchain.set_contents_scale(scale).is_err()
                || presenter.swapchain.resize(width, height).is_err()
            {
                return SOURCE_ABI_INTERNAL_ERROR;
            }
            SOURCE_ABI_OK
        })
    })
}

/// What a presented frame drew, so a caller can tell a frame with a world
/// in it from a bare clear without reading the pixels back.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceAbiWorldDraw {
    /// Draw calls issued, which is one per material with visible surfaces.
    pub batches: u64,
    /// Triangles those draws covered.
    pub triangles: u64,
    /// Triangles the whole map holds, which is what the view selected from.
    pub map_triangles: u64,
    /// Materials that resolved to a texture when the map was loaded.
    pub materials: u64,
    /// Triangles of that total which were the map's props.
    pub prop_triangles: u64,
    /// Placements of a prop the map draws.
    pub props: u64,
}

/// Loads a map onto a presenter's device, ready to be drawn.
///
/// The map is read through `context`'s own content mounts, so it resolves
/// exactly as it does for the rest of the engine, and the map's embedded
/// archive is mounted as part of loading it.
///
/// # Safety
/// Must be called on the thread that created the presenter. `map` must
/// describe readable bytes, and `out_drawn`, when not null, must point at
/// writable storage for one [`SourceAbiWorldDraw`].
#[no_mangle]
pub unsafe extern "C" fn source_render_world_load(
    handle: SourceAbiHandle,
    context: SourceAbiHandle,
    map: crate::SourceAbiSlice,
    out_drawn: *mut SourceAbiWorldDraw,
) -> SourceAbiStatus {
    ffi_status(|| {
        // SAFETY: the caller guarantees the slice describes readable bytes.
        let map = match unsafe { crate::read_utf8_slice(map) } {
            Ok(map) => map,
            Err(status) => return status,
        };
        let Some(filesystem) = crate::context_filesystem(context) else {
            return SOURCE_ABI_INVALID_HANDLE;
        };

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (handle, filesystem, out_drawn);
            SOURCE_ABI_INTERNAL_ERROR
        }

        #[cfg(target_os = "macos")]
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(presenter) = presenters.get_mut(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let mut paths = filesystem
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let scene = match source_materialsystem::Scene::load(&presenter.device, &mut paths, map)
            {
                Ok(scene) => scene,
                Err(_) => return SOURCE_ABI_INTERNAL_ERROR,
            };
            if !out_drawn.is_null() {
                let drawn = SourceAbiWorldDraw {
                    batches: 0,
                    triangles: 0,
                    map_triangles: scene.triangle_count() as u64,
                    materials: scene.bound_materials() as u64,
                    prop_triangles: 0,
                    props: scene.prop_count() as u64,
                };
                // SAFETY: the caller guarantees writable storage.
                unsafe { std::ptr::write(out_drawn, drawn) };
            }
            presenter.scene = Some(scene);
            SOURCE_ABI_OK
        })
    })
}

/// Draws the loaded map from where the player is standing and presents it.
///
/// `position` and `angles` are the engine's own view, in its own units and
/// its own pitch-yaw-roll order, so the caller passes what it already has
/// rather than building a matrix the renderer would only take apart again.
///
/// # Safety
/// Must be called on the thread that created the presenter. `position` and
/// `angles` must each point at three readable floats, and `out_drawn`,
/// when not null, at writable storage for one [`SourceAbiWorldDraw`].
#[no_mangle]
pub unsafe extern "C" fn source_render_world_present(
    handle: SourceAbiHandle,
    position: *const f32,
    angles: *const f32,
    out_drawn: *mut SourceAbiWorldDraw,
) -> SourceAbiStatus {
    ffi_status(|| {
        if position.is_null() || angles.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        // SAFETY: the caller guarantees three readable floats at each.
        let (position, angles) = unsafe {
            (
                [*position, *position.add(1), *position.add(2)],
                [*angles, *angles.add(1), *angles.add(2)],
            )
        };
        // A view that is not a number reaches Metal as a matrix it cannot
        // build, so it is refused here rather than presenting a frame of
        // whatever the arithmetic produced.
        if !position.iter().chain(angles.iter()).all(|v| v.is_finite()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (handle, out_drawn);
            SOURCE_ABI_INVALID_HANDLE
        }

        #[cfg(target_os = "macos")]
        PRESENTERS.with(|presenters| {
            let presenters = presenters.borrow();
            let Some(presenter) = presenters.get(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let Some(scene) = presenter.scene.as_ref() else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let eye = source_render::Eye { position, angles };
            match scene.present(
                &presenter.device,
                &presenter.swapchain,
                eye,
                presenter.overlay.as_ref(),
            ) {
                Ok(drawn) => {
                    if !out_drawn.is_null() {
                        let drawn = SourceAbiWorldDraw {
                            batches: drawn.batches as u64,
                            triangles: (drawn.indices / 3) as u64,
                            map_triangles: scene.triangle_count() as u64,
                            materials: scene.bound_materials() as u64,
                            prop_triangles: drawn.prop_triangles as u64,
                            props: scene.prop_count() as u64,
                        };
                        // SAFETY: the caller guarantees writable storage.
                        unsafe { std::ptr::write(out_drawn, drawn) };
                    }
                    SOURCE_ABI_OK
                }
                Err(_) => SOURCE_ABI_INTERNAL_ERROR,
            }
        })
    })
}

/// Draws and presents one frame.
///
/// A bare clear, which is what the frames before a map loads are. Once a
/// map is loaded, [`source_render_world_present`] draws it.
///
/// # Safety
/// Must be called on the thread that created the presenter.
#[no_mangle]
pub unsafe extern "C" fn source_render_presenter_present(
    handle: SourceAbiHandle,
    red: f32,
    green: f32,
    blue: f32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if ![red, green, blue].iter().all(|c| c.is_finite()) {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = handle;
            SOURCE_ABI_INVALID_HANDLE
        }

        #[cfg(target_os = "macos")]
        PRESENTERS.with(|presenters| {
            let presenters = presenters.borrow();
            let Some(presenter) = presenters.get(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let color = source_render::ClearColor {
                red: f64::from(red),
                green: f64::from(green),
                blue: f64::from(blue),
                alpha: 1.0,
            };
            match presenter.device.present(&presenter.swapchain, color, &[]) {
                Ok(()) => SOURCE_ABI_OK,
                Err(_) => SOURCE_ABI_INTERNAL_ERROR,
            }
        })
    })
}

/// The size in pixels the presenter's layer currently draws at, which is
/// the window's size in points times its backing scale.
///
/// # Safety
/// Must be called on the thread that created the presenter, and both out
/// pointers must be writable.
#[no_mangle]
pub unsafe extern "C" fn source_render_presenter_drawable_size(
    handle: SourceAbiHandle,
    out_width: *mut u32,
    out_height: *mut u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if out_width.is_null() || out_height.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = handle;
            SOURCE_ABI_INVALID_HANDLE
        }

        #[cfg(target_os = "macos")]
        PRESENTERS.with(|presenters| {
            let presenters = presenters.borrow();
            let Some(presenter) = presenters.get(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let (width, height) = presenter.swapchain.size();
            unsafe {
                std::ptr::write(out_width, width);
                std::ptr::write(out_height, height);
            }
            SOURCE_ABI_OK
        })
    })
}

/// Discards the two-dimensional output gathered so far, which is how a
/// frame's interface starts.
///
/// Creating the overlay is deferred to here rather than done with the
/// presenter, because a run that never draws an interface should not pay
/// for a pipeline and a texture it will not use.
///
/// # Safety
///
/// `handle` must name a presenter this library created.
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn source_render_ui_begin(handle: SourceAbiHandle) -> SourceAbiStatus {
    ffi_status(|| {
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(presenter) = presenters.get_mut(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            if presenter.overlay.is_none() {
                match source_materialsystem::Overlay::new(&presenter.device) {
                    Ok(overlay) => presenter.overlay = Some(overlay),
                    Err(_) => return SOURCE_ABI_INTERNAL_ERROR,
                }
            }
            if let Some(overlay) = presenter.overlay.as_mut() {
                overlay.clear();
            }
            SOURCE_ABI_OK
        })
    })
}

/// Hands over the pixels of a texture the engine has rasterised, naming it
/// `id` for the rectangles that will sample it.
///
/// # Safety
///
/// `handle` must name a presenter this library created, and `rgba` must
/// point at `width * height * 4` readable bytes.
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn source_render_ui_texture(
    handle: SourceAbiHandle,
    id: u32,
    width: u32,
    height: u32,
    rgba: *const u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        if rgba.is_null() || width == 0 || height == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let Some(count) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|texels| texels.checked_mul(4))
        else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        // SAFETY: the caller guarantees this many readable bytes, and the
        // slice is only read before this call returns.
        let pixels = unsafe { std::slice::from_raw_parts(rgba, count) };
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(presenter) = presenters.get_mut(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let device = &presenter.device;
            let Some(overlay) = presenter.overlay.as_mut() else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            match overlay.set_texture(device, id, width, height, pixels) {
                Ok(()) => SOURCE_ABI_OK,
                Err(_) => SOURCE_ABI_INTERNAL_ERROR,
            }
        })
    })
}

/// Replaces a rectangle of a texture the engine has already handed over.
///
/// The engine rasterises a font sheet once and then draws each glyph into
/// it as that character is first asked for, so without this a sheet holds
/// only the characters that happened to be needed when it was made.
///
/// # Safety
///
/// `handle` must name a presenter this library created, and `rgba` must
/// point at `width * height * 4` readable bytes.
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn source_render_ui_texture_region(
    handle: SourceAbiHandle,
    id: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    rgba: *const u8,
) -> SourceAbiStatus {
    ffi_status(|| {
        if rgba.is_null() || width == 0 || height == 0 {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        let Some(count) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|texels| texels.checked_mul(4))
        else {
            return SOURCE_ABI_INVALID_ARGUMENT;
        };
        // SAFETY: the caller guarantees this many readable bytes, and the
        // slice is only read before this call returns.
        let pixels = unsafe { std::slice::from_raw_parts(rgba, count) };
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(presenter) = presenters.get_mut(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let device = &presenter.device;
            let Some(overlay) = presenter.overlay.as_mut() else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            match overlay.set_sub_texture(device, id, x, y, width, height, pixels) {
                Ok(()) => SOURCE_ABI_OK,
                Err(_) => SOURCE_ABI_INTERNAL_ERROR,
            }
        })
    })
}

/// Records that one texture identifier names the same pixels as another.
///
/// The engine makes two identifiers for every font sheet, one drawn
/// normally and one additively, and gives the pixels to only one of them.
///
/// # Safety
///
/// `handle` must name a presenter this library created.
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn source_render_ui_texture_alias(
    handle: SourceAbiHandle,
    alias: u32,
    base: u32,
) -> SourceAbiStatus {
    ffi_status(|| {
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(overlay) = presenters
                .get_mut(&handle)
                .and_then(|presenter| presenter.overlay.as_mut())
            else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            overlay.alias(alias, base);
            SOURCE_ABI_OK
        })
    })
}

/// Whether a texture identifier already holds pixels, so the engine can
/// skip handing over a sheet that has not changed.
///
/// # Safety
///
/// `handle` must name a presenter this library created.
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn source_render_ui_has_texture(handle: SourceAbiHandle, id: u32) -> i32 {
    PRESENTERS.with(|presenters| {
        let presenters = presenters.borrow();
        i32::from(
            presenters
                .get(&handle)
                .and_then(|presenter| presenter.overlay.as_ref())
                .is_some_and(|overlay| overlay.has_texture(id)),
        )
    })
}

/// Adds one screen-space rectangle to the frame being gathered.
///
/// `bounds` is left, top, right and bottom in pixels; `coords` is the
/// texture coordinate of each of those corners; `tint` is red, green, blue
/// and alpha from zero to one. A `texture` of zero draws the tint flat,
/// which is what a filled rectangle is.
///
/// # Safety
///
/// `handle` must name a presenter this library created, and `bounds`,
/// `coords` and `tint` must each point at four readable floats.
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn source_render_ui_quad(
    handle: SourceAbiHandle,
    texture: u32,
    bounds: *const f32,
    coords: *const f32,
    tint: *const f32,
) -> SourceAbiStatus {
    ffi_status(|| {
        if bounds.is_null() || coords.is_null() || tint.is_null() {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        // SAFETY: the caller guarantees four readable floats at each.
        let read = |values: *const f32| -> [f32; 4] {
            let values = unsafe { std::slice::from_raw_parts(values, 4) };
            std::array::from_fn(|slot| values[slot])
        };
        let (bounds, coords, tint) = (read(bounds), read(coords), read(tint));
        // A rectangle with a value that is not a number would reach the
        // device as one and take the process down, so it is refused here
        // where the caller can still be told.
        if [bounds, coords, tint]
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return SOURCE_ABI_INVALID_ARGUMENT;
        }
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(overlay) = presenters
                .get_mut(&handle)
                .and_then(|presenter| presenter.overlay.as_mut())
            else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            overlay.push(source_materialsystem::Quad {
                bounds,
                coords,
                tint,
                texture: (texture != 0).then_some(texture),
            });
            SOURCE_ABI_OK
        })
    })
}

/// Uploads the gathered rectangles so the next present draws them, and
/// reports how many there were.
///
/// # Safety
///
/// `handle` must name a presenter this library created. `out_quads`, when
/// not null, must point at writable storage for one `uint64_t`.
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn source_render_ui_end(
    handle: SourceAbiHandle,
    out_quads: *mut u64,
) -> SourceAbiStatus {
    ffi_status(|| {
        PRESENTERS.with(|presenters| {
            let mut presenters = presenters.borrow_mut();
            let Some(presenter) = presenters.get_mut(&handle) else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            let (width, height) = presenter.swapchain.size();
            let device = &presenter.device;
            let Some(overlay) = presenter.overlay.as_mut() else {
                return SOURCE_ABI_INVALID_HANDLE;
            };
            if overlay.upload(device, width as f32, height as f32).is_err() {
                return SOURCE_ABI_INTERNAL_ERROR;
            }
            if !out_quads.is_null() {
                // SAFETY: the caller guarantees writable storage.
                unsafe { std::ptr::write(out_quads, overlay.len() as u64) };
            }
            SOURCE_ABI_OK
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window cannot be made on a test thread, since AppKit requires the
    /// main one, so what is checked here is every way in which a caller
    /// can get this wrong. Those are the paths a C++ caller reaches by
    /// accident, and the ones where returning a status beats faulting
    /// somewhere else later.
    #[test]
    fn refuses_what_it_cannot_present_into() {
        let mut handle: SourceAbiHandle = 7;
        let refused = unsafe {
            source_render_presenter_create(std::ptr::null_mut(), 640, 480, 1.0, &mut handle)
        };
        assert_eq!(refused, SOURCE_ABI_INVALID_ARGUMENT, "a null window");

        // Not a real window, but the size and scale are checked before it
        // is ever messaged, so these never reach AppKit.
        let pretend = std::ptr::dangling_mut::<u8>().cast::<c_void>();
        for (width, height, scale, why) in [
            (0, 480, 1.0, "no width"),
            (640, 0, 1.0, "no height"),
            (640, 480, -1.0, "a negative scale"),
            (640, 480, f64::NAN, "a scale that is not a number"),
        ] {
            assert_eq!(
                unsafe {
                    source_render_presenter_create(pretend, width, height, scale, &mut handle)
                },
                SOURCE_ABI_INVALID_ARGUMENT,
                "{why}"
            );
        }

        assert_eq!(
            unsafe { source_render_presenter_create(pretend, 640, 480, 1.0, std::ptr::null_mut()) },
            SOURCE_ABI_INVALID_ARGUMENT,
            "nowhere to write the handle"
        );
    }

    /// A handle that was never handed out, or was handed out on another
    /// thread, is reported rather than used. The second case is the one
    /// worth having: presenters are per thread because AppKit and Metal
    /// require it, and a C++ caller holding a handle has no way to tell.
    #[test]
    fn refuses_handles_it_did_not_hand_out() {
        assert_eq!(
            unsafe { source_render_presenter_destroy(0) },
            SOURCE_ABI_INVALID_HANDLE
        );
        assert_eq!(
            unsafe { source_render_presenter_resize(99_999, 640, 480, 1.0) },
            SOURCE_ABI_INVALID_HANDLE
        );
        assert_eq!(
            unsafe { source_render_presenter_present(99_999, 0.0, 0.0, 0.0) },
            SOURCE_ABI_INVALID_HANDLE
        );
        let (mut width, mut height) = (0u32, 0u32);
        assert_eq!(
            unsafe { source_render_presenter_drawable_size(99_999, &mut width, &mut height) },
            SOURCE_ABI_INVALID_HANDLE
        );
    }

    #[test]
    fn refuses_a_frame_it_cannot_make_a_colour_from() {
        assert_eq!(
            unsafe { source_render_presenter_present(1, f32::NAN, 0.0, 0.0) },
            SOURCE_ABI_INVALID_ARGUMENT
        );
        assert_eq!(
            unsafe { source_render_presenter_resize(1, 0, 0, 1.0) },
            SOURCE_ABI_INVALID_ARGUMENT
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_no_backing_scale_from_nothing() {
        assert_eq!(
            unsafe { source_render::backing_scale_of(std::ptr::null_mut()) },
            None
        );
    }
}

