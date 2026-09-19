//! Metal device ownership and offscreen rendering.

use crate::objc::{
    class, msg_send_ptr, selector, send_cstr, send_id, send_id_with_id, send_id_with_usize,
    send_void, send_void_with_id, send_void_with_usize, send_void_with_value, AutoreleasePool, Id,
    Owned, Sel,
};
use crate::resource::{Buffer, Texture};
use crate::shader::{Library, Pipeline};
use crate::swapchain::Swapchain;
use crate::TextureFormat;
use crate::{ClearColor, DeviceError, Readback, Result, TriangleList, Vertices};
use std::ffi::{c_void, CStr};

#[link(name = "Metal", kind = "framework")]
extern "C" {
    fn MTLCreateSystemDefaultDevice() -> *mut c_void;
}

// Linked for its classes rather than any function: `CAMetalLayer` is only
// registered with the runtime once QuartzCore is loaded.
#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

/// `MTLPixelFormatBGRA8Unorm`, the format the display path uses, so offscreen
/// captures are comparable with what reaches a window.
pub(crate) const PIXEL_FORMAT_BGRA8_UNORM: u64 = 80;
/// `MTLPixelFormatDepth32Float`, the depth buffer world geometry resolves
/// visibility with.
pub(crate) const PIXEL_FORMAT_DEPTH32_FLOAT: u64 = 252;
/// `MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget`.
const TEXTURE_USAGE_SHADER_READ_RENDER_TARGET: u64 = 1 | 4;
/// `MTLTextureUsageRenderTarget`, all a depth buffer is ever read as.
const TEXTURE_USAGE_RENDER_TARGET: u64 = 4;
/// `MTLStorageModePrivate`: a depth buffer never leaves the GPU.
const STORAGE_MODE_PRIVATE: u64 = 2;
/// `MTLCompareFunctionLessEqual`, so a nearer surface wins and a surface
/// drawn twice at the same depth still shows.
const COMPARE_FUNCTION_LESS_EQUAL: u64 = 3;
/// `MTLStoreActionDontCare`: nothing reads the depth buffer after the pass.
const STORE_ACTION_DONT_CARE: u64 = 0;
/// `MTLStorageModeShared`, required for reading pixels back on the CPU.
pub(crate) const STORAGE_MODE_SHARED: u64 = 0;
const LOAD_ACTION_CLEAR: u64 = 2;
const STORE_ACTION_STORE: u64 = 1;
/// `MTLPrimitiveTypeTriangle`.
const PRIMITIVE_TYPE_TRIANGLE: u64 = 3;

pub const BYTES_PER_PIXEL: usize = 4;

/// A Metal region, laid out as the framework declares it.
#[repr(C)]
pub(crate) struct Region {
    origin_x: u64,
    origin_y: u64,
    origin_z: u64,
    width: u64,
    height: u64,
    depth: u64,
}

impl Region {
    /// The whole of a two-dimensional image.
    pub(crate) fn whole(width: u32, height: u32) -> Self {
        Self {
            origin_x: 0,
            origin_y: 0,
            origin_z: 0,
            width: u64::from(width),
            height: u64::from(height),
            depth: 1,
        }
    }
}

/// The GPU the renderer submits to, and one queue to submit through.
pub struct Device {
    device: Owned,
    queue: Owned,
    name: String,
}

impl Device {
    pub fn open() -> Result<Self> {
        // SAFETY: the Create rule applies, so the returned device carries a
        // reference this call owns; a machine without a Metal GPU returns null.
        let device = unsafe { Owned::from_owned(MTLCreateSystemDefaultDevice()) }
            .ok_or(DeviceError::NoDevice)?;

        let name = device_name(device.as_id()).ok_or(DeviceError::NoName)?;

        // SAFETY: a `new`-prefixed method returns a reference the caller owns,
        // and the device is live for the duration of the call.
        let queue = unsafe {
            let queue = send_id(device.as_id(), selector(c"newCommandQueue"));
            Owned::from_owned(queue)
        }
        .ok_or(DeviceError::NoQueue)?;

        Ok(Self {
            device,
            queue,
            name,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Compiles Metal Shading Language source into a shader library.
    pub fn compile_library(&self, source: &str) -> Result<Library> {
        let _pool = AutoreleasePool::new();
        // SAFETY: the device is live and a pool is in scope for the
        // autoreleased NSString and any NSError the compiler produces.
        unsafe { crate::shader::compile(self.device.as_id(), source) }
    }

    /// Builds a render pipeline from a vertex and fragment function, targeting
    /// the same pixel format the offscreen capture path uses.
    pub fn create_pipeline(
        &self,
        library: &Library,
        vertex: &str,
        fragment: &str,
    ) -> Result<Pipeline> {
        let _pool = AutoreleasePool::new();
        // SAFETY: as above, with a live library owned by the caller.
        unsafe {
            crate::shader::build_pipeline(
                self.device.as_id(),
                library,
                vertex,
                fragment,
                PIXEL_FORMAT_BGRA8_UNORM,
                false,
                false,
            )
        }
    }

    /// Builds a render pipeline that resolves visibility with a depth buffer.
    ///
    /// World geometry arrives in whatever order the BSP stores it, so which
    /// surface is in front cannot be decided by draw order the way a sorted
    /// two-dimensional overlay can.
    pub fn create_depth_pipeline(
        &self,
        library: &Library,
        vertex: &str,
        fragment: &str,
    ) -> Result<Pipeline> {
        let _pool = AutoreleasePool::new();
        // SAFETY: as above, with a live library owned by the caller.
        unsafe {
            crate::shader::build_pipeline(
                self.device.as_id(),
                library,
                vertex,
                fragment,
                PIXEL_FORMAT_BGRA8_UNORM,
                true,
                false,
            )
        }
    }

    /// Builds a render pipeline that composites over what is already drawn
    /// instead of replacing it.
    ///
    /// It carries a depth attachment like `create_depth_pipeline` even
    /// though a user interface has no depth of its own, because a pass
    /// either has that attachment or it does not and the interface is
    /// drawn into the same pass as the world it sits over. Quads are
    /// emitted at the near plane so they pass the comparison the world is
    /// drawn under rather than needing one of their own.
    pub fn create_blended_pipeline(
        &self,
        library: &Library,
        vertex: &str,
        fragment: &str,
    ) -> Result<Pipeline> {
        let _pool = AutoreleasePool::new();
        // SAFETY: as above, with a live library owned by the caller.
        unsafe {
            crate::shader::build_pipeline(
                self.device.as_id(),
                library,
                vertex,
                fragment,
                PIXEL_FORMAT_BGRA8_UNORM,
                true,
                true,
            )
        }
    }

    /// Copies bytes into a buffer the GPU can read as vertex data.
    pub fn create_buffer(&self, bytes: &[u8]) -> Result<Buffer> {
        let _pool = AutoreleasePool::new();
        // SAFETY: the device is live and a pool is in scope.
        unsafe { crate::resource::create_buffer(self.device.as_id(), bytes) }
    }

    /// Creates a sampleable texture holding `bytes` in the layout `format`
    /// describes, with no mip levels below the base.
    pub fn create_texture(
        &self,
        width: u32,
        height: u32,
        format: TextureFormat,
        bytes: &[u8],
    ) -> Result<Texture> {
        self.create_mipped_texture(width, height, format, &[bytes])
    }

    /// Creates a sampleable texture from a whole mip chain, `levels[0]` being
    /// the base level.
    ///
    /// The chain is uploaded as given rather than generated on the GPU,
    /// because VTF already stores the levels the material author shipped and
    /// regenerating them would discard that choice.
    pub fn create_mipped_texture(
        &self,
        width: u32,
        height: u32,
        format: TextureFormat,
        levels: &[&[u8]],
    ) -> Result<Texture> {
        let _pool = AutoreleasePool::new();
        // SAFETY: as above.
        unsafe {
            crate::resource::create_texture(self.device.as_id(), width, height, format, levels)
        }
    }

    /// Creates the surface frames are presented through, sized in points.
    ///
    /// It is not attached to a window yet; `Swapchain::attach_to_view` hands
    /// it to the engine's view when there is one.
    pub fn create_swapchain(&self, width: u32, height: u32, scale: f64) -> Result<Swapchain> {
        let _pool = AutoreleasePool::new();
        // SAFETY: the device is live and a pool is in scope for the
        // autoreleased layer.
        unsafe { crate::swapchain::create(self.device.as_id(), width, height, scale) }
    }

    /// Draws a frame and presents it.
    pub fn present(
        &self,
        swapchain: &Swapchain,
        color: ClearColor,
        draws: &[TriangleList<'_>],
    ) -> Result<()> {
        self.present_frame(swapchain, color, draws, false)
            .map(|_| ())
    }

    /// Draws a frame, presents it, and reads back what was presented.
    ///
    /// This is the frame capture the campaign comparison gates use: the
    /// pixels come from the drawable that actually reached the display rather
    /// than from a separate offscreen render that might diverge from it.
    pub fn present_capturing(
        &self,
        swapchain: &Swapchain,
        color: ClearColor,
        draws: &[TriangleList<'_>],
    ) -> Result<Readback> {
        self.present_frame(swapchain, color, draws, true)?
            .ok_or(DeviceError::NoDrawable)
    }

    fn present_frame(
        &self,
        swapchain: &Swapchain,
        color: ClearColor,
        draws: &[TriangleList<'_>],
        capture: bool,
    ) -> Result<Option<Readback>> {
        let wants_depth = check_draws(draws)?;

        let _pool = AutoreleasePool::new();
        let (width, height) = swapchain.size();

        let depth_target = wants_depth
            .then(|| self.new_depth_target(width, height))
            .transpose()?;
        let depth_state = wants_depth.then(|| self.new_depth_state()).transpose()?;

        // SAFETY: the drawable and its texture come from the live layer, and
        // every selector below is sent with the signature Metal declares.
        let copy = unsafe {
            let (drawable, texture) = crate::swapchain::next_drawable(swapchain)?;
            let pass = render_pass(texture, color, depth_target.as_ref().map(Owned::as_id))?;

            let buffer = send_id(self.queue.as_id(), selector(c"commandBuffer"));
            if buffer.is_null() {
                return Err(DeviceError::NoCommandBuffer);
            }
            let encoder = send_id_with_id(
                buffer,
                selector(c"renderCommandEncoderWithDescriptor:"),
                pass,
            );
            if encoder.is_null() {
                return Err(DeviceError::NoEncoder);
            }
            if let Some(state) = &depth_state {
                send_void_with_id(encoder, selector(c"setDepthStencilState:"), state.as_id());
            }
            for draw in draws {
                encode_triangles(encoder, draw);
            }
            send_void(encoder, selector(c"endEncoding"));

            // A drawable's own storage is not readable on the CPU, so a
            // capture copies it into a shared texture in the same submission.
            let copy = if capture {
                let copy = self.new_render_target(width, height)?;
                let blit = send_id(buffer, selector(c"blitCommandEncoder"));
                if blit.is_null() {
                    return Err(DeviceError::NoEncoder);
                }
                let copy_texture: extern "C" fn(Id, Sel, Id, Id) =
                    std::mem::transmute(msg_send_ptr());
                copy_texture(
                    blit,
                    selector(c"copyFromTexture:toTexture:"),
                    texture,
                    copy.as_id(),
                );
                send_void(blit, selector(c"endEncoding"));
                Some(copy)
            } else {
                None
            };

            send_void_with_id(buffer, selector(c"presentDrawable:"), drawable);
            send_void(buffer, selector(c"commit"));
            send_void(buffer, selector(c"waitUntilCompleted"));
            copy
        };

        match copy {
            Some(copy) => self.read_back(copy.as_id(), width, height).map(Some),
            None => Ok(None),
        }
    }

    /// Clears an offscreen target to `color` and reads the result back.
    pub fn clear_offscreen(&self, width: u32, height: u32, color: ClearColor) -> Result<Readback> {
        self.render_offscreen(width, height, color, &[])
    }

    /// Renders offscreen and reads the result back.
    ///
    /// This is the deterministic capture path the visual gates are built on:
    /// it runs without a window, waits for the GPU, and returns exact pixels
    /// rather than anything sampled from a display. With no triangles it is
    /// just the clear.
    pub fn render_offscreen(
        &self,
        width: u32,
        height: u32,
        color: ClearColor,
        draws: &[TriangleList<'_>],
    ) -> Result<Readback> {
        if width == 0 || height == 0 {
            return Err(DeviceError::EmptyTarget);
        }
        let wants_depth = check_draws(draws)?;

        // Metal's factory methods return autoreleased objects, so everything
        // below is drained when this scope ends.
        let _pool = AutoreleasePool::new();

        let texture = self.new_render_target(width, height)?;
        let depth_target = wants_depth
            .then(|| self.new_depth_target(width, height))
            .transpose()?;
        let depth_state = wants_depth.then(|| self.new_depth_state()).transpose()?;
        let pass = render_pass(
            texture.as_id(),
            color,
            depth_target.as_ref().map(Owned::as_id),
        )?;

        // SAFETY: each receiver below is a live object created above, and each
        // selector is sent with the signature Metal declares for it.
        unsafe {
            let buffer = send_id(self.queue.as_id(), selector(c"commandBuffer"));
            if buffer.is_null() {
                return Err(DeviceError::NoCommandBuffer);
            }
            let encoder = send_id_with_id(
                buffer,
                selector(c"renderCommandEncoderWithDescriptor:"),
                pass,
            );
            if encoder.is_null() {
                return Err(DeviceError::NoEncoder);
            }
            if let Some(state) = &depth_state {
                send_void_with_id(encoder, selector(c"setDepthStencilState:"), state.as_id());
            }
            for draw in draws {
                encode_triangles(encoder, draw);
            }
            send_void(encoder, selector(c"endEncoding"));
            send_void(buffer, selector(c"commit"));
            send_void(buffer, selector(c"waitUntilCompleted"));
        }

        self.read_back(texture.as_id(), width, height)
    }

    fn new_render_target(&self, width: u32, height: u32) -> Result<Owned> {
        // SAFETY: the descriptor class is looked up from the linked framework
        // and each selector is sent with Metal's declared signature. The
        // descriptor itself is autoreleased into the caller's pool.
        let texture = unsafe {
            let descriptor_class = class(c"MTLTextureDescriptor");
            let descriptor = texture_descriptor(
                descriptor_class,
                PIXEL_FORMAT_BGRA8_UNORM,
                u64::from(width),
                u64::from(height),
            );
            if descriptor.is_null() {
                return Err(DeviceError::NoTexture);
            }
            send_void_with_usize(
                descriptor,
                selector(c"setUsage:"),
                TEXTURE_USAGE_SHADER_READ_RENDER_TARGET,
            );
            // Shared storage is what makes the pixels readable without a blit.
            send_void_with_usize(
                descriptor,
                selector(c"setStorageMode:"),
                STORAGE_MODE_SHARED,
            );
            let texture = send_id_with_id(
                self.device.as_id(),
                selector(c"newTextureWithDescriptor:"),
                descriptor,
            );
            Owned::from_owned(texture)
        };
        texture.ok_or(DeviceError::NoTexture)
    }

    /// A depth buffer for one pass, sized to the color target it accompanies.
    ///
    /// It is private storage and discarded at the end of the pass, because
    /// nothing outside the pass ever reads it.
    fn new_depth_target(&self, width: u32, height: u32) -> Result<Owned> {
        // SAFETY: the descriptor class comes from the linked framework and
        // each selector is sent with Metal's declared signature.
        let texture = unsafe {
            let descriptor = texture_descriptor(
                class(c"MTLTextureDescriptor"),
                PIXEL_FORMAT_DEPTH32_FLOAT,
                u64::from(width),
                u64::from(height),
            );
            if descriptor.is_null() {
                return Err(DeviceError::NoTexture);
            }
            send_void_with_usize(
                descriptor,
                selector(c"setUsage:"),
                TEXTURE_USAGE_RENDER_TARGET,
            );
            send_void_with_usize(
                descriptor,
                selector(c"setStorageMode:"),
                STORAGE_MODE_PRIVATE,
            );
            let texture = send_id_with_id(
                self.device.as_id(),
                selector(c"newTextureWithDescriptor:"),
                descriptor,
            );
            Owned::from_owned(texture)
        };
        texture.ok_or(DeviceError::NoTexture)
    }

    /// The depth comparison world geometry is drawn under.
    fn new_depth_state(&self) -> Result<Owned> {
        // SAFETY: the descriptor class comes from the linked framework, each
        // selector is sent with Metal's declared signature, and the
        // `new`-prefixed method returns a reference this call owns.
        let state = unsafe {
            let descriptor = send_id(class(c"MTLDepthStencilDescriptor"), selector(c"new"));
            let descriptor = Owned::from_owned(descriptor).ok_or(DeviceError::NoDepthState)?;
            send_void_with_usize(
                descriptor.as_id(),
                selector(c"setDepthCompareFunction:"),
                COMPARE_FUNCTION_LESS_EQUAL,
            );
            let send: extern "C" fn(Id, Sel, bool) = std::mem::transmute(msg_send_ptr());
            send(descriptor.as_id(), selector(c"setDepthWriteEnabled:"), true);
            let state = send_id_with_id(
                self.device.as_id(),
                selector(c"newDepthStencilStateWithDescriptor:"),
                descriptor.as_id(),
            );
            Owned::from_owned(state)
        };
        state.ok_or(DeviceError::NoDepthState)
    }

    fn read_back(&self, texture: Id, width: u32, height: u32) -> Result<Readback> {
        let bytes_per_row = width as usize * BYTES_PER_PIXEL;
        let mut pixels = vec![0u8; bytes_per_row * height as usize];
        let region = Region {
            origin_x: 0,
            origin_y: 0,
            origin_z: 0,
            width: u64::from(width),
            height: u64::from(height),
            depth: 1,
        };

        // SAFETY: the texture is live and shared-storage, the buffer is sized
        // for exactly the region requested, and the signature matches Metal's
        // declaration of getBytes:bytesPerRow:fromRegion:mipmapLevel:.
        unsafe {
            let send: extern "C" fn(Id, crate::objc::Sel, *mut c_void, u64, Region, u64) =
                std::mem::transmute(crate::objc::msg_send_ptr());
            send(
                texture,
                selector(c"getBytes:bytesPerRow:fromRegion:mipmapLevel:"),
                pixels.as_mut_ptr().cast(),
                bytes_per_row as u64,
                region,
                0,
            );
        }

        Ok(Readback {
            width,
            height,
            pixels,
        })
    }
}

/// Refuses a draw that does not describe whole triangles, or one that would
/// read past the end of its vertex buffer, which is undefined rather than
/// merely wrong.
/// Checks every draw in a pass and reports whether the pass needs a depth
/// buffer.
///
/// Metal fixes a pass's depth attachment for the whole pass, so the draws in
/// it have to agree about whether there is one; a pipeline built without
/// depth cannot be used in a pass that has it, or the reverse.
fn check_draws(draws: &[TriangleList<'_>]) -> Result<bool> {
    for draw in draws {
        check_triangles(Some(draw))?;
    }
    let with_depth = draws.iter().filter(|draw| draw.pipeline.depth).count();
    if with_depth != 0 && with_depth != draws.len() {
        return Err(DeviceError::MixedDepthPipelines {
            with_depth,
            draws: draws.len(),
        });
    }
    Ok(with_depth != 0)
}

fn check_triangles(triangles: Option<&TriangleList<'_>>) -> Result<()> {
    let Some(triangles) = triangles else {
        return Ok(());
    };
    // An indexed draw reads whole triangles out of the index buffer, so it is
    // the index count that has to divide by three; the vertices it selects
    // from need not, and commonly do not.
    let count = match &triangles.indices {
        Some(indices) => indices.count,
        None => triangles.vertices.count(),
    };
    if count == 0 || count % 3 != 0 {
        return Err(DeviceError::NotTriangles(count));
    }
    if let Vertices::Buffered {
        buffer,
        count,
        stride,
    } = &triangles.vertices
    {
        if *stride == 0 {
            return Err(DeviceError::EmptyVertexStride);
        }
        let needed = count * stride;
        if buffer.len() < needed {
            return Err(DeviceError::VertexBufferTooSmall {
                needed,
                found: buffer.len(),
            });
        }
    }
    // Every per-vertex attribute is read once per vertex the draw selects, so
    // each has to hold as many values as there are vertices even when the
    // draw only indexes some of them.
    for attribute in [
        triangles.coordinates,
        triangles.lightmap_coordinates,
        triangles.shade,
    ]
    .into_iter()
    .flatten()
    {
        if attribute.stride == 0 {
            return Err(DeviceError::EmptyVertexStride);
        }
        let needed = triangles
            .vertices
            .count()
            .checked_mul(attribute.stride)
            .ok_or(DeviceError::IndexRunOverflow)?;
        if attribute.buffer.len() < needed {
            return Err(DeviceError::VertexBufferTooSmall {
                needed,
                found: attribute.buffer.len(),
            });
        }
    }
    if let Some(uniforms) = triangles.uniforms {
        if uniforms.len() > crate::MAX_INLINE_UNIFORM_BYTES {
            return Err(DeviceError::UniformsTooLarge {
                bytes: uniforms.len(),
                limit: crate::MAX_INLINE_UNIFORM_BYTES,
            });
        }
    }
    if let Some(indices) = &triangles.indices {
        // A draw reads its own run out of the buffer, so what has to fit is
        // the end of that run rather than its length.
        let end = indices
            .first
            .checked_add(indices.count)
            .ok_or(DeviceError::IndexRunOverflow)?;
        let needed = end * indices.format.size();
        if indices.buffer.len() < needed {
            return Err(DeviceError::IndexBufferTooSmall {
                needed,
                found: indices.buffer.len(),
            });
        }
        // An index past the end of the vertex data faults the GPU rather than
        // drawing anything, so the indices are checked against the vertices
        // they select from before the draw is encoded. Shared storage makes
        // reading them back here a plain memory read.
        let vertex_count = triangles.vertices.count();
        // SAFETY: the buffer was created from a slice of at least `needed`
        // bytes, it uses shared storage so its contents are host-visible, and
        // it is borrowed for the length of this call.
        let largest = unsafe {
            indices
                .buffer
                .largest_index(indices.first, indices.count, indices.format)
        };
        if let Some(largest) = largest {
            if largest >= vertex_count {
                return Err(DeviceError::IndexOutOfRange {
                    index: largest,
                    vertices: vertex_count,
                });
            }
        }
    }
    Ok(())
}

/// Binds a pipeline, its vertex positions and any texture, then draws.
///
/// Positions are passed inline rather than through a buffer object, which is
/// what Metal's small-payload path is for and keeps the vertex data owned by
/// the caller's slice. Larger geometry binds a [`Buffer`] instead.
///
/// # Safety
/// `encoder` must be a live render command encoder that has not ended.
unsafe fn encode_triangles(encoder: Id, triangles: &TriangleList<'_>) {
    // SAFETY: every selector below is sent with the signature Metal declares,
    // and any pointer passed is valid for the length given because the slice
    // it comes from outlives the call.
    unsafe {
        send_void_with_id(
            encoder,
            selector(c"setRenderPipelineState:"),
            triangles.pipeline.state.as_id(),
        );

        let vertex_count = match &triangles.vertices {
            Vertices::Inline(positions) => {
                let set_vertex_bytes: extern "C" fn(Id, Sel, *const c_void, u64, u64) =
                    std::mem::transmute(msg_send_ptr());
                set_vertex_bytes(
                    encoder,
                    selector(c"setVertexBytes:length:atIndex:"),
                    positions.as_ptr().cast(),
                    std::mem::size_of_val(*positions) as u64,
                    0,
                );
                positions.len()
            }
            Vertices::Buffered { buffer, count, .. } => {
                let set_vertex_buffer: extern "C" fn(Id, Sel, Id, u64, u64) =
                    std::mem::transmute(msg_send_ptr());
                set_vertex_buffer(
                    encoder,
                    selector(c"setVertexBuffer:offset:atIndex:"),
                    buffer.id.as_id(),
                    0,
                    0,
                );
                *count
            }
        };

        for (attribute, index) in [
            (triangles.coordinates, crate::COORDINATE_BUFFER_INDEX),
            (
                triangles.lightmap_coordinates,
                crate::LIGHTMAP_COORDINATE_BUFFER_INDEX,
            ),
            (triangles.shade, crate::SHADE_BUFFER_INDEX),
        ] {
            let Some(attribute) = attribute else {
                continue;
            };
            let set_vertex_buffer: extern "C" fn(Id, Sel, Id, u64, u64) =
                std::mem::transmute(msg_send_ptr());
            set_vertex_buffer(
                encoder,
                selector(c"setVertexBuffer:offset:atIndex:"),
                attribute.buffer.id.as_id(),
                0,
                index,
            );
        }

        if let Some(uniforms) = triangles.uniforms {
            let set_vertex_bytes: extern "C" fn(Id, Sel, *const c_void, u64, u64) =
                std::mem::transmute(msg_send_ptr());
            set_vertex_bytes(
                encoder,
                selector(c"setVertexBytes:length:atIndex:"),
                uniforms.as_ptr().cast(),
                uniforms.len() as u64,
                crate::UNIFORM_BUFFER_INDEX,
            );
        }

        for (texture, index) in [
            (triangles.texture, crate::TEXTURE_INDEX),
            (triangles.lightmap, crate::LIGHTMAP_TEXTURE_INDEX),
        ] {
            let Some(texture) = texture else {
                continue;
            };
            let set_texture: extern "C" fn(Id, Sel, Id, u64) = std::mem::transmute(msg_send_ptr());
            set_texture(
                encoder,
                selector(c"setFragmentTexture:atIndex:"),
                texture.id.as_id(),
                index,
            );
        }

        match &triangles.indices {
            None => {
                let draw: extern "C" fn(Id, Sel, u64, u64, u64) =
                    std::mem::transmute(msg_send_ptr());
                draw(
                    encoder,
                    selector(c"drawPrimitives:vertexStart:vertexCount:"),
                    PRIMITIVE_TYPE_TRIANGLE,
                    0,
                    vertex_count as u64,
                );
            }
            Some(indices) => {
                let draw: extern "C" fn(Id, Sel, u64, u64, u64, Id, u64) =
                    std::mem::transmute(msg_send_ptr());
                draw(
                    encoder,
                    selector(
                        c"drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:",
                    ),
                    PRIMITIVE_TYPE_TRIANGLE,
                    indices.count as u64,
                    indices.format.metal(),
                    indices.buffer.id.as_id(),
                    // Metal takes the start as a byte offset, so a run that
                    // begins at index `first` begins that many index widths
                    // into the buffer.
                    (indices.first * indices.format.size()) as u64,
                );
            }
        }
    }
}

/// Builds a render pass that clears `texture` and keeps the result.
fn render_pass(texture: Id, color: ClearColor, depth: Option<Id>) -> Result<Id> {
    // SAFETY: the class comes from the linked framework, and every selector
    // below is sent with the signature Metal declares. The descriptor is
    // autoreleased into the caller's pool.
    unsafe {
        let pass = send_id(
            class(c"MTLRenderPassDescriptor"),
            selector(c"renderPassDescriptor"),
        );
        if pass.is_null() {
            return Err(DeviceError::NoRenderPass);
        }
        let attachments = send_id(pass, selector(c"colorAttachments"));
        let attachment = send_id_with_usize(attachments, selector(c"objectAtIndexedSubscript:"), 0);
        if attachment.is_null() {
            return Err(DeviceError::NoRenderPass);
        }
        send_void_with_id(attachment, selector(c"setTexture:"), texture);
        send_void_with_usize(attachment, selector(c"setLoadAction:"), LOAD_ACTION_CLEAR);
        send_void_with_usize(attachment, selector(c"setStoreAction:"), STORE_ACTION_STORE);
        send_void_with_value(attachment, selector(c"setClearColor:"), color);

        if let Some(depth) = depth {
            let attachment = send_id(pass, selector(c"depthAttachment"));
            if attachment.is_null() {
                return Err(DeviceError::NoRenderPass);
            }
            send_void_with_id(attachment, selector(c"setTexture:"), depth);
            send_void_with_usize(attachment, selector(c"setLoadAction:"), LOAD_ACTION_CLEAR);
            send_void_with_usize(
                attachment,
                selector(c"setStoreAction:"),
                STORE_ACTION_DONT_CARE,
            );
            // Clearing to the far plane means the first surface at any pixel
            // always passes, and every later one is compared against it.
            let send: extern "C" fn(Id, Sel, f64) = std::mem::transmute(msg_send_ptr());
            send(attachment, selector(c"setClearDepth:"), 1.0);
        }

        Ok(pass)
    }
}

/// Sends `texture2DDescriptorWithPixelFormat:width:height:mipmapped:`.
///
/// # Safety
/// `class` must be `MTLTextureDescriptor` or null.
pub(crate) unsafe fn texture_descriptor(class: Id, format: u64, width: u64, height: u64) -> Id {
    if class.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: the signature matches Metal's declaration; the trailing argument
    // is the method's BOOL, which is a byte on this platform.
    let send: extern "C" fn(Id, crate::objc::Sel, u64, u64, u64, bool) -> Id =
        unsafe { std::mem::transmute(crate::objc::msg_send_ptr()) };
    send(
        class,
        selector(c"texture2DDescriptorWithPixelFormat:width:height:mipmapped:"),
        format,
        width,
        height,
        false,
    )
}

fn device_name(device: Id) -> Option<String> {
    // SAFETY: `name` returns an NSString the device owns, and `UTF8String`
    // returns a C string valid until the autorelease pool drains, which is
    // after this function copies out of it.
    let text = unsafe {
        let string = send_id(device, selector(c"name"));
        send_cstr(string, selector(c"UTF8String"))
    };
    if text.is_null() {
        return None;
    }
    // SAFETY: the runtime guarantees a NUL-terminated string here.
    Some(
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned(),
    )
}
