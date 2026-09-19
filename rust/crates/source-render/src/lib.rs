//! Rust-owned rendering device.
//!
//! The permanent presentation stack replaces the deprecated ToGL adapter with
//! a native Metal backend on Apple silicon. Metal is reached directly through
//! a minimal Objective-C binding in this crate rather than through an external
//! graphics abstraction, so the renderer adds no dependencies and nothing
//! stands between the engine and the platform API.
//!
//! Only device and queue ownership exists so far. ToGL remains the rendering
//! path and the visual oracle until this earns parity.

use std::fmt;

mod camera;
#[cfg(target_os = "macos")]
mod metal;
#[cfg(target_os = "macos")]
mod objc;
#[cfg(target_os = "macos")]
mod resource;
#[cfg(target_os = "macos")]
mod shader;
#[cfg(target_os = "macos")]
mod swapchain;

pub use camera::{view_projection, Eye, Lens, LensError, Matrix};
#[cfg(target_os = "macos")]
pub use metal::Device;
#[cfg(target_os = "macos")]
pub use resource::{Buffer, IndexFormat, Texture};
#[cfg(target_os = "macos")]
pub use shader::{Library, Pipeline};
#[cfg(target_os = "macos")]
pub use swapchain::{backing_scale_of, Swapchain};

/// How a texture's pixels are laid out.
///
/// The block-compressed entries are the ones HL2's VTFs are overwhelmingly
/// stored in, and Apple silicon samples them natively, so those payloads
/// upload as they are found on disk rather than being decoded on the CPU
/// first.
///
/// This describes the stored image rather than the device, so it is the same
/// on every platform; only the mapping to a Metal format is backend code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFormat {
    /// Eight bits per channel, blue first, matching the capture readback.
    Bgra8Unorm,
    /// `DXT1`.
    Bc1Rgba,
    /// `DXT3`, whose alpha is stored uncompressed per pixel.
    Bc2Rgba,
    /// `DXT5`.
    Bc3Rgba,
}

impl TextureFormat {
    /// The width and height, in pixels, that one stored block covers.
    pub fn block_extent(self) -> u32 {
        match self {
            Self::Bgra8Unorm => 1,
            Self::Bc1Rgba | Self::Bc2Rgba | Self::Bc3Rgba => 4,
        }
    }

    /// The stored size of one block, which with the extent above is what a
    /// caller needs to size an upload before it has a device to ask.
    pub fn bytes_per_block(self) -> usize {
        match self {
            Self::Bgra8Unorm => 4,
            Self::Bc1Rgba => 8,
            Self::Bc2Rgba | Self::Bc3Rgba => 16,
        }
    }
}

/// Where a draw reads its clip-space positions from.
///
/// Small geometry is cheaper to pass inline than to stage through a buffer,
/// and Metal caps that path at a few kilobytes, so anything larger has to own
/// a [`Buffer`].
#[cfg(target_os = "macos")]
pub enum Vertices<'a> {
    Inline(&'a [[f32; 2]]),
    Buffered {
        buffer: &'a Buffer,
        count: usize,
        /// Bytes per vertex. World geometry is three floats of position where
        /// an overlay is two, and the buffer can only be checked against the
        /// count when the size of one vertex is known.
        stride: usize,
    },
}

#[cfg(target_os = "macos")]
impl Vertices<'_> {
    pub fn count(&self) -> usize {
        match self {
            Self::Inline(positions) => positions.len(),
            Self::Buffered { count, .. } => *count,
        }
    }
}

/// Indices selecting which vertices each triangle uses.
///
/// World geometry shares most of its vertices between neighbouring triangles,
/// so drawing it from an index buffer is what keeps a surface's vertex data
/// down to roughly what the BSP actually stores.
#[cfg(target_os = "macos")]
pub struct Indices<'a> {
    pub buffer: &'a Buffer,
    pub count: usize,
    pub format: IndexFormat,
    /// The index this draw starts at. A map's world is one index buffer
    /// grouped into a run per material, so each draw takes its own run out
    /// of the whole rather than owning a buffer of its own.
    pub first: usize,
}

/// One per-vertex attribute held in its own buffer.
///
/// The stride is stated rather than assumed because only the shader knows
/// how the buffer is laid out, and without it a buffer too short for the
/// draw cannot be refused before the GPU reads past its end.
#[derive(Clone, Copy)]
pub struct VertexAttribute<'a> {
    pub buffer: &'a Buffer,
    pub stride: usize,
}

impl<'a> VertexAttribute<'a> {
    /// An attribute of tightly packed values of one type.
    pub fn packed<T>(buffer: &'a Buffer) -> Self {
        Self {
            buffer,
            stride: std::mem::size_of::<T>(),
        }
    }
}

impl<'a> TriangleList<'a> {
    /// Triangles drawn from their positions alone, with nothing bound.
    ///
    /// The rest of a draw is added by the methods below, so that a caller
    /// states what it uses and nothing else. Spelling every binding out at
    /// every call site made adding one a change to all of them, and made a
    /// draw that binds nothing read as though it had made choices.
    pub fn new(pipeline: &'a Pipeline, vertices: Vertices<'a>) -> Self {
        Self {
            pipeline,
            vertices,
            texture: None,
            coordinates: None,
            lightmap: None,
            lightmap_coordinates: None,
            shade: None,
            indices: None,
            uniforms: None,
        }
    }

    /// The texture the fragment stage samples.
    pub fn with_texture(mut self, texture: &'a Texture) -> Self {
        self.texture = Some(texture);
        self
    }

    /// Where each vertex samples that texture. A shader that works the
    /// coordinates out from the position itself needs none.
    pub fn with_coordinates(mut self, coordinates: VertexAttribute<'a>) -> Self {
        self.coordinates = Some(coordinates);
        self
    }

    /// The baked lighting to modulate by, and where each vertex falls in it.
    ///
    /// The two are taken together because a packed lightmap holds one block
    /// per surface, and nothing but these coordinates can say which.
    pub fn with_lightmap(mut self, lightmap: &'a Texture, at: VertexAttribute<'a>) -> Self {
        self.lightmap = Some(lightmap);
        self.lightmap_coordinates = Some(at);
        self
    }

    /// A colour per vertex, which the fragment stage modulates the
    /// material by.
    ///
    /// This is how Source lights what a lightmap cannot: a prop, a player
    /// or anything else the compiler did not know the position of when it
    /// baked the map, whose light is worked out per vertex from the
    /// ambient cube of the leaf it stands in.
    pub fn with_shade(mut self, shade: VertexAttribute<'a>) -> Self {
        self.shade = Some(shade);
        self
    }

    /// Draw the positions indirectly, through a run of an index buffer.
    pub fn with_indices(mut self, indices: Indices<'a>) -> Self {
        self.indices = Some(indices);
        self
    }

    /// Constants every vertex reads.
    pub fn with_uniforms(mut self, uniforms: &'a [u8]) -> Self {
        self.uniforms = Some(uniforms);
        self
    }
}

/// Triangles to draw: a pipeline, world or clip-space positions, the texture
/// the fragment shader samples if it takes one, the coordinates it samples
/// at, and indices if the positions are to be drawn indirectly rather than
/// in order.
#[cfg(target_os = "macos")]
pub struct TriangleList<'a> {
    pub pipeline: &'a Pipeline,
    pub vertices: Vertices<'a>,
    pub texture: Option<&'a Texture>,
    /// Per-vertex texture coordinates, in their own buffer because that is
    /// how a map stores them: one coordinate per position, written
    /// separately from the positions themselves.
    pub coordinates: Option<VertexAttribute<'a>>,
    /// The baked lighting the material is modulated by, if the surface has
    /// any. A draw without one comes out flat, which is a surface's albedo
    /// rather than how it looks in the world.
    pub lightmap: Option<&'a Texture>,
    /// Per-vertex coordinates into that lighting. These are separate from
    /// the material's because the two are projected onto a surface at
    /// different scales: one lightmap sample covers many texels.
    pub lightmap_coordinates: Option<VertexAttribute<'a>>,
    /// A colour per vertex the fragment stage modulates by. See
    /// [`TriangleList::with_shade`].
    pub shade: Option<VertexAttribute<'a>>,
    pub indices: Option<Indices<'a>>,
    /// Constants every vertex reads, such as the world-to-clip transform.
    /// They are passed inline because Metal's small-payload path exists for
    /// exactly this and a transform is smaller than a cache line.
    pub uniforms: Option<&'a [u8]>,
}

/// The vertex buffer index a draw's uniforms are bound at, which the shader
/// declares as `buffer(1)` because the positions themselves take `buffer(0)`.
#[cfg(target_os = "macos")]
pub const UNIFORM_BUFFER_INDEX: u64 = 1;

/// The vertex buffer index a draw's texture coordinates are bound at,
/// after the positions at zero and the uniforms at one.
#[cfg(target_os = "macos")]
pub const COORDINATE_BUFFER_INDEX: u64 = 2;

/// The vertex buffer index a draw's lightmap coordinates are bound at.
#[cfg(target_os = "macos")]
pub const LIGHTMAP_COORDINATE_BUFFER_INDEX: u64 = 3;

/// The vertex buffer index a draw's per-vertex colours are bound at.
#[cfg(target_os = "macos")]
pub const SHADE_BUFFER_INDEX: u64 = 4;

/// The fragment texture index the material's own texture is bound at, which
/// the shader declares as `texture(0)`.
#[cfg(target_os = "macos")]
pub const TEXTURE_INDEX: u64 = 0;

/// The fragment texture index the lightmap is bound at.
#[cfg(target_os = "macos")]
pub const LIGHTMAP_TEXTURE_INDEX: u64 = 1;

/// Metal's cap on a `setVertexBytes:` payload.
#[cfg(target_os = "macos")]
pub const MAX_INLINE_UNIFORM_BYTES: usize = 4096;

pub type Result<T> = std::result::Result<T, DeviceError>;

/// A clear color in the layout Metal declares, so it can be passed straight
/// through to `setClearColor:`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClearColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

/// Pixels captured from an offscreen target, as BGRA bytes in row order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readback {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Readback {
    /// The BGRA bytes of one pixel, or `None` when it is outside the target.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels
            .get(offset..offset + 4)
            .map(|bytes| [bytes[0], bytes[1], bytes[2], bytes[3]])
    }
}

/// Not `Copy`, because the shader failures carry the compiler's own message;
/// a pipeline that will not build is not worth reporting without it. Not `Eq`
/// either, since a rejected scale factor may be NaN, which is exactly the
/// value that is not equal to itself.
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceError {
    /// The platform has no Metal backend compiled in.
    UnsupportedPlatform,
    /// The machine reported no Metal GPU.
    NoDevice,
    /// The device would not report a name, which means it is not usable.
    NoName,
    /// The device would not create a command queue.
    NoQueue,
    /// A zero-sized target was requested.
    EmptyTarget,
    /// The device would not create the offscreen texture.
    NoTexture,
    /// The queue would not create a command buffer.
    NoCommandBuffer,
    /// The command buffer would not create a render encoder.
    NoEncoder,
    /// The render pass could not be described.
    NoRenderPass,
    /// A vertex count that is not whole triangles was requested.
    NotTriangles(usize),
    /// Shader source or a function name contained an interior NUL, so it
    /// cannot cross into Objective-C.
    ShaderSourceNotRepresentable,
    /// The Metal compiler rejected the source, with its message.
    ShaderCompilationFailed(String),
    /// The library holds no function under that name.
    ShaderFunctionMissing(String),
    /// The pipeline descriptor could not be built.
    NoPipelineDescriptor,
    /// Metal rejected the pipeline, with its message.
    PipelineCreationFailed(String),
    /// A resource was requested with no contents.
    EmptyResource,
    /// The device would not create the buffer.
    NoBuffer,
    /// The pixels handed over do not fill the texture requested.
    TextureSizeMismatch {
        expected: usize,
        found: usize,
    },
    /// A compressed texture was asked for at a size that is not whole blocks.
    UnalignedTexture {
        width: u32,
        height: u32,
        block: u32,
    },
    /// One level of a mip chain did not hold what its own size needs.
    LevelSizeMismatch {
        level: usize,
        expected: usize,
        found: usize,
    },
    /// More mip levels were given than halving reaches a single pixel in.
    TooManyLevels {
        found: usize,
        available: usize,
    },
    /// The draw would read past the end of its vertex buffer.
    VertexBufferTooSmall {
        needed: usize,
        found: usize,
    },
    /// Vertices of no size describe no geometry, and would make any buffer
    /// look large enough for any count.
    EmptyVertexStride,
    /// More constants than Metal's inline payload holds.
    UniformsTooLarge {
        bytes: usize,
        limit: usize,
    },
    /// The depth comparison world geometry needs could not be created.
    NoDepthState,
    /// A pass mixes pipelines that write depth with ones that do not, which
    /// Metal cannot satisfy because the attachment is fixed for the pass.
    /// An index run whose start and length do not add up.
    IndexRunOverflow,
    MixedDepthPipelines {
        with_depth: usize,
        draws: usize,
    },
    /// The draw would read past the end of its index buffer.
    IndexBufferTooSmall {
        needed: usize,
        found: usize,
    },
    /// An index selects a vertex the draw does not have, which faults the GPU
    /// rather than drawing the wrong thing.
    IndexOutOfRange {
        index: usize,
        vertices: usize,
    },
    /// The layer the surface presents through could not be created.
    NoSwapchain,
    /// No drawable became free, which a real frame loop retries rather than
    /// treating as fatal.
    NoDrawable,
    /// A backing scale factor that cannot describe a display was given.
    InvalidScale(f64),
    /// A null view was handed over to present into.
    NoView,
}

impl fmt::Display for DeviceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                write!(formatter, "no renderer backend for this platform")
            }
            Self::NoDevice => write!(formatter, "no Metal device is available"),
            Self::NoName => write!(formatter, "the Metal device reported no name"),
            Self::NoQueue => write!(formatter, "the Metal device created no command queue"),
            Self::EmptyTarget => write!(formatter, "a render target needs a nonzero size"),
            Self::NoTexture => write!(formatter, "the Metal device created no texture"),
            Self::NoCommandBuffer => write!(formatter, "the Metal queue created no command buffer"),
            Self::NoEncoder => write!(formatter, "the command buffer created no render encoder"),
            Self::NoRenderPass => write!(formatter, "the render pass could not be described"),
            Self::NotTriangles(count) => {
                write!(formatter, "{count} vertices are not whole triangles")
            }
            Self::ShaderSourceNotRepresentable => {
                write!(formatter, "shader text cannot contain a NUL byte")
            }
            Self::ShaderCompilationFailed(message) => {
                write!(formatter, "the shader did not compile: {message}")
            }
            Self::ShaderFunctionMissing(name) => {
                write!(formatter, "the library holds no function named {name}")
            }
            Self::NoPipelineDescriptor => {
                write!(formatter, "the pipeline could not be described")
            }
            Self::PipelineCreationFailed(message) => {
                write!(formatter, "the pipeline was rejected: {message}")
            }
            Self::EmptyResource => write!(formatter, "a GPU resource needs contents"),
            Self::NoBuffer => write!(formatter, "the Metal device created no buffer"),
            Self::TextureSizeMismatch { expected, found } => write!(
                formatter,
                "the texture needs {expected} bytes of pixels but got {found}"
            ),
            Self::UnalignedTexture {
                width,
                height,
                block,
            } => write!(
                formatter,
                "a {width}x{height} texture is not whole {block}x{block} blocks"
            ),
            Self::VertexBufferTooSmall { needed, found } => write!(
                formatter,
                "the draw reads {needed} bytes from a {found} byte vertex buffer"
            ),
            Self::EmptyVertexStride => {
                write!(formatter, "a vertex stride of zero describes no vertices")
            }
            Self::UniformsTooLarge { bytes, limit } => write!(
                formatter,
                "{bytes} bytes of constants exceed the {limit} byte inline limit"
            ),
            Self::NoDepthState => write!(formatter, "no depth comparison state"),
            Self::IndexRunOverflow => {
                write!(formatter, "index run start and length overflow")
            }
            Self::MixedDepthPipelines { with_depth, draws } => write!(
                formatter,
                "{with_depth} of {draws} draws in the pass write depth, which has to be all or none"
            ),
            Self::LevelSizeMismatch {
                level,
                expected,
                found,
            } => write!(
                formatter,
                "mip level {level} needs {expected} bytes but was given {found}"
            ),
            Self::TooManyLevels { found, available } => write!(
                formatter,
                "{found} mip levels were given for an image with {available}"
            ),
            Self::IndexBufferTooSmall { needed, found } => write!(
                formatter,
                "the draw reads {needed} bytes from a {found} byte index buffer"
            ),
            Self::IndexOutOfRange { index, vertices } => write!(
                formatter,
                "index {index} selects a vertex outside the {vertices} the draw has"
            ),
            Self::NoSwapchain => write!(formatter, "no Metal layer was created to present through"),
            Self::NoDrawable => write!(formatter, "no drawable became available to render into"),
            Self::InvalidScale(scale) => {
                write!(formatter, "{scale} is not a usable backing scale factor")
            }
            Self::NoView => write!(formatter, "there is no view to present into"),
        }
    }
}

impl std::error::Error for DeviceError {}

/// The device on platforms without a backend yet. Windows and Linux are part
/// of the later platform expansion, and refusing here is honest about that
/// rather than pretending a device exists.
#[cfg(not(target_os = "macos"))]
pub struct Device(std::convert::Infallible);

/// A shader library cannot exist without a device, so these mirror the macOS
/// types only to keep the API one shape across platforms.
#[cfg(not(target_os = "macos"))]
pub struct Library(std::convert::Infallible);

#[cfg(not(target_os = "macos"))]
pub struct Pipeline(std::convert::Infallible);

#[cfg(not(target_os = "macos"))]
pub struct Buffer(std::convert::Infallible);

#[cfg(not(target_os = "macos"))]
pub struct Swapchain(std::convert::Infallible);

#[cfg(not(target_os = "macos"))]
impl Swapchain {
    pub fn size(&self) -> (u32, u32) {
        match self.0 {}
    }

    pub fn contents_scale(&self) -> f64 {
        match self.0 {}
    }

    pub fn resize(&mut self, _width: u32, _height: u32) -> Result<()> {
        match self.0 {}
    }

    pub fn set_contents_scale(&mut self, _scale: f64) -> Result<()> {
        match self.0 {}
    }

    /// # Safety
    /// Unreachable: no swapchain can exist on a platform without a backend.
    pub unsafe fn attach_to_view(&self, _view: *mut std::ffi::c_void) -> Result<()> {
        match self.0 {}
    }
}

#[cfg(not(target_os = "macos"))]
pub struct Texture(std::convert::Infallible);

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFormat {
    Uint16,
    Uint32,
}

#[cfg(not(target_os = "macos"))]
impl IndexFormat {
    pub fn size(self) -> usize {
        match self {
            Self::Uint16 => 2,
            Self::Uint32 => 4,
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub struct Indices<'a> {
    pub buffer: &'a Buffer,
    pub count: usize,
    pub format: IndexFormat,
    pub first: usize,
}

#[cfg(not(target_os = "macos"))]
pub enum Vertices<'a> {
    Inline(&'a [[f32; 2]]),
    Buffered {
        buffer: &'a Buffer,
        count: usize,
        stride: usize,
    },
}

#[cfg(not(target_os = "macos"))]
pub struct TriangleList<'a> {
    pub pipeline: &'a Pipeline,
    pub vertices: Vertices<'a>,
    pub texture: Option<&'a Texture>,
    pub coordinates: Option<VertexAttribute<'a>>,
    pub lightmap: Option<&'a Texture>,
    pub lightmap_coordinates: Option<VertexAttribute<'a>>,
    /// A colour per vertex the fragment stage modulates by. See
    /// [`TriangleList::with_shade`].
    pub shade: Option<VertexAttribute<'a>>,
    pub indices: Option<Indices<'a>>,
    pub uniforms: Option<&'a [u8]>,
}

#[cfg(not(target_os = "macos"))]
pub const UNIFORM_BUFFER_INDEX: u64 = 1;

#[cfg(not(target_os = "macos"))]
pub const COORDINATE_BUFFER_INDEX: u64 = 2;

#[cfg(not(target_os = "macos"))]
pub const LIGHTMAP_COORDINATE_BUFFER_INDEX: u64 = 3;

#[cfg(not(target_os = "macos"))]
pub const SHADE_BUFFER_INDEX: u64 = 4;

#[cfg(not(target_os = "macos"))]
pub const TEXTURE_INDEX: u64 = 0;

#[cfg(not(target_os = "macos"))]
pub const LIGHTMAP_TEXTURE_INDEX: u64 = 1;

#[cfg(not(target_os = "macos"))]
pub const MAX_INLINE_UNIFORM_BYTES: usize = 4096;

#[cfg(not(target_os = "macos"))]
impl Device {
    pub fn open() -> Result<Self> {
        Err(DeviceError::UnsupportedPlatform)
    }

    pub fn name(&self) -> &str {
        match self.0 {}
    }

    pub fn compile_library(&self, _source: &str) -> Result<Library> {
        match self.0 {}
    }

    pub fn create_buffer(&self, _bytes: &[u8]) -> Result<Buffer> {
        match self.0 {}
    }

    pub fn create_texture(
        &self,
        _width: u32,
        _height: u32,
        _format: TextureFormat,
        _bytes: &[u8],
    ) -> Result<Texture> {
        match self.0 {}
    }

    pub fn create_mipped_texture(
        &self,
        _width: u32,
        _height: u32,
        _format: TextureFormat,
        _levels: &[&[u8]],
    ) -> Result<Texture> {
        match self.0 {}
    }

    pub fn create_pipeline(
        &self,
        _library: &Library,
        _vertex: &str,
        _fragment: &str,
    ) -> Result<Pipeline> {
        match self.0 {}
    }

    pub fn create_depth_pipeline(
        &self,
        _library: &Library,
        _vertex: &str,
        _fragment: &str,
    ) -> Result<Pipeline> {
        match self.0 {}
    }

    pub fn create_blended_pipeline(
        &self,
        _library: &Library,
        _vertex: &str,
        _fragment: &str,
    ) -> Result<Pipeline> {
        match self.0 {}
    }

    pub fn create_swapchain(&self, _width: u32, _height: u32, _scale: f64) -> Result<Swapchain> {
        match self.0 {}
    }

    pub fn present(
        &self,
        _swapchain: &Swapchain,
        _color: ClearColor,
        _draws: &[TriangleList<'_>],
    ) -> Result<()> {
        match self.0 {}
    }

    pub fn present_capturing(
        &self,
        _swapchain: &Swapchain,
        _color: ClearColor,
        _draws: &[TriangleList<'_>],
    ) -> Result<Readback> {
        match self.0 {}
    }

    pub fn clear_offscreen(
        &self,
        _width: u32,
        _height: u32,
        _color: ClearColor,
    ) -> Result<Readback> {
        match self.0 {}
    }

    pub fn render_offscreen(
        &self,
        _width: u32,
        _height: u32,
        _color: ClearColor,
        _draws: &[TriangleList<'_>],
    ) -> Result<Readback> {
        match self.0 {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the device, requiring one on Apple silicon.
    ///
    /// Every Apple-silicon Mac has a Metal GPU, so a missing device there is a
    /// defect in this crate rather than a machine without hardware. Older
    /// Intel Macs and virtualized runners can genuinely lack one, and the test
    /// reports that instead of asserting it away.
    #[cfg(target_os = "macos")]
    fn require_device() -> Option<Device> {
        match Device::open() {
            Ok(device) => Some(device),
            Err(error) if cfg!(target_arch = "aarch64") => {
                panic!("Apple silicon must have a Metal device: {error}")
            }
            Err(error) => {
                assert_eq!(error, DeviceError::NoDevice);
                None
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn opens_the_system_metal_device_and_a_queue() {
        let Some(device) = require_device() else {
            return;
        };
        assert!(!device.name().is_empty());
        // Opening twice has to work, because the engine reopens the device
        // across a mode change.
        let second = Device::open().expect("the device opened once already");
        assert_eq!(second.name(), device.name());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn clears_an_offscreen_target_to_exact_pixels() {
        let Some(device) = require_device() else {
            return;
        };

        // Fully saturated channels so the result is exact rather than subject
        // to rounding, which is what a capture gate needs.
        let color = ClearColor {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };
        let readback = device.clear_offscreen(4, 3, color).expect("cleared");

        assert_eq!(readback.width, 4);
        assert_eq!(readback.height, 3);
        assert_eq!(readback.pixels.len(), 4 * 3 * 4);
        // BGRA order, so pure red reads back as blue 0, green 0, red 255.
        for y in 0..3 {
            for x in 0..4 {
                assert_eq!(readback.pixel(x, y), Some([0, 0, 255, 255]), "at {x},{y}");
            }
        }
        assert_eq!(readback.pixel(4, 0), None);
        assert_eq!(readback.pixel(0, 3), None);

        let black = device
            .clear_offscreen(
                1,
                1,
                ClearColor {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 0.0,
                },
            )
            .expect("cleared");
        assert_eq!(black.pixel(0, 0), Some([0, 0, 0, 0]));

        assert_eq!(
            device.clear_offscreen(0, 1, color).err(),
            Some(DeviceError::EmptyTarget)
        );
    }

    /// A shader that covers whatever positions it is handed, in a fixed color.
    #[cfg(target_os = "macos")]
    const FLAT_SHADER: &str = "
        #include <metal_stdlib>
        using namespace metal;
        vertex float4 flat_vertex(const device float2 *positions [[buffer(0)]],
                                  uint index [[vertex_id]]) {
            return float4(positions[index], 0.0, 1.0);
        }
        fragment float4 flat_fragment() {
            return float4(0.0, 1.0, 0.0, 1.0);
        }
    ";

    #[test]
    #[cfg(target_os = "macos")]
    fn draws_a_triangle_over_the_clear() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(FLAT_SHADER).expect("compiled");
        let pipeline = device
            .create_pipeline(&library, "flat_vertex", "flat_fragment")
            .expect("built");

        // Clip space runs -1 to 1, so a triangle reaching 3 covers every pixel
        // of the target. Full coverage keeps the result independent of how
        // edges land on pixel centers, which is what makes this a usable gate.
        let covering = [[-1.0f32, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let red = ClearColor {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };
        let drawn = device
            .render_offscreen(
                8,
                8,
                red,
                &[TriangleList::new(&pipeline, Vertices::Inline(&covering))],
            )
            .expect("rendered");

        // Green everywhere rather than the red clear, so the draw ran and the
        // vertex positions reached the shader.
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(drawn.pixel(x, y), Some([0, 255, 0, 255]), "at {x},{y}");
            }
        }

        // The same pass with a triangle in one corner has to leave the clear
        // showing elsewhere, which separates a real rasterization from a
        // pipeline that happens to paint the whole target.
        let corner = [[-1.0f32, -1.0], [-0.5, -1.0], [-1.0, -0.5]];
        let partial = device
            .render_offscreen(
                8,
                8,
                red,
                &[TriangleList::new(&pipeline, Vertices::Inline(&corner))],
            )
            .expect("rendered");
        assert_eq!(partial.pixel(7, 7), Some([0, 0, 255, 255]));
        assert!(
            partial.pixels.chunks(4).any(|pixel| pixel[1] == 255),
            "the corner triangle covered nothing"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn reports_what_the_shader_compiler_rejected() {
        let Some(device) = require_device() else {
            return;
        };

        let Err(DeviceError::ShaderCompilationFailed(message)) =
            device.compile_library("this is not a shader")
        else {
            panic!("broken source has to be refused");
        };
        assert!(!message.is_empty());

        let library = device.compile_library(FLAT_SHADER).expect("compiled");
        assert_eq!(
            device
                .create_pipeline(&library, "absent_vertex", "flat_fragment")
                .err(),
            Some(DeviceError::ShaderFunctionMissing(
                "absent_vertex".to_owned()
            ))
        );
        assert_eq!(
            device.compile_library("shader\0with a nul").err(),
            Some(DeviceError::ShaderSourceNotRepresentable)
        );

        let pipeline = device
            .create_pipeline(&library, "flat_vertex", "flat_fragment")
            .expect("built");
        // Two vertices cannot make a triangle, and silently dropping them
        // would hide a caller's mistake in an empty frame.
        assert_eq!(
            device
                .render_offscreen(
                    2,
                    2,
                    ClearColor {
                        red: 0.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha: 1.0,
                    },
                    &[TriangleList::new(
                        &pipeline,
                        Vertices::Inline(&[[0.0, 0.0], [1.0, 0.0]])
                    )],
                )
                .err(),
            Some(DeviceError::NotTriangles(2))
        );
    }

    /// Samples a texture across a covering triangle, flipping vertically so
    /// the texture's first row lands at the top of the captured image.
    #[cfg(target_os = "macos")]
    const TEXTURED_SHADER: &str = "
        #include <metal_stdlib>
        using namespace metal;
        struct Varying {
            float4 position [[position]];
            float2 uv;
        };
        vertex Varying textured_vertex(const device float2 *positions [[buffer(0)]],
                                       uint index [[vertex_id]]) {
            Varying out;
            float2 clip = positions[index];
            out.position = float4(clip, 0.0, 1.0);
            out.uv = float2(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
            return out;
        }
        fragment float4 textured_fragment(Varying in [[stage_in]],
                                          texture2d<float> image [[texture(0)]]) {
            constexpr sampler nearest(filter::nearest, address::clamp_to_edge);
            return image.sample(nearest, in.uv);
        }
    ";

    #[test]
    #[cfg(target_os = "macos")]
    fn samples_an_uploaded_texture_through_a_vertex_buffer() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(TEXTURED_SHADER).expect("compiled");
        let pipeline = device
            .create_pipeline(&library, "textured_vertex", "textured_fragment")
            .expect("built");

        // Four distinct BGRA texels, so each quadrant of the result identifies
        // exactly which one it sampled and a flipped or rotated upload cannot
        // pass.
        const RED: [u8; 4] = [0, 0, 255, 255];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        const BLUE: [u8; 4] = [255, 0, 0, 255];
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        let mut pixels = Vec::new();
        for texel in [RED, GREEN, BLUE, WHITE] {
            pixels.extend_from_slice(&texel);
        }
        let texture = device
            .create_texture(2, 2, TextureFormat::Bgra8Unorm, &pixels)
            .expect("uploaded");
        assert_eq!((texture.width(), texture.height()), (2, 2));
        assert_eq!(texture.format(), TextureFormat::Bgra8Unorm);

        // The same covering triangle, but staged through a buffer rather than
        // inline, so this exercises the path real geometry takes.
        let covering: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let vertex_bytes = unsafe {
            std::slice::from_raw_parts(
                covering.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&covering),
            )
        };
        let vertices = device.create_buffer(vertex_bytes).expect("uploaded");
        assert_eq!(vertices.len(), 24);
        assert!(!vertices.is_empty());

        let captured = device
            .render_offscreen(
                4,
                4,
                ClearColor {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                },
                &[TriangleList::new(
                    &pipeline,
                    Vertices::Buffered {
                        buffer: &vertices,
                        count: 3,
                        stride: std::mem::size_of::<[f32; 2]>(),
                    },
                )
                .with_texture(&texture)],
            )
            .expect("rendered");

        assert_eq!(captured.pixel(0, 0), Some(RED));
        assert_eq!(captured.pixel(3, 0), Some(GREEN));
        assert_eq!(captured.pixel(0, 3), Some(BLUE));
        assert_eq!(captured.pixel(3, 3), Some(WHITE));
    }

    /// Samples a chosen level explicitly, which is what makes a mip upload
    /// observable: the default sampler would pick a level by footprint and
    /// could pass while the smaller levels held nothing.
    #[cfg(target_os = "macos")]
    const MIPPED_SHADER: &str = "
        #include <metal_stdlib>
        using namespace metal;
        struct Varying {
            float4 position [[position]];
            float2 uv;
        };
        vertex Varying mipped_vertex(const device float2 *positions [[buffer(0)]],
                                     uint index [[vertex_id]]) {
            Varying out;
            float2 clip = positions[index];
            out.position = float4(clip, 0.0, 1.0);
            out.uv = float2(clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5);
            return out;
        }
        fragment float4 mipped_base(Varying in [[stage_in]],
                                    texture2d<float> image [[texture(0)]]) {
            constexpr sampler nearest(filter::nearest, address::clamp_to_edge, mip_filter::nearest);
            return image.sample(nearest, in.uv, level(0));
        }
        fragment float4 mipped_small(Varying in [[stage_in]],
                                     texture2d<float> image [[texture(0)]]) {
            constexpr sampler nearest(filter::nearest, address::clamp_to_edge, mip_filter::nearest);
            return image.sample(nearest, in.uv, level(1));
        }
    ";

    #[test]
    #[cfg(target_os = "macos")]
    fn uploads_and_samples_each_level_of_a_mip_chain() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(MIPPED_SHADER).expect("compiled");
        let base_pipeline = device
            .create_pipeline(&library, "mipped_vertex", "mipped_base")
            .expect("built");
        let small_pipeline = device
            .create_pipeline(&library, "mipped_vertex", "mipped_small")
            .expect("built");

        const RED: [u8; 4] = [0, 0, 255, 255];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        // A 2x2 base of one colour over a 1x1 level of another, so whichever
        // level the shader reads names itself in the result.
        let base: Vec<u8> = RED.repeat(4);
        let small: Vec<u8> = GREEN.to_vec();
        let texture = device
            .create_mipped_texture(2, 2, TextureFormat::Bgra8Unorm, &[&base, &small])
            .expect("uploaded");
        assert_eq!(texture.levels(), 2);

        let covering: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let vertex_bytes = unsafe {
            std::slice::from_raw_parts(
                covering.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&covering),
            )
        };
        let vertices = device.create_buffer(vertex_bytes).expect("uploaded");
        let black = ClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };

        let sample_with = |pipeline: &Pipeline| {
            device
                .render_offscreen(
                    2,
                    2,
                    black,
                    &[TriangleList::new(
                        pipeline,
                        Vertices::Buffered {
                            buffer: &vertices,
                            count: 3,
                            stride: std::mem::size_of::<[f32; 2]>(),
                        },
                    )
                    .with_texture(&texture)],
                )
                .expect("rendered")
        };

        assert_eq!(sample_with(&base_pipeline).pixel(0, 0), Some(RED));
        assert_eq!(sample_with(&small_pipeline).pixel(0, 0), Some(GREEN));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn refuses_mip_chains_that_do_not_match_their_own_sizes() {
        let Some(device) = require_device() else {
            return;
        };

        const RED: [u8; 4] = [0, 0, 255, 255];
        let base: Vec<u8> = RED.repeat(4);

        // Level one of a 2x2 image is a single pixel, not four.
        assert_eq!(
            device
                .create_mipped_texture(2, 2, TextureFormat::Bgra8Unorm, &[&base, &base])
                .err(),
            Some(DeviceError::LevelSizeMismatch {
                level: 1,
                expected: 4,
                found: 16
            })
        );

        // A 2x2 image halves to one pixel in two levels, so a third has no
        // size to describe.
        let small: Vec<u8> = RED.to_vec();
        assert_eq!(
            device
                .create_mipped_texture(2, 2, TextureFormat::Bgra8Unorm, &[&base, &small, &small])
                .err(),
            Some(DeviceError::TooManyLevels {
                found: 3,
                available: 2
            })
        );

        assert_eq!(
            device
                .create_mipped_texture(2, 2, TextureFormat::Bgra8Unorm, &[])
                .err(),
            Some(DeviceError::EmptyResource)
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn draws_a_quad_from_shared_indexed_vertices() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(TEXTURED_SHADER).expect("compiled");
        let pipeline = device
            .create_pipeline(&library, "textured_vertex", "textured_fragment")
            .expect("built");

        const RED: [u8; 4] = [0, 0, 255, 255];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        const BLUE: [u8; 4] = [255, 0, 0, 255];
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        let mut pixels = Vec::new();
        for texel in [RED, GREEN, BLUE, WHITE] {
            pixels.extend_from_slice(&texel);
        }
        let texture = device
            .create_texture(2, 2, TextureFormat::Bgra8Unorm, &pixels)
            .expect("uploaded");

        // Four corners rather than six, with the diagonal pair shared between
        // the two triangles: this is the saving indexed drawing exists for and
        // the reason BSP surfaces are stored this way.
        let corners: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
        let vertex_bytes = unsafe {
            std::slice::from_raw_parts(
                corners.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&corners),
            )
        };
        let vertices = device.create_buffer(vertex_bytes).expect("uploaded");

        let order: [u16; 6] = [0, 1, 2, 2, 1, 3];
        let index_bytes = unsafe {
            std::slice::from_raw_parts(order.as_ptr().cast::<u8>(), std::mem::size_of_val(&order))
        };
        let indices = device.create_buffer(index_bytes).expect("uploaded");
        assert_eq!(indices.len(), 12);

        let captured = device
            .render_offscreen(
                4,
                4,
                ClearColor {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                },
                &[TriangleList::new(
                    &pipeline,
                    Vertices::Buffered {
                        buffer: &vertices,
                        count: 4,
                        stride: std::mem::size_of::<[f32; 2]>(),
                    },
                )
                .with_texture(&texture)
                .with_indices(Indices {
                    buffer: &indices,
                    count: 6,
                    format: IndexFormat::Uint16,
                    first: 0,
                })],
            )
            .expect("rendered");

        // The same image the covering triangle produces, which is what makes
        // this a check on the index path rather than on the shader.
        assert_eq!(captured.pixel(0, 0), Some(RED));
        assert_eq!(captured.pixel(3, 0), Some(GREEN));
        assert_eq!(captured.pixel(0, 3), Some(BLUE));
        assert_eq!(captured.pixel(3, 3), Some(WHITE));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn refuses_indices_that_do_not_describe_a_drawable_batch() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(TEXTURED_SHADER).expect("compiled");
        let pipeline = device
            .create_pipeline(&library, "textured_vertex", "textured_fragment")
            .expect("built");
        let corners: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]];
        let vertex_bytes = unsafe {
            std::slice::from_raw_parts(
                corners.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&corners),
            )
        };
        let vertices = device.create_buffer(vertex_bytes).expect("uploaded");
        let black = ClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };

        let draw_with = |indices: &Buffer, count: usize| {
            device
                .render_offscreen(
                    4,
                    4,
                    black,
                    &[TriangleList::new(
                        &pipeline,
                        Vertices::Buffered {
                            buffer: &vertices,
                            count: 4,
                            stride: std::mem::size_of::<[f32; 2]>(),
                        },
                    )
                    .with_indices(Indices {
                        buffer: indices,
                        count,
                        format: IndexFormat::Uint16,
                        first: 0,
                    })],
                )
                .err()
        };

        // Three indices of the six the draw claims.
        let short: [u16; 3] = [0, 1, 2];
        let short_bytes = unsafe {
            std::slice::from_raw_parts(short.as_ptr().cast::<u8>(), std::mem::size_of_val(&short))
        };
        let short = device.create_buffer(short_bytes).expect("uploaded");
        assert_eq!(
            draw_with(&short, 6),
            Some(DeviceError::IndexBufferTooSmall {
                needed: 12,
                found: 6
            })
        );

        // A fifth vertex the draw does not have, which would fault the GPU.
        let past_end: [u16; 6] = [0, 1, 2, 2, 1, 4];
        let past_end_bytes = unsafe {
            std::slice::from_raw_parts(
                past_end.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&past_end),
            )
        };
        let past_end = device.create_buffer(past_end_bytes).expect("uploaded");
        assert_eq!(
            draw_with(&past_end, 6),
            Some(DeviceError::IndexOutOfRange {
                index: 4,
                vertices: 4
            })
        );

        // Indices that do not group into whole triangles.
        let partial: [u16; 4] = [0, 1, 2, 3];
        let partial_bytes = unsafe {
            std::slice::from_raw_parts(
                partial.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&partial),
            )
        };
        let partial = device.create_buffer(partial_bytes).expect("uploaded");
        assert_eq!(draw_with(&partial, 4), Some(DeviceError::NotTriangles(4)));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn samples_a_block_compressed_texture() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(TEXTURED_SHADER).expect("compiled");
        let pipeline = device
            .create_pipeline(&library, "textured_vertex", "textured_fragment")
            .expect("built");

        // One BC1 block, which stores two endpoint colors as RGB565 followed
        // by four bytes of two-bit indices. Setting both endpoints to the same
        // color makes every index select it, so the block decodes to a solid
        // color whatever the indices say. 0xF800 is red at full intensity, and
        // five bits expand back to exactly 255, so the expected value is exact
        // rather than approximate.
        let mut block = Vec::new();
        block.extend_from_slice(&0xF800u16.to_le_bytes());
        block.extend_from_slice(&0xF800u16.to_le_bytes());
        block.extend_from_slice(&[0u8; 4]);
        let texture = device
            .create_texture(4, 4, TextureFormat::Bc1Rgba, &block)
            .expect("uploaded");
        assert_eq!(texture.format(), TextureFormat::Bc1Rgba);

        let covering: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let captured =
            device
                .render_offscreen(
                    4,
                    4,
                    ClearColor {
                        red: 0.0,
                        green: 0.0,
                        blue: 1.0,
                        alpha: 1.0,
                    },
                    &[TriangleList::new(&pipeline, Vertices::Inline(&covering))
                        .with_texture(&texture)],
                )
                .expect("rendered");

        // Red in BGRA, and distinct from the blue clear, so this cannot pass
        // on a draw that silently did nothing.
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(captured.pixel(x, y), Some([0, 0, 255, 255]), "at {x},{y}");
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn presents_a_drawable_and_captures_what_was_presented() {
        let Some(device) = require_device() else {
            return;
        };

        // Two points per pixel, which is what a Retina display reports, so
        // the drawable has to be twice the size in points.
        let mut swapchain = device.create_swapchain(4, 4, 2.0).expect("created");
        assert_eq!(swapchain.size(), (8, 8));
        assert_eq!(swapchain.contents_scale(), 2.0);

        let library = device.compile_library(FLAT_SHADER).expect("compiled");
        let pipeline = device
            .create_pipeline(&library, "flat_vertex", "flat_fragment")
            .expect("built");
        let covering: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
        let red = ClearColor {
            red: 1.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };

        // The capture comes out of the drawable that was presented, not a
        // separate render, so green here means the presented frame carried
        // the draw rather than the clear.
        let captured = device
            .present_capturing(
                &swapchain,
                red,
                &[TriangleList::new(&pipeline, Vertices::Inline(&covering))],
            )
            .expect("presented");
        assert_eq!((captured.width, captured.height), (8, 8));
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(captured.pixel(x, y), Some([0, 255, 0, 255]), "at {x},{y}");
            }
        }

        // A window moved to a different display changes size and scale
        // independently, and both have to reach the drawables.
        swapchain.resize(6, 3).expect("resized");
        assert_eq!(swapchain.size(), (12, 6));
        swapchain.set_contents_scale(1.0).expect("rescaled");
        assert_eq!(swapchain.size(), (6, 3));

        let after_resize = device
            .present_capturing(&swapchain, red, &[])
            .expect("presented");
        assert_eq!((after_resize.width, after_resize.height), (6, 3));
        assert_eq!(after_resize.pixel(5, 2), Some([0, 0, 255, 255]));

        // Presenting without capturing has to drive the same path.
        device.present(&swapchain, red, &[]).expect("presented");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn refuses_surfaces_that_cannot_describe_a_display() {
        let Some(device) = require_device() else {
            return;
        };

        assert_eq!(
            device.create_swapchain(0, 4, 1.0).err(),
            Some(DeviceError::EmptyTarget)
        );
        // Matched rather than compared, because NaN is one of the rejected
        // values and it is not equal to itself.
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let rejected = device.create_swapchain(4, 4, scale).err();
            assert!(
                matches!(rejected, Some(DeviceError::InvalidScale(reported))
                    if reported.to_bits() == scale.to_bits()),
                "scale {scale} gave {rejected:?}"
            );
        }

        let mut swapchain = device.create_swapchain(4, 4, 1.0).expect("created");
        assert_eq!(swapchain.resize(0, 4).err(), Some(DeviceError::EmptyTarget));
        assert_eq!(
            swapchain.set_contents_scale(0.0).err(),
            Some(DeviceError::InvalidScale(0.0))
        );
        // A refused change leaves the surface as it was, rather than half
        // applied.
        assert_eq!(swapchain.size(), (4, 4));
        assert_eq!(swapchain.contents_scale(), 1.0);

        // SAFETY: a null view is exactly the case being checked, and the
        // implementation refuses before sending any message.
        assert_eq!(
            unsafe { swapchain.attach_to_view(std::ptr::null_mut()) }.err(),
            Some(DeviceError::NoView)
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn refuses_resources_that_do_not_describe_themselves() {
        let Some(device) = require_device() else {
            return;
        };

        assert_eq!(
            device.create_buffer(&[]).err(),
            Some(DeviceError::EmptyResource)
        );
        assert_eq!(
            device
                .create_texture(0, 4, TextureFormat::Bgra8Unorm, &[0; 16])
                .err(),
            Some(DeviceError::EmptyTarget)
        );
        // A short upload would otherwise leave the tail of the texture reading
        // whatever the allocation happened to hold.
        assert_eq!(
            device
                .create_texture(2, 2, TextureFormat::Bgra8Unorm, &[0; 8])
                .err(),
            Some(DeviceError::TextureSizeMismatch {
                expected: 16,
                found: 8
            })
        );
        // Half a block has no upload region Metal can describe.
        assert_eq!(
            device
                .create_texture(6, 4, TextureFormat::Bc1Rgba, &[0; 16])
                .err(),
            Some(DeviceError::UnalignedTexture {
                width: 6,
                height: 4,
                block: 4
            })
        );
        // Sized by blocks rather than pixels: 8x8 of BC1 is four blocks.
        assert_eq!(
            device
                .create_texture(8, 8, TextureFormat::Bc1Rgba, &[0; 16])
                .err(),
            Some(DeviceError::TextureSizeMismatch {
                expected: 32,
                found: 16
            })
        );

        let library = device.compile_library(FLAT_SHADER).expect("compiled");
        let pipeline = device
            .create_pipeline(&library, "flat_vertex", "flat_fragment")
            .expect("built");
        let short = device.create_buffer(&[0u8; 8]).expect("uploaded");
        assert_eq!(
            device
                .render_offscreen(
                    2,
                    2,
                    ClearColor {
                        red: 0.0,
                        green: 0.0,
                        blue: 0.0,
                        alpha: 1.0,
                    },
                    &[TriangleList::new(
                        &pipeline,
                        Vertices::Buffered {
                            buffer: &short,
                            count: 3,
                            stride: std::mem::size_of::<[f32; 2]>(),
                        }
                    )],
                )
                .err(),
            Some(DeviceError::VertexBufferTooSmall {
                needed: 24,
                found: 8
            })
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn refuses_platforms_without_a_backend() {
        assert_eq!(Device::open().err(), Some(DeviceError::UnsupportedPlatform));
    }

    #[test]
    fn describes_every_failure() {
        for error in [
            DeviceError::UnsupportedPlatform,
            DeviceError::NoDevice,
            DeviceError::NoName,
            DeviceError::NoQueue,
            DeviceError::EmptyTarget,
            DeviceError::NoTexture,
            DeviceError::NoCommandBuffer,
            DeviceError::NoEncoder,
            DeviceError::NoRenderPass,
            DeviceError::NotTriangles(2),
            DeviceError::ShaderSourceNotRepresentable,
            DeviceError::ShaderCompilationFailed("unterminated".to_owned()),
            DeviceError::ShaderFunctionMissing("absent".to_owned()),
            DeviceError::NoPipelineDescriptor,
            DeviceError::PipelineCreationFailed("no attachment".to_owned()),
            DeviceError::EmptyResource,
            DeviceError::NoBuffer,
            DeviceError::TextureSizeMismatch {
                expected: 16,
                found: 8,
            },
            DeviceError::VertexBufferTooSmall {
                needed: 24,
                found: 8,
            },
            DeviceError::IndexBufferTooSmall {
                needed: 12,
                found: 6,
            },
            DeviceError::IndexOutOfRange {
                index: 4,
                vertices: 4,
            },
            DeviceError::UnalignedTexture {
                width: 6,
                height: 4,
                block: 4,
            },
            DeviceError::NoSwapchain,
            DeviceError::NoDrawable,
            DeviceError::InvalidScale(0.0),
            DeviceError::NoView,
            DeviceError::EmptyVertexStride,
            DeviceError::UniformsTooLarge {
                bytes: 8192,
                limit: MAX_INLINE_UNIFORM_BYTES,
            },
            DeviceError::NoDepthState,
            DeviceError::MixedDepthPipelines {
                with_depth: 1,
                draws: 2,
            },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }

    /// World surfaces arrive as positions in Source's world, not clip space,
    /// and are placed on screen by the camera transform the engine supplies.
    #[cfg(target_os = "macos")]
    const WORLD_SHADER: &str = "
        #include <metal_stdlib>
        using namespace metal;
        vertex float4 world_vertex(const device packed_float3 *positions [[buffer(0)]],
                                   constant float4x4 &view_projection [[buffer(1)]],
                                   uint index [[vertex_id]]) {
            return view_projection * float4(positions[index], 1.0);
        }
        fragment float4 near_fragment() {
            return float4(0.0, 1.0, 0.0, 1.0);
        }
        fragment float4 far_fragment() {
            return float4(1.0, 0.0, 0.0, 1.0);
        }
    ";

    /// A square standing across the view at `distance`, reaching `half_extent`
    /// either side of the eye line, as the two triangles a BSP surface of the
    /// same shape triangulates into.
    #[cfg(target_os = "macos")]
    fn wall(distance: f32, half_extent: f32) -> [[f32; 3]; 6] {
        let (d, h) = (distance, half_extent);
        [
            [d, -h, -h],
            [d, h, -h],
            [d, h, h],
            [d, -h, -h],
            [d, h, h],
            [d, -h, h],
        ]
    }

    #[cfg(target_os = "macos")]
    fn upload_positions(device: &Device, positions: &[[f32; 3]]) -> Buffer {
        // SAFETY: `f32` has no padding and no invalid bit patterns, so the
        // positions are readable as the bytes the GPU copies.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                positions.as_ptr().cast::<u8>(),
                std::mem::size_of_val(positions),
            )
        };
        device.create_buffer(bytes).expect("uploaded")
    }

    #[cfg(target_os = "macos")]
    const SQUARE_90_DEGREE_LENS: Lens = Lens {
        horizontal_fov: 90.0,
        aspect: 1.0,
        near: 1.0,
        far: 4096.0,
    };

    #[cfg(target_os = "macos")]
    fn eye_at_origin() -> Eye {
        Eye {
            position: [0.0, 0.0, 0.0],
            angles: [0.0, 0.0, 0.0],
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn places_world_geometry_on_screen_through_the_camera() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(WORLD_SHADER).expect("compiled");
        let pipeline = device
            .create_depth_pipeline(&library, "world_vertex", "near_fragment")
            .expect("built");

        // At 64 units out under a 90 degree lens, the edge of the screen is
        // 64 units to the side, so a wall reaching 32 covers exactly the
        // middle half of it. That is what makes this a test of the transform
        // rather than of whether anything drew at all.
        let positions = wall(64.0, 32.0);
        let vertices = upload_positions(&device, &positions);
        let transform =
            view_projection(eye_at_origin(), SQUARE_90_DEGREE_LENS).expect("a square lens");
        let blue = ClearColor {
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            alpha: 1.0,
        };

        let drawn = device
            .render_offscreen(
                16,
                16,
                blue,
                &[TriangleList::new(
                    &pipeline,
                    Vertices::Buffered {
                        buffer: &vertices,
                        count: positions.len(),
                        stride: std::mem::size_of::<[f32; 3]>(),
                    },
                )
                .with_uniforms(transform.as_bytes())],
            )
            .expect("rendered");

        const GREEN: [u8; 4] = [0, 255, 0, 255];
        const BLUE: [u8; 4] = [255, 0, 0, 255];

        assert_eq!(
            drawn.pixel(8, 8),
            Some(GREEN),
            "the wall the camera faces is in the middle of the screen"
        );
        // The middle half of a sixteen pixel target is pixels four to eleven,
        // so just inside is covered and just outside is not.
        assert_eq!(drawn.pixel(5, 8), Some(GREEN), "inside the wall's edge");
        assert_eq!(drawn.pixel(2, 8), Some(BLUE), "outside the wall's edge");
        assert_eq!(drawn.pixel(8, 2), Some(BLUE), "above the wall's edge");
        for (x, y) in [(0, 0), (15, 0), (0, 15), (15, 15)] {
            assert_eq!(
                drawn.pixel(x, y),
                Some(BLUE),
                "the corner at {x},{y} is past a wall that covers half the view"
            );
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn draws_one_material_run_at_a_time_out_of_a_shared_index_buffer() {
        let Some(device) = require_device() else {
            return;
        };

        // A map's world is one index buffer grouped into a run per material.
        // Each run has to draw its own triangles: a draw that ignored the
        // start would redraw the first material for every batch, which looks
        // plausible on screen and is entirely wrong.
        const RUN_SHADER: &str = "
            #include <metal_stdlib>
            using namespace metal;
            vertex float4 run_vertex(const device packed_float3 *positions [[buffer(0)]],
                                     constant float4x4 &view_projection [[buffer(1)]],
                                     uint index [[vertex_id]]) {
                return view_projection * float4(positions[index], 1.0);
            }
            fragment float4 run_fragment() {
                return float4(0.0, 1.0, 0.0, 1.0);
            }
        ";
        let library = device.compile_library(RUN_SHADER).expect("compiled");
        let pipeline = device
            .create_depth_pipeline(&library, "run_vertex", "run_fragment")
            .expect("built");

        // Two walls: the first fills the view, the second is off to the side
        // where the camera cannot see it.
        let mut positions = wall(64.0, 64.0).to_vec();
        positions.extend_from_slice(&wall(-64.0, 64.0));
        let vertices = upload_positions(&device, &positions);
        let indices: Vec<u32> = (0..12).collect();
        // SAFETY: `u32` has no padding and no invalid bit patterns.
        let index_bytes = unsafe {
            std::slice::from_raw_parts(
                indices.as_ptr().cast::<u8>(),
                std::mem::size_of_val(indices.as_slice()),
            )
        };
        let index_buffer = device.create_buffer(index_bytes).expect("uploaded");
        let transform =
            view_projection(eye_at_origin(), SQUARE_90_DEGREE_LENS).expect("a square lens");
        let blue = ClearColor {
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            alpha: 1.0,
        };

        let run = |first: usize| {
            device
                .render_offscreen(
                    8,
                    8,
                    blue,
                    &[TriangleList::new(
                        &pipeline,
                        Vertices::Buffered {
                            buffer: &vertices,
                            count: positions.len(),
                            stride: std::mem::size_of::<[f32; 3]>(),
                        },
                    )
                    .with_indices(Indices {
                        buffer: &index_buffer,
                        count: 6,
                        format: IndexFormat::Uint32,
                        first,
                    })
                    .with_uniforms(transform.as_bytes())],
                )
                .expect("rendered")
        };

        const GREEN: [u8; 4] = [0, 255, 0, 255];
        const BLUE: [u8; 4] = [255, 0, 0, 255];
        assert_eq!(
            run(0).pixel(4, 4),
            Some(GREEN),
            "the first run draws the wall in front of the camera"
        );
        assert_eq!(
            run(6).pixel(4, 4),
            Some(BLUE),
            "the second run draws the wall behind it, leaving the view clear"
        );

        // A run reaching past the buffer would have the GPU read indices
        // that are not there.
        let past = device.render_offscreen(
            8,
            8,
            blue,
            &[TriangleList::new(
                &pipeline,
                Vertices::Buffered {
                    buffer: &vertices,
                    count: positions.len(),
                    stride: std::mem::size_of::<[f32; 3]>(),
                },
            )
            .with_indices(Indices {
                buffer: &index_buffer,
                count: 6,
                format: IndexFormat::Uint32,
                first: 9,
            })
            .with_uniforms(transform.as_bytes())],
        );
        assert!(matches!(past, Err(DeviceError::IndexBufferTooSmall { .. })));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn samples_a_texture_at_the_coordinates_each_vertex_carries() {
        let Some(device) = require_device() else {
            return;
        };

        // Positions and texture coordinates arrive in separate buffers
        // because that is how a map stores them. If the second buffer is not
        // bound where the shader expects it, every vertex samples the same
        // corner of the texture and the surface comes out flat.
        const SAMPLING_SHADER: &str = "
            #include <metal_stdlib>
            using namespace metal;
            struct Surface {
                float4 position [[position]];
                float2 texcoord;
            };
            vertex Surface sampled_vertex(const device packed_float3 *positions [[buffer(0)]],
                                          const device packed_float2 *texcoords [[buffer(2)]],
                                          constant float4x4 &view_projection [[buffer(1)]],
                                          uint index [[vertex_id]]) {
                Surface out;
                out.position = view_projection * float4(positions[index], 1.0);
                out.texcoord = texcoords[index];
                return out;
            }
            fragment float4 sampled_fragment(Surface in [[stage_in]],
                                             texture2d<float> base [[texture(0)]]) {
                constexpr sampler point(address::clamp_to_edge, filter::nearest);
                return base.sample(point, in.texcoord);
            }
        ";
        let library = device.compile_library(SAMPLING_SHADER).expect("compiled");
        let pipeline = device
            .create_depth_pipeline(&library, "sampled_vertex", "sampled_fragment")
            .expect("built");

        // A two by two texture whose left half is red and right half is
        // blue, in the blue-first order the readback also uses.
        let red = [0u8, 0, 255, 255];
        let blue_pixel = [255u8, 0, 0, 255];
        let mut pixels = Vec::new();
        for _ in 0..2 {
            pixels.extend_from_slice(&red);
            pixels.extend_from_slice(&blue_pixel);
        }
        let texture = device
            .create_texture(2, 2, TextureFormat::Bgra8Unorm, &pixels)
            .expect("uploaded");

        let positions = wall(64.0, 64.0);
        let vertices = upload_positions(&device, &positions);
        // The wall's corners in the same order `wall` gives them, so the
        // texture's left half lands on the screen's left half.
        let texcoords: [[f32; 2]; 6] = [
            [0.0, 1.0],
            [1.0, 1.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [1.0, 0.0],
            [0.0, 0.0],
        ];
        // SAFETY: `f32` has no padding and no invalid bit patterns.
        let coordinate_bytes = unsafe {
            std::slice::from_raw_parts(
                texcoords.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&texcoords),
            )
        };
        let coordinates = device.create_buffer(coordinate_bytes).expect("uploaded");
        let transform =
            view_projection(eye_at_origin(), SQUARE_90_DEGREE_LENS).expect("a square lens");

        let drawn = device
            .render_offscreen(
                8,
                8,
                ClearColor {
                    red: 0.0,
                    green: 1.0,
                    blue: 0.0,
                    alpha: 1.0,
                },
                &[TriangleList::new(
                    &pipeline,
                    Vertices::Buffered {
                        buffer: &vertices,
                        count: positions.len(),
                        stride: std::mem::size_of::<[f32; 3]>(),
                    },
                )
                .with_texture(&texture)
                .with_coordinates(VertexAttribute::packed::<[f32; 2]>(&coordinates))
                .with_uniforms(transform.as_bytes())],
            )
            .expect("rendered");

        // Source's world has Y running left from the eye, so the texture's
        // first column lands on the right of the screen.
        assert_eq!(
            drawn.pixel(6, 4),
            Some(red),
            "the texture's left half is sampled on one side"
        );
        assert_eq!(
            drawn.pixel(1, 4),
            Some(blue_pixel),
            "and its right half on the other, so the coordinates varied"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn lets_the_nearer_surface_win_whichever_order_it_is_drawn_in() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(WORLD_SHADER).expect("compiled");
        let near_pipeline = device
            .create_depth_pipeline(&library, "world_vertex", "near_fragment")
            .expect("built");
        let far_pipeline = device
            .create_depth_pipeline(&library, "world_vertex", "far_fragment")
            .expect("built");

        // Both walls more than cover the view, so every pixel has a near and
        // a far candidate and the only thing deciding the result is depth.
        let near = wall(64.0, 128.0);
        let far = wall(256.0, 512.0);
        let near_vertices = upload_positions(&device, &near);
        let far_vertices = upload_positions(&device, &far);
        let transform =
            view_projection(eye_at_origin(), SQUARE_90_DEGREE_LENS).expect("a square lens");
        let black = ClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };

        let draw = |pipeline, buffer, count| {
            TriangleList::new(
                pipeline,
                Vertices::Buffered {
                    buffer,
                    count,
                    stride: std::mem::size_of::<[f32; 3]>(),
                },
            )
            .with_uniforms(transform.as_bytes())
        };

        for (order, draws) in [
            (
                "near first",
                [
                    draw(&near_pipeline, &near_vertices, near.len()),
                    draw(&far_pipeline, &far_vertices, far.len()),
                ],
            ),
            (
                "far first",
                [
                    draw(&far_pipeline, &far_vertices, far.len()),
                    draw(&near_pipeline, &near_vertices, near.len()),
                ],
            ),
        ] {
            let drawn = device
                .render_offscreen(8, 8, black, &draws)
                .expect("rendered");

            for y in 0..8 {
                for x in 0..8 {
                    assert_eq!(
                        drawn.pixel(x, y),
                        Some([0, 255, 0, 255]),
                        "drawing {order}, the nearer wall is what shows at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn refuses_draws_a_pass_cannot_carry() {
        let Some(device) = require_device() else {
            return;
        };

        let library = device.compile_library(WORLD_SHADER).expect("compiled");
        let with_depth = device
            .create_depth_pipeline(&library, "world_vertex", "near_fragment")
            .expect("built");
        let flat = device.compile_library(FLAT_SHADER).expect("compiled");
        let without_depth = device
            .create_pipeline(&flat, "flat_vertex", "flat_fragment")
            .expect("built");

        let positions = wall(64.0, 32.0);
        let vertices = upload_positions(&device, &positions);
        let transform =
            view_projection(eye_at_origin(), SQUARE_90_DEGREE_LENS).expect("a square lens");
        let black = ClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };
        let world = |pipeline, stride| {
            TriangleList::new(
                pipeline,
                Vertices::Buffered {
                    buffer: &vertices,
                    count: positions.len(),
                    stride,
                },
            )
        };
        let position = std::mem::size_of::<[f32; 3]>();

        // A pass whose attachment some of its draws do not expect.
        assert_eq!(
            device
                .render_offscreen(
                    8,
                    8,
                    black,
                    &[
                        world(&with_depth, position).with_uniforms(transform.as_bytes()),
                        world(&without_depth, position),
                    ],
                )
                .err(),
            Some(DeviceError::MixedDepthPipelines {
                with_depth: 1,
                draws: 2,
            })
        );

        // Vertices of no size make any buffer look big enough for any count,
        // so the buffer check would pass and the GPU would read rubbish.
        assert_eq!(
            device
                .render_offscreen(
                    8,
                    8,
                    black,
                    &[world(&with_depth, 0).with_uniforms(transform.as_bytes())],
                )
                .err(),
            Some(DeviceError::EmptyVertexStride)
        );

        // Three-float vertices in a buffer sized for however many the wall
        // has, asked for twice as many.
        assert_eq!(
            device
                .render_offscreen(
                    8,
                    8,
                    black,
                    &[TriangleList::new(
                        &with_depth,
                        Vertices::Buffered {
                            buffer: &vertices,
                            count: positions.len() * 2,
                            stride: position,
                        }
                    )
                    .with_uniforms(transform.as_bytes())],
                )
                .err(),
            Some(DeviceError::VertexBufferTooSmall {
                needed: positions.len() * 2 * position,
                found: positions.len() * position,
            })
        );

        // A per-vertex attribute is read once per vertex, so one holding
        // fewer values than the draw has vertices would have the GPU read
        // past its end. Its stride cannot be inferred from the buffer,
        // which is why a draw states it.
        let short = device.create_buffer(&[0u8; 8]).expect("uploaded");
        let lightmap = device
            .create_texture(1, 1, TextureFormat::Bgra8Unorm, &[0x80, 0x80, 0x80, 0xff])
            .expect("a lightmap uploads");
        for attribute in [
            VertexAttribute::packed::<[f32; 2]>(&short),
            VertexAttribute {
                buffer: &short,
                stride: 0,
            },
        ] {
            let expected = if attribute.stride == 0 {
                DeviceError::EmptyVertexStride
            } else {
                DeviceError::VertexBufferTooSmall {
                    needed: positions.len() * attribute.stride,
                    found: short.len(),
                }
            };
            assert_eq!(
                device
                    .render_offscreen(
                        8,
                        8,
                        black,
                        &[world(&with_depth, position)
                            .with_coordinates(attribute)
                            .with_uniforms(transform.as_bytes())],
                    )
                    .err()
                    .as_ref(),
                Some(&expected),
                "a coordinate buffer is checked against the draw's vertices"
            );
            assert_eq!(
                device
                    .render_offscreen(
                        8,
                        8,
                        black,
                        &[world(&with_depth, position)
                            .with_lightmap(&lightmap, attribute)
                            .with_uniforms(transform.as_bytes())],
                    )
                    .err()
                    .as_ref(),
                Some(&expected),
                "and so is a lightmap's"
            );
        }

        // More constants than Metal's inline path holds.
        let oversized = vec![0u8; MAX_INLINE_UNIFORM_BYTES + 16];
        assert_eq!(
            device
                .render_offscreen(
                    8,
                    8,
                    black,
                    &[world(&with_depth, position).with_uniforms(&oversized)],
                )
                .err(),
            Some(DeviceError::UniformsTooLarge {
                bytes: MAX_INLINE_UNIFORM_BYTES + 16,
                limit: MAX_INLINE_UNIFORM_BYTES,
            })
        );
    }
}
