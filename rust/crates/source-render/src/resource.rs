//! GPU-resident buffers and textures.
//!
//! These are the two resource kinds the engine's content maps onto: geometry
//! becomes vertex buffers and VTF images become textures. Both are held by
//! owning handles so a resource is released when the Rust value is dropped,
//! rather than on a separate lifetime the caller has to track.

use crate::objc::{
    class, msg_send_ptr, selector, send_id_with_id, send_void_with_usize, Id, Owned, Sel,
};
use crate::{DeviceError, Result, TextureFormat};
use std::ffi::c_void;

/// `MTLResourceStorageModeShared`, so the CPU writes are visible to the GPU
/// without an explicit transfer on Apple silicon's unified memory.
const RESOURCE_STORAGE_MODE_SHARED: u64 = 0;
/// `MTLTextureUsageShaderRead`.
const TEXTURE_USAGE_SHADER_READ: u64 = 1;

impl TextureFormat {
    /// The `MTLPixelFormat` value.
    fn metal(self) -> u64 {
        match self {
            Self::Bgra8Unorm => crate::metal::PIXEL_FORMAT_BGRA8_UNORM,
            Self::Bc1Rgba => 130,
            Self::Bc2Rgba => 132,
            Self::Bc3Rgba => 134,
        }
    }

    /// The stride between rows of blocks, which is what Metal wants rather
    /// than a stride between rows of pixels.
    fn bytes_per_row(self, width: u32) -> usize {
        let blocks = width.div_ceil(self.block_extent()) as usize;
        blocks * self.bytes_per_block()
    }

    /// The total stored size of an image.
    fn encoded_len(self, width: u32, height: u32) -> usize {
        let rows = height.div_ceil(self.block_extent()) as usize;
        self.bytes_per_row(width) * rows
    }
}

/// The dimensions of mip level `level` of a `width` by `height` image.
///
/// Levels halve until they reach one pixel and then stop, which is the chain
/// VTF stores and what Metal expects each level to measure.
fn level_extent(width: u32, height: u32, level: u32) -> (u32, u32) {
    ((width >> level).max(1), (height >> level).max(1))
}

/// How many levels a complete chain down to one pixel holds.
fn full_chain_len(width: u32, height: u32) -> usize {
    let longest = width.max(height);
    (u32::BITS - longest.leading_zeros()) as usize
}

/// The host-visible mapping of a shared-storage buffer.
///
/// # Safety
/// `buffer` must be a live Metal buffer using shared storage.
unsafe fn msg_send_contents(buffer: Id) -> *const u8 {
    // SAFETY: the signature matches Metal's declaration of `contents`, which
    // takes no arguments and returns the mapping for shared storage.
    unsafe {
        let send: extern "C" fn(Id, Sel) -> *const c_void = std::mem::transmute(msg_send_ptr());
        send(buffer, selector(c"contents")).cast()
    }
}

/// The width of the entries in an index buffer.
///
/// Source stores its world and model index data as `unsigned short`, which is
/// what nearly every BSP surface and studio mesh needs; the wider entry is
/// here for the occasional batch whose vertex span does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFormat {
    /// `MTLIndexTypeUInt16`.
    Uint16,
    /// `MTLIndexTypeUInt32`.
    Uint32,
}

impl IndexFormat {
    /// The `MTLIndexType` value.
    pub(crate) fn metal(self) -> u64 {
        match self {
            Self::Uint16 => 0,
            Self::Uint32 => 1,
        }
    }

    /// The stored size of one index.
    pub fn size(self) -> usize {
        match self {
            Self::Uint16 => 2,
            Self::Uint32 => 4,
        }
    }
}

/// Bytes the GPU can read, typically vertices.
pub struct Buffer {
    pub(crate) id: Owned,
    length: usize,
}

impl Buffer {
    /// The size in bytes, which bounds what a draw may read out of it.
    pub fn len(&self) -> usize {
        self.length
    }

    /// The largest of the first `count` indices held here, or `None` when the
    /// buffer is too short to hold them.
    ///
    /// # Safety
    /// The buffer must hold at least `count` indices of `format`, and must use
    /// shared storage so its contents are host-visible.
    pub(crate) unsafe fn largest_index(
        &self,
        first: usize,
        count: usize,
        format: IndexFormat,
    ) -> Option<usize> {
        let end = first.checked_add(count)?;
        if self.length < end * format.size() {
            return None;
        }
        // SAFETY: `contents` returns the shared-storage mapping, valid for the
        // buffer's length, and the caller guarantees `count` indices fit.
        let contents = unsafe { msg_send_contents(self.id.as_id()) };
        if contents.is_null() {
            return None;
        }
        // Only the run this draw reads is examined, because a map's world is
        // one buffer of runs and the indices outside this one select
        // vertices this draw never touches.
        match format {
            IndexFormat::Uint16 => {
                let indices = unsafe { std::slice::from_raw_parts(contents.cast::<u16>(), end) };
                indices[first..].iter().copied().max().map(usize::from)
            }
            IndexFormat::Uint32 => {
                let indices = unsafe { std::slice::from_raw_parts(contents.cast::<u32>(), end) };
                indices[first..]
                    .iter()
                    .copied()
                    .max()
                    .map(|index| index as usize)
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
}

/// A two-dimensional image the GPU can sample.
pub struct Texture {
    pub(crate) id: Owned,
    width: u32,
    height: u32,
    format: TextureFormat,
    levels: usize,
}

impl Texture {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    /// How many mip levels were uploaded, counting the base level.
    pub fn levels(&self) -> usize {
        self.levels
    }
}

/// Copies `bytes` into a new GPU buffer.
///
/// # Safety
/// `device` must be a live Metal device, called inside an autorelease pool.
pub(crate) unsafe fn create_buffer(device: Id, bytes: &[u8]) -> Result<Buffer> {
    if bytes.is_empty() {
        return Err(DeviceError::EmptyResource);
    }

    // SAFETY: the signature matches Metal's declaration of
    // newBufferWithBytes:length:options:, the pointer is valid for the length
    // given, and the `new` prefix means the result is owned here.
    let buffer = unsafe {
        let send: extern "C" fn(Id, Sel, *const c_void, u64, u64) -> Id =
            std::mem::transmute(msg_send_ptr());
        let buffer = send(
            device,
            selector(c"newBufferWithBytes:length:options:"),
            bytes.as_ptr().cast(),
            bytes.len() as u64,
            RESOURCE_STORAGE_MODE_SHARED,
        );
        Owned::from_owned(buffer)
    };

    Ok(Buffer {
        id: buffer.ok_or(DeviceError::NoBuffer)?,
        length: bytes.len(),
    })
}

/// Creates a sampleable texture and uploads `bytes` into it.
///
/// For [`TextureFormat::Bgra8Unorm`] the bytes are BGRA in row order, matching
/// both the capture format and the layout the readback returns, so an image
/// can round-trip without a channel swap in between. For the compressed
/// formats they are the stored blocks, uploaded unchanged.
///
/// # Safety
/// `device` must be a live Metal device, called inside an autorelease pool.
pub(crate) unsafe fn create_texture(
    device: Id,
    width: u32,
    height: u32,
    format: TextureFormat,
    levels: &[&[u8]],
) -> Result<Texture> {
    if width == 0 || height == 0 {
        return Err(DeviceError::EmptyTarget);
    }
    if levels.is_empty() {
        return Err(DeviceError::EmptyResource);
    }
    // Metal can only replace whole blocks, so a compressed image that stops
    // part way through one has no valid upload region. This binds the base
    // level only: the tail of a chain legitimately reaches sizes below one
    // block, which Metal stores as a single padded block.
    let extent = format.block_extent();
    if extent > 1 && (width % extent != 0 || height % extent != 0) {
        return Err(DeviceError::UnalignedTexture {
            width,
            height,
            block: extent,
        });
    }
    // A chain may stop early, which is what a material without small mips
    // looks like, but it may not run past the one-pixel level.
    let available = full_chain_len(width, height);
    if levels.len() > available {
        return Err(DeviceError::TooManyLevels {
            found: levels.len(),
            available,
        });
    }
    for (level, bytes) in levels.iter().enumerate() {
        let (level_width, level_height) = level_extent(width, height, level as u32);
        let expected = format.encoded_len(level_width, level_height);
        if bytes.len() == expected {
            continue;
        }
        // The base level is the whole image for a caller that passed one, so
        // it keeps reporting the plain mismatch rather than naming a level
        // the caller never asked to think about.
        return Err(if level == 0 {
            DeviceError::TextureSizeMismatch {
                expected,
                found: bytes.len(),
            }
        } else {
            DeviceError::LevelSizeMismatch {
                level,
                expected,
                found: bytes.len(),
            }
        });
    }

    // SAFETY: the descriptor class comes from the linked framework and each
    // selector below is sent with Metal's declared signature. The descriptor
    // is autoreleased into the caller's pool; the texture is owned here.
    let texture = unsafe {
        let descriptor = crate::metal::texture_descriptor(
            class(c"MTLTextureDescriptor"),
            format.metal(),
            u64::from(width),
            u64::from(height),
        );
        if descriptor.is_null() {
            return Err(DeviceError::NoTexture);
        }
        send_void_with_usize(
            descriptor,
            selector(c"setUsage:"),
            TEXTURE_USAGE_SHADER_READ,
        );
        send_void_with_usize(
            descriptor,
            selector(c"setStorageMode:"),
            crate::metal::STORAGE_MODE_SHARED,
        );
        send_void_with_usize(
            descriptor,
            selector(c"setMipmapLevelCount:"),
            levels.len() as u64,
        );
        let texture = send_id_with_id(device, selector(c"newTextureWithDescriptor:"), descriptor);
        Owned::from_owned(texture)
    }
    .ok_or(DeviceError::NoTexture)?;

    for (level, bytes) in levels.iter().enumerate() {
        let (level_width, level_height) = level_extent(width, height, level as u32);
        let region = crate::metal::Region::whole(level_width, level_height);
        // SAFETY: the texture was created with this many levels, each level
        // measures what `level_extent` reports, and the slice holds exactly
        // the bytes that region covers, checked above.
        unsafe {
            let send: extern "C" fn(Id, Sel, crate::metal::Region, u64, *const c_void, u64) =
                std::mem::transmute(msg_send_ptr());
            send(
                texture.as_id(),
                selector(c"replaceRegion:mipmapLevel:withBytes:bytesPerRow:"),
                region,
                level as u64,
                bytes.as_ptr().cast(),
                format.bytes_per_row(level_width) as u64,
            );
        }
    }

    Ok(Texture {
        id: texture,
        width,
        height,
        format,
        levels: levels.len(),
    })
}
