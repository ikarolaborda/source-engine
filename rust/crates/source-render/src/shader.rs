//! Shader compilation and render pipeline state.
//!
//! Shaders are compiled from Metal Shading Language source at runtime here.
//! Offline packaging into a `.metallib` is a separate delivery slice; this is
//! what makes a pipeline testable without a content build first.

use crate::objc::{
    class, msg_send_ptr, selector, send_cstr, send_id, send_id_with_cstr, send_id_with_id, Id,
    Owned, Sel,
};
use crate::{DeviceError, Result};
use std::ffi::{CStr, CString};

/// A compiled collection of shader functions.
pub struct Library(pub(crate) Owned);

/// A render pipeline built from a vertex and fragment function.
pub struct Pipeline {
    pub(crate) state: Owned,
    /// Whether this pipeline was built to write depth. A pass has to attach a
    /// depth buffer exactly when its pipeline expects one, so the draw path
    /// reads this rather than asking the caller to keep the two in step.
    pub(crate) depth: bool,
}

/// Builds an autoreleased `NSString` from Rust text.
///
/// Returns null when the text contains an interior NUL, which cannot be
/// represented as a C string.
pub(crate) fn ns_string(text: &str) -> Id {
    let Ok(text) = CString::new(text) else {
        return std::ptr::null_mut();
    };
    // SAFETY: the class comes from a linked framework and the pointer is a
    // valid NUL-terminated C string for the duration of the call. The result
    // is autoreleased into the caller's pool.
    unsafe {
        send_id_with_cstr(
            class(c"NSString"),
            selector(c"stringWithUTF8String:"),
            text.as_ptr(),
        )
    }
}

/// Reads an `NSError`'s description, for reporting a compile or link failure
/// with the message the compiler actually produced.
pub(crate) fn error_description(error: Id) -> String {
    if error.is_null() {
        return "no error description".to_owned();
    }
    // SAFETY: `localizedDescription` returns an NSString the error owns, and
    // `UTF8String` is valid until the pool drains, which is after the copy.
    let text = unsafe {
        let description = send_id(error, selector(c"localizedDescription"));
        send_cstr(description, selector(c"UTF8String"))
    };
    if text.is_null() {
        return "no error description".to_owned();
    }
    // SAFETY: the runtime guarantees a NUL-terminated string here.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

/// Compiles Metal Shading Language source into a library.
///
/// # Safety
/// `device` must be a live Metal device, called inside an autorelease pool.
pub(crate) unsafe fn compile(device: Id, source: &str) -> Result<Library> {
    let source = ns_string(source);
    if source.is_null() {
        return Err(DeviceError::ShaderSourceNotRepresentable);
    }

    let mut error: Id = std::ptr::null_mut();
    // SAFETY: the signature matches Metal's declaration of
    // newLibraryWithSource:options:error:, which writes an NSError through the
    // out parameter on failure and returns an owned library on success.
    let library = unsafe {
        let send: extern "C" fn(Id, Sel, Id, Id, *mut Id) -> Id =
            std::mem::transmute(msg_send_ptr());
        let library = send(
            device,
            selector(c"newLibraryWithSource:options:error:"),
            source,
            std::ptr::null_mut(),
            &mut error,
        );
        Owned::from_owned(library)
    };

    match library {
        Some(library) => Ok(Library(library)),
        None => Err(DeviceError::ShaderCompilationFailed(error_description(
            error,
        ))),
    }
}

/// Builds a render pipeline for one color attachment format.
///
/// # Safety
/// `device` must be a live Metal device, called inside an autorelease pool.
pub(crate) unsafe fn build_pipeline(
    device: Id,
    library: &Library,
    vertex: &str,
    fragment: &str,
    pixel_format: u64,
    depth: bool,
) -> Result<Pipeline> {
    // SAFETY: each receiver is live and each selector is sent with the
    // signature Metal declares for it.
    unsafe {
        let vertex_function = shader_function(library, vertex)?;
        let fragment_function = shader_function(library, fragment)?;

        let descriptor = send_id(class(c"MTLRenderPipelineDescriptor"), selector(c"new"));
        let descriptor = Owned::from_owned(descriptor).ok_or(DeviceError::NoPipelineDescriptor)?;

        crate::objc::send_void_with_id(
            descriptor.as_id(),
            selector(c"setVertexFunction:"),
            vertex_function.as_id(),
        );
        crate::objc::send_void_with_id(
            descriptor.as_id(),
            selector(c"setFragmentFunction:"),
            fragment_function.as_id(),
        );

        let attachments = send_id(descriptor.as_id(), selector(c"colorAttachments"));
        let attachment =
            crate::objc::send_id_with_usize(attachments, selector(c"objectAtIndexedSubscript:"), 0);
        if attachment.is_null() {
            return Err(DeviceError::NoPipelineDescriptor);
        }
        crate::objc::send_void_with_usize(attachment, selector(c"setPixelFormat:"), pixel_format);
        if depth {
            crate::objc::send_void_with_usize(
                descriptor.as_id(),
                selector(c"setDepthAttachmentPixelFormat:"),
                crate::metal::PIXEL_FORMAT_DEPTH32_FLOAT,
            );
        }

        let mut error: Id = std::ptr::null_mut();
        let send: extern "C" fn(Id, Sel, Id, *mut Id) -> Id = std::mem::transmute(msg_send_ptr());
        let pipeline = send(
            device,
            selector(c"newRenderPipelineStateWithDescriptor:error:"),
            descriptor.as_id(),
            &mut error,
        );
        match Owned::from_owned(pipeline) {
            Some(state) => Ok(Pipeline { state, depth }),
            None => Err(DeviceError::PipelineCreationFailed(error_description(
                error,
            ))),
        }
    }
}

/// Looks up one function in a library by name.
///
/// # Safety
/// Called inside an autorelease pool with a live library.
unsafe fn shader_function(library: &Library, name: &str) -> Result<Owned> {
    let requested = ns_string(name);
    if requested.is_null() {
        return Err(DeviceError::ShaderSourceNotRepresentable);
    }
    // SAFETY: a `new`-prefixed method returns a reference this call owns.
    let function = unsafe {
        let function = send_id_with_id(
            library.0.as_id(),
            selector(c"newFunctionWithName:"),
            requested,
        );
        Owned::from_owned(function)
    };
    function.ok_or_else(|| DeviceError::ShaderFunctionMissing(name.to_owned()))
}
