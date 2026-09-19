//! The presentation surface and its drawables.
//!
//! A swapchain here is a `CAMetalLayer`, which is what macOS presents through.
//! It is created standing alone rather than from a window, so the renderer can
//! be exercised and captured without one, and is then adopted by the engine's
//! SDL view when there is a window to draw into.

use crate::objc::{
    class, msg_send_ptr, selector, send_id, send_void_with_id, send_void_with_usize,
    send_void_with_value, AutoreleasePool, Id, Owned, Sel,
};
use crate::{DeviceError, Result};
use std::ffi::c_void;

/// A `CGSize`, which is what the layer measures its drawables in.
#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}

/// The surface frames are presented through.
pub struct Swapchain {
    layer: Owned,
    width: u32,
    height: u32,
    scale: f64,
}

impl Swapchain {
    /// The drawable size in pixels, which is the backing-store size rather
    /// than the size in points the window reports.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The backing scale factor, 2.0 on a Retina display.
    pub fn contents_scale(&self) -> f64 {
        self.scale
    }

    pub(crate) fn id(&self) -> Id {
        self.layer.as_id()
    }

    /// Resizes the drawables to `width` by `height` points at the current
    /// scale.
    ///
    /// Sizes are in points so callers pass what the window reports, and the
    /// scale is applied here; a window moved between displays changes scale
    /// without changing its size in points.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.configure(width, height, self.scale)
    }

    /// Sets the backing scale factor, for a move between a Retina display and
    /// a non-Retina one.
    pub fn set_contents_scale(&mut self, scale: f64) -> Result<()> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(DeviceError::InvalidScale(scale));
        }
        self.configure(self.width_in_points(), self.height_in_points(), scale)
    }

    fn width_in_points(&self) -> u32 {
        (f64::from(self.width) / self.scale).round() as u32
    }

    fn height_in_points(&self) -> u32 {
        (f64::from(self.height) / self.scale).round() as u32
    }

    fn configure(&mut self, width: u32, height: u32, scale: f64) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(DeviceError::EmptyTarget);
        }
        let pixels = Size {
            width: f64::from(width) * scale,
            height: f64::from(height) * scale,
        };

        let _pool = AutoreleasePool::new();
        // SAFETY: the layer is live and both selectors are sent with the
        // signatures CoreAnimation declares.
        unsafe {
            send_void_with_value(self.layer.as_id(), selector(c"setContentsScale:"), scale);
            send_void_with_value(self.layer.as_id(), selector(c"setDrawableSize:"), pixels);
        }

        self.width = pixels.width as u32;
        self.height = pixels.height as u32;
        self.scale = scale;
        Ok(())
    }

    /// Hands the layer to an `NSView`, so the view presents what this
    /// swapchain draws.
    ///
    /// # Safety
    /// `view` must be a live `NSView`, and this must be called on the main
    /// thread, which is where AppKit requires view changes to happen.
    pub unsafe fn attach_to_view(&self, view: *mut c_void) -> Result<()> {
        if view.is_null() {
            return Err(DeviceError::NoView);
        }
        let _pool = AutoreleasePool::new();
        // SAFETY: the caller guarantees a live view on the main thread, and
        // both selectors are sent with the signatures AppKit declares. Setting
        // the layer before asking for a layer-backed view is what makes the
        // view adopt this one rather than create its own.
        unsafe {
            let view = view.cast::<std::ffi::c_void>() as Id;
            send_void_with_id(view, selector(c"setLayer:"), self.layer.as_id());
            let set_wants_layer: extern "C" fn(Id, Sel, bool) = std::mem::transmute(msg_send_ptr());
            set_wants_layer(view, selector(c"setWantsLayer:"), true);
        }
        Ok(())
    }

    /// The same, for callers holding an `NSWindow` rather than its view.
    ///
    /// This exists so that the C++ side, which is where the engine's SDL
    /// window lives, never has to send an Objective-C message itself: it
    /// hands the window across and every message stays here.
    ///
    /// # Safety
    /// `window` must be a live `NSWindow`, and this must be called on the
    /// main thread.
    pub unsafe fn attach_to_window(&self, window: *mut c_void) -> Result<()> {
        if window.is_null() {
            return Err(DeviceError::NoView);
        }
        let _pool = AutoreleasePool::new();
        // SAFETY: the caller guarantees a live window on the main thread,
        // and `-contentView` returns a view the window owns, which is
        // borrowed only for the `attach_to_view` call below.
        let view = unsafe { send_id(window.cast::<c_void>() as Id, selector(c"contentView")) };
        if view.is_null() {
            return Err(DeviceError::NoView);
        }
        unsafe { self.attach_to_view(view.cast()) }
    }
}

/// How many pixels a point is on the display a window is on.
///
/// Read from the window rather than passed in, so that a caller placing
/// an engine window does not have to know whether the display is Retina.
///
/// # Safety
/// `window` must be a live `NSWindow`, on the main thread.
pub unsafe fn backing_scale_of(window: *mut c_void) -> Option<f64> {
    if window.is_null() {
        return None;
    }
    let _pool = AutoreleasePool::new();
    // SAFETY: the caller guarantees a live window, and
    // `-backingScaleFactor` returns a `CGFloat`, which is `f64` here.
    let scale = unsafe {
        let send: extern "C" fn(Id, Sel) -> f64 = std::mem::transmute(msg_send_ptr());
        send(
            window.cast::<c_void>() as Id,
            selector(c"backingScaleFactor"),
        )
    };
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

/// Creates a layer sized `width` by `height` points at `scale`.
///
/// # Safety
/// `device` must be a live Metal device, called inside an autorelease pool.
pub(crate) unsafe fn create(device: Id, width: u32, height: u32, scale: f64) -> Result<Swapchain> {
    if width == 0 || height == 0 {
        return Err(DeviceError::EmptyTarget);
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err(DeviceError::InvalidScale(scale));
    }

    // SAFETY: the class comes from the linked framework, `+layer` returns an
    // autoreleased layer that is retained here, and each selector below is
    // sent with the signature CoreAnimation declares.
    let layer = unsafe {
        let layer = send_id(class(c"CAMetalLayer"), selector(c"layer"));
        if layer.is_null() {
            return Err(DeviceError::NoSwapchain);
        }
        let layer = crate::objc::retain(layer).ok_or(DeviceError::NoSwapchain)?;

        send_void_with_id(layer.as_id(), selector(c"setDevice:"), device);
        send_void_with_usize(
            layer.as_id(),
            selector(c"setPixelFormat:"),
            crate::metal::PIXEL_FORMAT_BGRA8_UNORM,
        );
        // Drawables are readable, because the capture path copies out of one
        // to compare a presented frame against a reference. The cost is that
        // the driver cannot assume a drawable is only ever rendered into.
        let set_framebuffer_only: extern "C" fn(Id, Sel, bool) =
            std::mem::transmute(msg_send_ptr());
        set_framebuffer_only(layer.as_id(), selector(c"setFramebufferOnly:"), false);
        layer
    };

    let mut swapchain = Swapchain {
        layer,
        width,
        height,
        scale: 1.0,
    };
    swapchain.configure(width, height, scale)?;
    Ok(swapchain)
}

/// Acquires the next drawable to render into.
///
/// # Safety
/// Called inside an autorelease pool with a live layer.
pub(crate) unsafe fn next_drawable(swapchain: &Swapchain) -> Result<(Id, Id)> {
    // SAFETY: the layer is live and `nextDrawable` returns an autoreleased
    // drawable, or nil when none became free in time.
    unsafe {
        let drawable = send_id(swapchain.id(), selector(c"nextDrawable"));
        if drawable.is_null() {
            return Err(DeviceError::NoDrawable);
        }
        let texture = send_id(drawable, selector(c"texture"));
        if texture.is_null() {
            return Err(DeviceError::NoDrawable);
        }
        Ok((drawable, texture))
    }
}
