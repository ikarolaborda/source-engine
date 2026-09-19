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

/// Draws and presents one frame.
///
/// Only a clear for now, which is what proves the window is Metal-backed
/// and being presented into at all. The scene goes here.
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
