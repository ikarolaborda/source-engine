//! The Metal device behind the Direct3D 9 classes in `tometal/`.
//!
//! Direct3D 9 is an immediate state machine and Metal is not: a Metal draw
//! needs a render pass, a pipeline compiled for the pass's formats, the vertex
//! layout and the blend state, and separate depth and sampler objects. This
//! reads the state block a draw is handed, finds or builds each of those, and
//! binds only what changed since the last draw in the pass.
//!
//! Every entry point is called by one thread at a time, which is the contract
//! ToGL put on the shader API and the shader API already keeps. Objects are
//! handed to C++ as raw pointers and come back the same way; the C++ classes
//! own them and scrub them out of the state block before destroying them.

use crate::format::{self, FormatInfo};
use crate::msg;
use crate::mtl::*;
use crate::objc::{
    class, error_text, ns_string, AutoreleasePool, Id, MTLCreateSystemDefaultDevice, NSUInteger,
    Owned,
};
use crate::state::{self, rs, samp, Handle, Rect, State, SurfaceRef, VertexElement};
use crate::translate::{self, SamplerKind, Stage, Translated};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

/// The `NSWindow` to present into, set by the window manager before the
/// device exists.
static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn set_window(window: *mut c_void) {
    WINDOW.store(window, Ordering::Release);
}

/// Vertex-stage buffer slots. Streams take their Direct3D number.
const SLOT_MISSING_STREAM: NSUInteger = 15;
const SLOT_VS_FLOATS: NSUInteger = 16;
const SLOT_VS_EXTRA: NSUInteger = 17;
/// Pixel-stage buffer slots.
const SLOT_PS_FLOATS: NSUInteger = 0;
const SLOT_PS_EXTRA: NSUInteger = 1;

/// How many frames a buffer the GPU was given may still be read for. The
/// layer holds three drawables, so no more than three frames are in flight.
const FRAMES_IN_FLIGHT: u64 = 3;

/// The per-draw block the translated shaders declare as `D3DExtra`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Extra {
    ints: [[i32; 4]; 16],
    clip_plane0: [f32; 4],
    pos_fixup: [f32; 4],
    bools: u32,
    alpha_func: u32,
    alpha_ref: f32,
    flags: u32,
}

const UTILITY_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct QuadArgs {
    float4 dst;     // x0, y0, x1, y1 in normalized device coordinates
    float4 src;     // u0, v0, u1, v1
    float4 color;
    float  depth;
};

struct QuadOut {
    float4 position [[position]];
    float2 uv;
};

vertex QuadOut quad_vs(uint vid [[vertex_id]], constant QuadArgs &a [[buffer(0)]]) {
    bool right = (vid & 1) != 0;
    bool bottom = (vid & 2) != 0;
    QuadOut o;
    o.position = float4(right ? a.dst.z : a.dst.x, bottom ? a.dst.w : a.dst.y, a.depth, 1.0);
    o.uv = float2(right ? a.src.z : a.src.x, bottom ? a.src.w : a.src.y);
    return o;
}

fragment float4 clear_fs(constant QuadArgs &a [[buffer(0)]]) {
    return a.color;
}

fragment float4 blit_fs(QuadOut in [[stage_in]],
                        texture2d<float> t [[texture(0)]],
                        sampler s [[sampler(0)]]) {
    return t.sample(s, in.uv);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct QuadArgs {
    dst: [f32; 4],
    src: [f32; 4],
    color: [f32; 4],
    depth: f32,
    _pad: [f32; 3],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextureKind {
    D2,
    Cube,
    Volume,
}

struct Lock {
    face: u32,
    level: u32,
    x: u32,
    y: u32,
    z: u32,
    width: u32,
    height: u32,
    depth: u32,
    pitch: usize,
    readback: bool,
    data: Vec<u8>,
}

pub struct Texture {
    tex: Owned,
    srgb: Option<Owned>,
    kind: TextureKind,
    width: u32,
    height: u32,
    depth: u32,
    levels: u32,
    usage: u32,
    info: FormatInfo,
    shared: bool,
    locks: Vec<Lock>,
}

impl Texture {
    fn level_size(&self, level: u32) -> (u32, u32, u32) {
        (
            (self.width >> level).max(1),
            (self.height >> level).max(1),
            (self.depth >> level).max(1),
        )
    }

    /// The object to bind for sampling or rendering, which is the sRGB view
    /// when conversion is wanted and the format has one.
    fn view(&self, srgb: bool) -> Id {
        match (&self.srgb, srgb) {
            (Some(view), true) => view.id(),
            _ => self.tex.id(),
        }
    }

    fn format(&self, srgb: bool) -> u64 {
        if srgb && self.srgb.is_some() {
            self.info.metal_srgb
        } else {
            self.info.metal
        }
    }
}

pub struct Buffer {
    buf: Owned,
    contents: *mut u8,
    size: u32,
    index_size: u32,
    /// The frame a draw last read this buffer in, or zero for never.
    last_used: u64,
}

pub struct VertexDeclaration {
    id: u64,
    elements: Vec<VertexElement>,
}

/// One translation of a shader. Direct3D bytecode does not say which of a
/// pixel shader's samplers are shadow maps, and Metal has to be told, because
/// a depth comparison is a different texture type and a different call. So a
/// shader is translated once with none, which is nearly every draw, and again
/// for each set of shadow samplers a draw turns out to bind.
struct Variant {
    translated: Translated,
    library: Option<Owned>,
    /// The compiled entry point, per value of the alpha-test function
    /// constant. A vertex function only ever has the `false` one.
    functions: [Option<Owned>; 2],
    failed: bool,
}

pub struct Shader {
    id: u64,
    name: String,
    bytecode: Vec<u32>,
    /// The translation with no shadow samplers, which everything that only
    /// needs the shader's interface reads.
    translated: Translated,
    /// Keyed by the mask of samplers read as shadow maps.
    variants: HashMap<u32, Variant>,
}

/// An occlusion query. Its answer is a sample count the GPU writes into the
/// visibility buffer of the command buffer the query was drawn in, which can
/// only be read once that command buffer has completed.
pub struct Query {
    /// The submission and slot the count will land in, until it is read.
    pending: Option<(u64, usize)>,
    result: Option<u32>,
}

/// One command buffer's worth of work and the visibility results it writes.
struct Submission {
    serial: u64,
    command_buffer: Option<Owned>,
    visibility: Owned,
    counts: *mut u64,
}

/// Slots in one visibility buffer. The engine issues a few dozen queries a
/// frame, for light glows and the sun.
const VISIBILITY_SLOTS: usize = 2048;
/// Submissions kept so a late poll can still find its answer.
const SUBMISSIONS_KEPT: usize = 8;
const VISIBILITY_DISABLED: NSUInteger = 0;
const VISIBILITY_COUNTING: NSUInteger = 2;
const COMMAND_BUFFER_COMPLETED: NSUInteger = 4;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
struct PassKey {
    color: SurfaceRef,
    depth: SurfaceRef,
    srgb: bool,
}

#[derive(Default)]
struct PendingClear {
    key: PassKey,
    color: Option<[f64; 4]>,
    depth: Option<f64>,
    stencil: Option<u32>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineKey {
    vs: u64,
    ps: u64,
    shadow_samplers: u32,
    alpha_test: bool,
    decl: u64,
    strides: [u32; state::STREAMS],
    color_format: u64,
    depth_format: u64,
    stencil: bool,
    blend: [u32; 7],
    write_mask: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct UtilityKey {
    fragment: &'static str,
    color_format: u64,
    depth_format: u64,
    stencil: bool,
    write_color: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DepthKey {
    depth: [u32; 3],
    stencil: [u32; 12],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SamplerKey {
    address: [u32; 3],
    filter: [u32; 3],
    anisotropy: u32,
    max_mip: u32,
    /// A shadow-map lookup: the sampler compares rather than returns.
    compare: bool,
}

/// What the open encoder already has bound, so that a draw sends only what
/// differs. It dies with the encoder: nothing carries from one pass to the
/// next in Metal.
#[derive(Default)]
struct Bound {
    pipeline: Id0,
    depth_state: Id0,
    stencil_ref: Option<u32>,
    cull: Option<NSUInteger>,
    fill: Option<NSUInteger>,
    bias: Option<(u32, u32)>,
    viewport: Option<MTLViewport>,
    scissor: Option<MTLScissorRect>,
    vertex_buffers: [(Id0, u64); 16],
    textures: [Id0; state::SAMPLERS],
    samplers: [Id0; state::SAMPLERS],
}

/// A nullable object pointer that defaults to null, for the bound-state cache.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Id0(Id);

impl Default for Id0 {
    fn default() -> Self {
        Self(std::ptr::null_mut())
    }
}

struct Pass {
    encoder: Owned,
    key: PassKey,
    color_format: u64,
    depth_format: u64,
    stencil: bool,
    width: u32,
    height: u32,
    bound: Bound,
}

struct RingBuffer {
    buf: Owned,
    contents: *mut u8,
    retired: u64,
}

/// Per-draw constants. Each draw's registers are copied to the end of a
/// large shared buffer, which is cheaper than a Metal buffer per draw and has
/// no size limit the way inline bytes do.
struct Ring {
    chunk: usize,
    current: Option<RingBuffer>,
    offset: usize,
    used: Vec<RingBuffer>,
    free: Vec<RingBuffer>,
}

pub struct Device {
    mtl: Owned,
    queue: Owned,
    layer: Option<Owned>,
    window: Id,
    command_buffer: Option<Owned>,
    /// Newest last. The last entry belongs to the open command buffer.
    submissions: std::collections::VecDeque<Submission>,
    spare_visibility: Vec<(Owned, *mut u64)>,
    serial: u64,
    next_slot: usize,
    /// The slot an issued-but-unfinished query counts into, and whether the
    /// open encoder has been told to count yet.
    counting: Option<(usize, bool)>,
    pass: Option<Pass>,
    pending_clear: Option<PendingClear>,
    frame: u64,
    next_id: u64,
    pipelines: HashMap<PipelineKey, Option<Owned>>,
    utility_library: Option<Owned>,
    utility_pipelines: HashMap<UtilityKey, Option<Owned>>,
    depth_states: HashMap<DepthKey, Owned>,
    samplers: HashMap<SamplerKey, Owned>,
    dummy_2d: Option<Owned>,
    dummy_cube: Option<Owned>,
    dummy_3d: Option<Owned>,
    missing_stream: Option<Owned>,
    ring: Ring,
    warned: HashSet<String>,
    drawable_size: (u32, u32),
    stats_draws: u64,
    /// Queries begun, ended with a draw counted, polled before ready, answered,
    /// and answered with a nonzero count, since the last report.
    stats_queries: [u64; 5],
    /// Where to write the back buffer every so many frames, when asked to by
    /// SOURCE_D3D9_SHOT_DIR and SOURCE_D3D9_SHOT_EVERY. A run can then be
    /// looked at without capturing the screen it happens to be on.
    shots: Option<(std::path::PathBuf, u64)>,
}

fn log(text: &str) {
    eprintln!("d3d9metal: {text}");
}

impl Device {
    /// Opens the system's Metal device and, when the window manager has
    /// handed a window over, puts a Metal layer on it.
    pub fn create(width: u32, height: u32) -> Option<Box<Self>> {
        let _pool = AutoreleasePool::new();
        // SAFETY: the returned device follows the Create rule, so the
        // reference is ours; `newCommandQueue` likewise.
        let (mtl, queue) = unsafe {
            let mtl = Owned::from_owned(MTLCreateSystemDefaultDevice())?;
            let queue = Owned::from_owned(msg![ret: Id; mtl.id(), "newCommandQueue"])?;
            (mtl, queue)
        };

        let mut device = Box::new(Self {
            mtl,
            queue,
            layer: None,
            window: WINDOW.load(Ordering::Acquire),
            command_buffer: None,
            submissions: std::collections::VecDeque::new(),
            spare_visibility: Vec::new(),
            serial: 0,
            next_slot: 0,
            counting: None,
            pass: None,
            pending_clear: None,
            frame: 1,
            next_id: 1,
            pipelines: HashMap::new(),
            utility_library: None,
            utility_pipelines: HashMap::new(),
            depth_states: HashMap::new(),
            samplers: HashMap::new(),
            dummy_2d: None,
            dummy_cube: None,
            dummy_3d: None,
            missing_stream: None,
            ring: Ring {
                chunk: 4 << 20,
                current: None,
                offset: 0,
                used: Vec::new(),
                free: Vec::new(),
            },
            warned: HashSet::new(),
            drawable_size: (0, 0),
            stats_draws: 0,
            stats_queries: [0; 5],
            shots: std::env::var_os("SOURCE_D3D9_SHOT_DIR").map(|dir| {
                let every = std::env::var("SOURCE_D3D9_SHOT_EVERY")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600);
                (std::path::PathBuf::from(dir), u64::max(every, 1))
            }),
        });

        device.attach_layer();
        device.create_fallbacks();
        // SAFETY: `name` returns an NSString owned by the device.
        let name = unsafe { crate::objc::string_from_ns(msg![ret: Id; device.mtl.id(), "name"]) };
        log(&format!(
            "device '{name}', back buffer {width}x{height}, layer {}",
            if device.layer.is_some() {
                "attached"
            } else {
                "absent"
            }
        ));
        Some(device)
    }

    fn warn_once(&mut self, text: String) {
        if self.warned.insert(text.clone()) {
            log(&text);
        }
    }

    fn unique_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Gives the window's content view a `CAMetalLayer` to present through.
    /// This runs on the main thread, where the shader API creates its device,
    /// which is what AppKit requires of a change to a view.
    fn attach_layer(&mut self) {
        if self.window.is_null() {
            return;
        }
        // SAFETY: `window` is the live NSWindow the window manager handed
        // over, the selectors are NSWindow's, NSView's and CAMetalLayer's
        // own, and the layer is retained before the pool can release it.
        unsafe {
            let view = msg![ret: Id; self.window, "contentView"];
            if view.is_null() {
                return;
            }
            let layer = msg![ret: Id; class(c"CAMetalLayer"), "layer"];
            let Some(layer) = Owned::retain(layer) else {
                return;
            };
            msg![layer.id(), "setDevice:", self.mtl.id() => Id];
            msg![layer.id(), "setPixelFormat:", format::mtlfmt::BGRA8_UNORM => NSUInteger];
            msg![layer.id(), "setFramebufferOnly:", true => bool];
            msg![layer.id(), "setOpaque:", true => bool];
            msg![view, "setWantsLayer:", true => bool];
            msg![view, "setLayer:", layer.id() => Id];
            self.layer = Some(layer);
        }
        self.sync_drawable_size();
    }

    /// Keeps the layer's drawable the size of the view in pixels, which
    /// changes when the window is resized, goes full screen or is dragged to
    /// a display of another scale.
    fn sync_drawable_size(&mut self) {
        let Some(layer) = &self.layer else {
            return;
        };
        // SAFETY: the window and its view are live, `bounds` returns a CGRect
        // of four doubles and `backingScaleFactor` a double, which the
        // platform returns in floating-point registers as declared.
        unsafe {
            let view = msg![ret: Id; self.window, "contentView"];
            if view.is_null() {
                return;
            }
            let bounds = msg![ret: CGRect; view, "bounds"];
            let scale = msg![ret: f64; self.window, "backingScaleFactor"];
            let scale = if scale > 0.0 { scale } else { 1.0 };
            let width = (bounds.width * scale).round().max(1.0) as u32;
            let height = (bounds.height * scale).round().max(1.0) as u32;
            if (width, height) != self.drawable_size {
                msg![layer.id(), "setContentsScale:", scale => f64];
                let size = CGSize {
                    width: f64::from(width),
                    height: f64::from(height),
                };
                msg![layer.id(), "setDrawableSize:", size => CGSize];
                self.drawable_size = (width, height);
                log(&format!("drawable {width}x{height} at scale {scale}"));
            }
        }
    }

    pub fn drawable_size(&self) -> (u32, u32) {
        self.drawable_size
    }

    /// Textures to bind where a shader samples a stage nothing is bound to,
    /// or one of the wrong dimensionality, and the vertex data read by an
    /// input the declaration does not supply.
    fn create_fallbacks(&mut self) {
        let white = [0xFFu8; 4];
        self.dummy_2d = self.new_plain_texture(TEXTURE_TYPE_2D, &white);
        self.dummy_cube = self.new_plain_texture(TEXTURE_TYPE_CUBE, &white);
        self.dummy_3d = self.new_plain_texture(TEXTURE_TYPE_3D, &white);

        // (0, 0, 0, 1), which is what OpenGL gives a disabled attribute and
        // so what the game was last seen running against.
        let missing: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
        // SAFETY: the bytes are valid for the length given, and the method
        // copies them into a buffer this owns.
        self.missing_stream = unsafe {
            Owned::from_owned(
                msg![ret: Id; self.mtl.id(), "newBufferWithBytes:length:options:",
                missing.as_ptr().cast::<c_void>() => *const c_void,
                16 => NSUInteger,
                RESOURCE_STORAGE_SHARED => NSUInteger],
            )
        };
    }

    fn new_plain_texture(&self, kind: NSUInteger, pixel: &[u8; 4]) -> Option<Owned> {
        // SAFETY: a descriptor from alloc/init is owned here and released at
        // the end of the block; the texture copies the pixel it is given.
        unsafe {
            let desc = Owned::from_owned(
                msg![ret: Id; msg![ret: Id; class(c"MTLTextureDescriptor"), "alloc"], "init"],
            )?;
            msg![desc.id(), "setTextureType:", kind => NSUInteger];
            msg![desc.id(), "setPixelFormat:", format::mtlfmt::BGRA8_UNORM => NSUInteger];
            msg![desc.id(), "setWidth:", 1 => NSUInteger];
            msg![desc.id(), "setHeight:", 1 => NSUInteger];
            msg![desc.id(), "setUsage:", TEXTURE_USAGE_SHADER_READ => NSUInteger];
            msg![desc.id(), "setStorageMode:", STORAGE_MODE_SHARED => NSUInteger];
            let tex = Owned::from_owned(
                msg![ret: Id; self.mtl.id(), "newTextureWithDescriptor:", desc.id() => Id],
            )?;
            let region = MTLRegion {
                origin: MTLOrigin::default(),
                size: MTLSize {
                    width: 1,
                    height: 1,
                    depth: 1,
                },
            };
            let slices = if kind == TEXTURE_TYPE_CUBE { 6 } else { 1 };
            for slice in 0..slices {
                msg![tex.id(), "replaceRegion:mipmapLevel:slice:withBytes:bytesPerRow:bytesPerImage:",
                    region => MTLRegion, 0 => NSUInteger, slice => NSUInteger,
                    pixel.as_ptr().cast::<c_void>() => *const c_void, 4 => NSUInteger, 4 => NSUInteger];
            }
            Some(tex)
        }
    }

    pub fn reset(&mut self, _width: u32, _height: u32) {
        let _pool = AutoreleasePool::new();
        self.flush(false);
        self.sync_drawable_size();
    }

    // ------------------------------------------------------------------
    // Command buffer and pass management
    // ------------------------------------------------------------------

    fn command_buffer(&mut self) -> Id {
        if let Some(buffer) = &self.command_buffer {
            return buffer.id();
        }
        // SAFETY: `commandBuffer` returns an autoreleased object, retained
        // here so that it outlives the caller's pool.
        let buffer = unsafe { Owned::retain(msg![ret: Id; self.queue.id(), "commandBuffer"]) };
        let id = buffer.as_ref().map_or(std::ptr::null_mut(), Owned::id);
        self.command_buffer = buffer;
        if !id.is_null() {
            self.begin_submission();
        }
        id
    }

    /// Gives the new command buffer a zeroed visibility buffer of its own, so
    /// that a query's answer cannot be overwritten by a later frame before
    /// the engine has polled for it.
    fn begin_submission(&mut self) {
        while self.submissions.len() >= SUBMISSIONS_KEPT {
            let done = self.submissions.front().is_some_and(|oldest| {
                oldest.command_buffer.as_ref().is_none_or(|buffer| {
                    // SAFETY: `status` is a plain property of a live object.
                    unsafe {
                        msg![ret: NSUInteger; buffer.id(), "status"] >= COMMAND_BUFFER_COMPLETED
                    }
                })
            });
            if !done {
                break;
            }
            if let Some(oldest) = self.submissions.pop_front() {
                self.spare_visibility
                    .push((oldest.visibility, oldest.counts));
            }
        }

        let recycled = self.spare_visibility.pop();
        let fresh = recycled.or_else(|| {
            let (buffer, contents) = self.new_metal_buffer((VISIBILITY_SLOTS * 8) as u32)?;
            Some((buffer, contents.cast::<u64>()))
        });
        let Some((visibility, counts)) = fresh else {
            return;
        };
        // SAFETY: the buffer is `VISIBILITY_SLOTS` counts long and no command
        // buffer that writes it is in flight, having completed or never run.
        unsafe { std::ptr::write_bytes(counts, 0, VISIBILITY_SLOTS) };

        self.serial += 1;
        self.next_slot = 0;
        self.submissions.push_back(Submission {
            serial: self.serial,
            command_buffer: None,
            visibility,
            counts,
        });
    }

    fn end_pass(&mut self) {
        if let Some(pass) = self.pass.take() {
            // SAFETY: the encoder is live and has not been ended.
            unsafe { msg![pass.encoder.id(), "endEncoding"] };
        }
        // A new encoder starts with counting off whatever the last was doing.
        if let Some((_, applied)) = &mut self.counting {
            *applied = false;
        }
    }

    /// Ends the open pass, realises a clear nothing drew over, and submits
    /// what has been encoded. Waits for the GPU when asked to, which is what
    /// a read-back needs.
    fn flush(&mut self, wait: bool) {
        self.end_pass();
        self.flush_pending_clear();
        if let Some(buffer) = self.command_buffer.take() {
            // SAFETY: the buffer is live and uncommitted.
            unsafe {
                msg![buffer.id(), "commit"];
                if wait {
                    msg![buffer.id(), "waitUntilCompleted"];
                }
            }
            // Kept so a query can ask whether its answer has been written.
            if let Some(submission) = self.submissions.back_mut() {
                submission.command_buffer = Some(buffer);
            }
        }
    }

    /// A clear of a whole target is held until the first draw, so that it
    /// becomes the pass's load action rather than a pass of its own. If
    /// something else has to happen first, it is given that pass after all.
    fn flush_pending_clear(&mut self) {
        if let Some(pending) = self.pending_clear.take() {
            self.end_pass();
            if self.begin_pass(pending.key, Some(&pending)) {
                self.end_pass();
            }
        }
    }

    fn texture<'a>(handle: Handle) -> Option<&'a mut Texture> {
        // SAFETY: a nonzero handle is a pointer from `Box::into_raw` that the
        // C++ owner has not yet destroyed, and one thread calls in at a time.
        unsafe { (handle as *mut Texture).as_mut() }
    }

    fn begin_pass(&mut self, key: PassKey, clear: Option<&PendingClear>) -> bool {
        let Some(color) = Self::texture(key.color.texture) else {
            return false;
        };
        let (width, height, _) = color.level_size(key.color.level);
        let color_view = color.view(key.srgb);
        let color_format = color.format(key.srgb);

        let mut depth_view: Id = std::ptr::null_mut();
        let mut depth_format = format::mtlfmt::INVALID;
        let mut stencil = false;
        if let Some(depth) = Self::texture(key.depth.texture) {
            let (dw, dh, _) = depth.level_size(key.depth.level);
            if dw >= width && dh >= height && depth.info.depth {
                depth_view = depth.tex.id();
                depth_format = depth.info.metal;
                stencil = depth.info.stencil;
            } else {
                self.warn_once(format!(
                    "depth surface {dw}x{dh} is smaller than its {width}x{height} target and was left off the pass"
                ));
            }
        }

        let command_buffer = self.command_buffer();
        if command_buffer.is_null() {
            return false;
        }
        let visibility = self
            .submissions
            .back()
            .map_or(std::ptr::null_mut(), |s| s.visibility.id());

        // SAFETY: the descriptor is autoreleased and used only within the
        // caller's pool; every object set on it is live; the encoder is
        // retained because it has to outlive that pool.
        let encoder = unsafe {
            let desc = msg![ret: Id; class(c"MTLRenderPassDescriptor"), "renderPassDescriptor"];
            if !visibility.is_null() {
                msg![desc, "setVisibilityResultBuffer:", visibility => Id];
            }
            let attachments = msg![ret: Id; desc, "colorAttachments"];
            let attachment =
                msg![ret: Id; attachments, "objectAtIndexedSubscript:", 0 => NSUInteger];
            msg![attachment, "setTexture:", color_view => Id];
            msg![attachment, "setLevel:", NSUInteger::from(key.color.level) => NSUInteger];
            msg![attachment, "setSlice:", NSUInteger::from(key.color.face) => NSUInteger];
            msg![attachment, "setStoreAction:", STORE_ACTION_STORE => NSUInteger];
            match clear.and_then(|c| c.color) {
                Some(c) => {
                    msg![attachment, "setLoadAction:", LOAD_ACTION_CLEAR => NSUInteger];
                    let value = MTLClearColor {
                        red: c[0],
                        green: c[1],
                        blue: c[2],
                        alpha: c[3],
                    };
                    msg![attachment, "setClearColor:", value => MTLClearColor];
                }
                None => msg![attachment, "setLoadAction:", LOAD_ACTION_LOAD => NSUInteger],
            }

            if !depth_view.is_null() {
                let attachment = msg![ret: Id; desc, "depthAttachment"];
                msg![attachment, "setTexture:", depth_view => Id];
                msg![attachment, "setStoreAction:", STORE_ACTION_STORE => NSUInteger];
                match clear.and_then(|c| c.depth) {
                    Some(z) => {
                        msg![attachment, "setLoadAction:", LOAD_ACTION_CLEAR => NSUInteger];
                        msg![attachment, "setClearDepth:", z => f64];
                    }
                    None => msg![attachment, "setLoadAction:", LOAD_ACTION_LOAD => NSUInteger],
                }
                if stencil {
                    let attachment = msg![ret: Id; desc, "stencilAttachment"];
                    msg![attachment, "setTexture:", depth_view => Id];
                    msg![attachment, "setStoreAction:", STORE_ACTION_STORE => NSUInteger];
                    match clear.and_then(|c| c.stencil) {
                        Some(s) => {
                            msg![attachment, "setLoadAction:", LOAD_ACTION_CLEAR => NSUInteger];
                            msg![attachment, "setClearStencil:", s => u32];
                        }
                        None => msg![attachment, "setLoadAction:", LOAD_ACTION_LOAD => NSUInteger],
                    }
                }
            }

            let encoder =
                msg![ret: Id; command_buffer, "renderCommandEncoderWithDescriptor:", desc => Id];
            let Some(encoder) = Owned::retain(encoder) else {
                return false;
            };
            // Direct3D's front face is the clockwise one, and neither API
            // flips the image on the way to the target, so it stays so.
            msg![encoder.id(), "setFrontFacingWinding:", WINDING_CLOCKWISE => NSUInteger];
            encoder
        };

        self.pass = Some(Pass {
            encoder,
            key,
            color_format,
            depth_format,
            stencil,
            width,
            height,
            bound: Bound::default(),
        });
        true
    }

    fn pass_key(state: &State) -> PassKey {
        let color = state.render_targets[0];
        let srgb = state.render_states[rs::SRGBWRITEENABLE] != 0
            && Self::texture(color.texture).is_some_and(|t| t.srgb.is_some());
        PassKey {
            color,
            depth: state.depth_stencil,
            srgb,
        }
    }

    /// Makes the open pass the one the state block's targets describe.
    fn ensure_pass(&mut self, state: &State) -> bool {
        let key = Self::pass_key(state);
        if key.color.texture == 0 {
            return false;
        }
        if self.pass.as_ref().is_some_and(|pass| pass.key == key) && self.pending_clear.is_none() {
            return true;
        }

        self.end_pass();
        match self.pending_clear.take() {
            Some(pending) if pending.key == key => self.begin_pass(key, Some(&pending)),
            Some(pending) => {
                if self.begin_pass(pending.key, Some(&pending)) {
                    self.end_pass();
                }
                self.begin_pass(key, None)
            }
            None => self.begin_pass(key, None),
        }
    }

    // ------------------------------------------------------------------
    // Textures
    // ------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn create_texture(
        &mut self,
        kind: u32,
        width: u32,
        height: u32,
        depth: u32,
        levels: u32,
        usage: u32,
        d3d_format: u32,
        label: Option<&str>,
    ) -> Option<Box<Texture>> {
        let _pool = AutoreleasePool::new();
        let Some(info) = format::lookup(d3d_format) else {
            self.warn_once(format!("texture format {d3d_format:#x} is not supported"));
            return None;
        };
        let (kind, mtl_kind) = match kind {
            state::TEXTURE_CUBE => (TextureKind::Cube, TEXTURE_TYPE_CUBE),
            state::TEXTURE_VOLUME => (TextureKind::Volume, TEXTURE_TYPE_3D),
            _ => (TextureKind::D2, TEXTURE_TYPE_2D),
        };
        if width == 0 || height == 0 {
            return None;
        }

        let target = usage & (state::D3DUSAGE_RENDERTARGET | state::D3DUSAGE_DEPTHSTENCIL) != 0
            || info.depth;
        let mut mtl_usage = TEXTURE_USAGE_SHADER_READ;
        if target {
            mtl_usage |= TEXTURE_USAGE_RENDER_TARGET;
        }
        if info.metal_srgb != format::mtlfmt::INVALID {
            mtl_usage |= TEXTURE_USAGE_PIXEL_FORMAT_VIEW;
        }
        // Depth stays on the GPU. Everything else is shared, which on unified
        // memory costs nothing and lets a lock upload or read back directly.
        let shared = !info.depth;
        let levels = levels.max(1);

        // SAFETY: the descriptor is owned for the block; the texture and its
        // view follow the `new` rule and are owned by the returned value.
        unsafe {
            let desc = Owned::from_owned(
                msg![ret: Id; msg![ret: Id; class(c"MTLTextureDescriptor"), "alloc"], "init"],
            )?;
            msg![desc.id(), "setTextureType:", mtl_kind => NSUInteger];
            msg![desc.id(), "setPixelFormat:", info.metal => NSUInteger];
            msg![desc.id(), "setWidth:", NSUInteger::from(width) => NSUInteger];
            msg![desc.id(), "setHeight:", NSUInteger::from(height) => NSUInteger];
            if kind == TextureKind::Volume {
                msg![desc.id(), "setDepth:", NSUInteger::from(depth.max(1)) => NSUInteger];
            }
            msg![desc.id(), "setMipmapLevelCount:", NSUInteger::from(levels) => NSUInteger];
            msg![desc.id(), "setUsage:", mtl_usage => NSUInteger];
            msg![desc.id(), "setStorageMode:",
                (if shared { STORAGE_MODE_SHARED } else { STORAGE_MODE_PRIVATE }) => NSUInteger];

            let tex = Owned::from_owned(
                msg![ret: Id; self.mtl.id(), "newTextureWithDescriptor:", desc.id() => Id],
            );
            let Some(tex) = tex else {
                self.warn_once(format!(
                    "Metal refused a {width}x{height}x{depth} texture, format {d3d_format:#x}, {levels} levels, usage {usage:#x}"
                ));
                return None;
            };
            if let Some(label) = label.and_then(ns_string) {
                msg![tex.id(), "setLabel:", label.id() => Id];
            }
            let srgb = if info.metal_srgb != format::mtlfmt::INVALID {
                Owned::from_owned(
                    msg![ret: Id; tex.id(), "newTextureViewWithPixelFormat:", info.metal_srgb => NSUInteger],
                )
            } else {
                None
            };

            Some(Box::new(Texture {
                tex,
                srgb,
                kind,
                width,
                height,
                depth: depth.max(1),
                levels,
                usage,
                info,
                shared,
                locks: Vec::new(),
            }))
        }
    }

    pub fn destroy_texture(&mut self, handle: Handle) {
        let in_pass = self.pass.as_ref().is_some_and(|pass| {
            pass.key.color.texture == handle || pass.key.depth.texture == handle
        });
        if in_pass {
            let _pool = AutoreleasePool::new();
            self.end_pass();
        }
        if self
            .pending_clear
            .as_ref()
            .is_some_and(|p| p.key.color.texture == handle || p.key.depth.texture == handle)
        {
            self.pending_clear = None;
        }
        if handle != 0 {
            // SAFETY: the handle came from `Box::into_raw` and its owner is
            // destroying it exactly once.
            drop(unsafe { Box::from_raw(handle as *mut Texture) });
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn lock_texture(
        &mut self,
        handle: Handle,
        face: u32,
        level: u32,
        rect: Option<Rect>,
        front: u32,
        back: u32,
        readback: bool,
    ) -> Option<(*mut u8, usize, usize)> {
        let _pool = AutoreleasePool::new();
        let texture = Self::texture(handle)?;
        if level >= texture.levels {
            return None;
        }
        let (lw, lh, ld) = texture.level_size(level);
        let (x, y, width, height) = match rect {
            Some(r) if r.right > r.left && r.bottom > r.top => {
                let x = r.left.max(0) as u32;
                let y = r.top.max(0) as u32;
                (
                    x,
                    y,
                    (r.right as u32).min(lw) - x.min(lw),
                    (r.bottom as u32).min(lh) - y.min(lh),
                )
            }
            _ => (0, 0, lw, lh),
        };
        let (z, depth) = if texture.kind == TextureKind::Volume && back > front {
            (front, back.min(ld) - front.min(ld))
        } else {
            (
                0,
                if texture.kind == TextureKind::Volume {
                    ld
                } else {
                    1
                },
            )
        };
        if width == 0 || height == 0 || depth == 0 {
            return None;
        }

        let pitch = texture.info.row_bytes(width);
        let slice_bytes = pitch * texture.info.rows(height);
        let mut data = vec![0u8; slice_bytes * depth as usize];

        if readback {
            if !texture.shared || texture.info.expand != format::Expand::None {
                self.warn_once(String::from(
                    "a depth surface was locked for reading, which is not supported",
                ));
                return None;
            }
            // Whatever was last drawn into it has to have happened first.
            self.flush(true);
            let texture = Self::texture(handle)?;
            let region = MTLRegion {
                origin: MTLOrigin {
                    x: x.into(),
                    y: y.into(),
                    z: z.into(),
                },
                size: MTLSize {
                    width: width.into(),
                    height: height.into(),
                    depth: depth.into(),
                },
            };
            // SAFETY: `data` is exactly the size the region, row stride and
            // image stride describe, and the texture is CPU-visible.
            unsafe {
                msg![texture.tex.id(), "getBytes:bytesPerRow:bytesPerImage:fromRegion:mipmapLevel:slice:",
                    data.as_mut_ptr().cast::<c_void>() => *mut c_void,
                    pitch as NSUInteger => NSUInteger, slice_bytes as NSUInteger => NSUInteger,
                    region => MTLRegion, NSUInteger::from(level) => NSUInteger, NSUInteger::from(face) => NSUInteger];
            }
        }

        let texture = Self::texture(handle)?;
        texture
            .locks
            .retain(|lock| !(lock.face == face && lock.level == level));
        texture.locks.push(Lock {
            face,
            level,
            x,
            y,
            z,
            width,
            height,
            depth,
            pitch,
            readback,
            data,
        });
        let lock = texture.locks.last_mut()?;
        Some((lock.data.as_mut_ptr(), pitch, slice_bytes))
    }

    pub fn unlock_texture(&mut self, handle: Handle, face: u32, level: u32) {
        let _pool = AutoreleasePool::new();
        let Some(texture) = Self::texture(handle) else {
            return;
        };
        let Some(index) = texture
            .locks
            .iter()
            .position(|lock| lock.face == face && lock.level == level)
        else {
            return;
        };
        let mut lock = texture.locks.swap_remove(index);
        if lock.readback {
            return;
        }
        if !texture.shared {
            self.warn_once(String::from(
                "a depth surface was locked for writing, which is not supported",
            ));
            return;
        }

        if texture.info.opaque_alpha {
            for pixel in lock.data.chunks_exact_mut(4) {
                pixel[3] = 0xFF;
            }
        }

        // Formats Metal lacks are widened to the BGRA the texture really is.
        let mut pitch = lock.pitch;
        if texture.info.expand != format::Expand::None {
            lock.data = texture.info.expand.apply(&lock.data);
            pitch = lock.width as usize * 4;
        }

        let region = MTLRegion {
            origin: MTLOrigin {
                x: lock.x.into(),
                y: lock.y.into(),
                z: lock.z.into(),
            },
            size: MTLSize {
                width: lock.width.into(),
                height: lock.height.into(),
                depth: lock.depth.into(),
            },
        };
        let slice_bytes = pitch * texture.info.rows(lock.height);
        // SAFETY: the scratch buffer holds exactly the region described, in
        // rows of `pitch` bytes and images of `slice_bytes`.
        unsafe {
            msg![texture.tex.id(), "replaceRegion:mipmapLevel:slice:withBytes:bytesPerRow:bytesPerImage:",
                region => MTLRegion, NSUInteger::from(level) => NSUInteger, NSUInteger::from(face) => NSUInteger,
                lock.data.as_ptr().cast::<c_void>() => *const c_void,
                pitch as NSUInteger => NSUInteger, slice_bytes as NSUInteger => NSUInteger];
        }

        if level == 0
            && texture.usage & state::D3DUSAGE_AUTOGENMIPMAP != 0
            && texture.levels > 1
            && texture.info.block == 1
        {
            let tex = texture.tex.id();
            self.generate_mipmaps(tex);
        }
    }

    fn generate_mipmaps(&mut self, tex: Id) {
        self.end_pass();
        self.flush_pending_clear();
        let command_buffer = self.command_buffer();
        if command_buffer.is_null() {
            return;
        }
        // SAFETY: the blit encoder is used and ended within this pool.
        unsafe {
            let blit = msg![ret: Id; command_buffer, "blitCommandEncoder"];
            msg![blit, "generateMipmapsForTexture:", tex => Id];
            msg![blit, "endEncoding"];
        }
    }

    // ------------------------------------------------------------------
    // Buffers
    // ------------------------------------------------------------------

    fn new_metal_buffer(&self, size: u32) -> Option<(Owned, *mut u8)> {
        // SAFETY: `newBufferWithLength:` follows the `new` rule, and a shared
        // buffer's contents pointer is valid for the buffer's lifetime.
        unsafe {
            let buf = Owned::from_owned(
                msg![ret: Id; self.mtl.id(), "newBufferWithLength:options:",
                NSUInteger::from(size.max(4)) => NSUInteger, RESOURCE_STORAGE_SHARED => NSUInteger],
            )?;
            let contents = msg![ret: *mut c_void; buf.id(), "contents"].cast::<u8>();
            if contents.is_null() {
                return None;
            }
            Some((buf, contents))
        }
    }

    pub fn create_buffer(&mut self, size: u32, index_size: u32) -> Option<Box<Buffer>> {
        let _pool = AutoreleasePool::new();
        let (buf, contents) = self.new_metal_buffer(size)?;
        Some(Box::new(Buffer {
            buf,
            contents,
            size,
            index_size,
            last_used: 0,
        }))
    }

    pub fn destroy_buffer(&mut self, handle: Handle) {
        if handle != 0 {
            // SAFETY: as for textures. Metal keeps the underlying buffer
            // alive for any command buffer that still refers to it.
            drop(unsafe { Box::from_raw(handle as *mut Buffer) });
        }
    }

    /// Hands out the buffer's own memory. When the GPU may still be reading
    /// what is there, the buffer is first swapped for a fresh one, which is
    /// what Direct3D's DISCARD asks for outright and what a plain lock needs
    /// to be safe; NOOVERWRITE is the caller's promise that it is not needed.
    pub fn lock_buffer(&mut self, handle: Handle, offset: u32, size: u32, flags: u32) -> *mut u8 {
        // SAFETY: as for textures.
        let Some(buffer) = (unsafe { (handle as *mut Buffer).as_mut() }) else {
            return std::ptr::null_mut();
        };
        if offset > buffer.size {
            return std::ptr::null_mut();
        }

        let in_flight =
            buffer.last_used != 0 && self.frame.saturating_sub(buffer.last_used) < FRAMES_IN_FLIGHT;
        if in_flight && flags & state::D3DLOCK_NOOVERWRITE == 0 {
            let _pool = AutoreleasePool::new();
            if let Some((fresh, contents)) = self.new_metal_buffer(buffer.size) {
                let whole = offset == 0 && (size == 0 || size >= buffer.size);
                if flags & state::D3DLOCK_DISCARD == 0 && !whole {
                    // SAFETY: both allocations are `buffer.size` bytes long
                    // and distinct.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            buffer.contents,
                            contents,
                            buffer.size as usize,
                        )
                    };
                }
                buffer.buf = fresh;
                buffer.contents = contents;
                buffer.last_used = 0;
            }
        }
        // SAFETY: `offset` was checked against the allocation's size.
        unsafe { buffer.contents.add(offset as usize) }
    }

    // ------------------------------------------------------------------
    // Declarations and shaders
    // ------------------------------------------------------------------

    pub fn create_vertex_declaration(
        &mut self,
        elements: &[VertexElement],
    ) -> Box<VertexDeclaration> {
        Box::new(VertexDeclaration {
            id: self.unique_id(),
            elements: elements.to_vec(),
        })
    }

    pub fn create_shader(
        &mut self,
        stage: i32,
        bytecode: &[u32],
        name: &str,
    ) -> Option<Box<Shader>> {
        let options = translate::Options::default();
        let translated = match translate::translate(bytecode, &options) {
            Ok(translated) => translated,
            Err(error) => {
                log(&format!("shader '{name}' could not be translated: {error}"));
                return None;
            }
        };
        let expected = if stage == state::SHADER_VERTEX {
            Stage::Vertex
        } else {
            Stage::Pixel
        };
        if translated.stage != expected {
            log(&format!(
                "shader '{name}' is not the stage it was created as"
            ));
            return None;
        }
        Some(Box::new(Shader {
            id: self.unique_id(),
            name: name.to_owned(),
            bytecode: bytecode.to_vec(),
            translated,
            variants: HashMap::new(),
        }))
    }

    /// Compiles a shader's entry point the first time a draw needs it. Most
    /// of what the engine creates is never drawn with, and compiling is by
    /// far the slowest thing the device does.
    fn shader_function(
        &mut self,
        handle: Handle,
        alpha_test: bool,
        shadow_samplers: u32,
    ) -> Option<Id> {
        // SAFETY: as for textures.
        let shader = unsafe { (handle as *mut Shader).as_mut() }?;
        let slot = usize::from(alpha_test);

        let Shader {
            name,
            bytecode,
            translated: plain,
            variants,
            ..
        } = shader;
        let variant = variants.entry(shadow_samplers).or_insert_with(|| {
            let translated = if shadow_samplers == 0 {
                Ok(plain.clone())
            } else {
                translate::translate(bytecode, &translate::Options { shadow_samplers })
            };
            match translated {
                Ok(translated) => Variant { translated, library: None, functions: [None, None], failed: false },
                Err(error) => {
                    log(&format!("shader '{name}' could not be translated with shadow samplers {shadow_samplers:#x}: {error}"));
                    Variant { translated: plain.clone(), library: None, functions: [None, None], failed: true }
                }
            }
        });
        if let Some(function) = &variant.functions[slot] {
            return Some(function.id());
        }
        if variant.failed {
            return None;
        }

        let vertex = variant.translated.stage == Stage::Vertex;
        if variant.library.is_none() {
            // A vertex function's position has to come out bit-identical in
            // every pass that draws the same geometry, or passes that expect
            // to land on each other's depth do not.
            let source = if vertex {
                variant
                    .translated
                    .msl
                    .replace("[[position]]", "[[position, invariant]]")
            } else {
                variant.translated.msl.clone()
            };
            let source = ns_string(&source)?;
            // SAFETY: options are owned for the block, the error is an
            // autoreleased out-parameter read before the pool drains, and the
            // library follows the `new` rule.
            unsafe {
                let options = Owned::from_owned(
                    msg![ret: Id; msg![ret: Id; class(c"MTLCompileOptions"), "alloc"], "init"],
                )?;
                msg![options.id(), "setPreserveInvariance:", true => bool];
                let mut error: Id = std::ptr::null_mut();
                let library = msg![ret: Id; self.mtl.id(), "newLibraryWithSource:options:error:",
                    source.id() => Id, options.id() => Id, &mut error => *mut Id];
                match Owned::from_owned(library) {
                    Some(library) => variant.library = Some(library),
                    None => {
                        log(&format!(
                            "shader '{name}' did not compile: {}",
                            error_text(error)
                        ));
                        variant.failed = true;
                        return None;
                    }
                }
            }
        }

        let library = variant.library.as_ref()?.id();
        let entry = ns_string(if vertex { "vs_main" } else { "ps_main" })?;
        // SAFETY: as above; the constant's value is read during the call.
        let function = unsafe {
            let values = Owned::from_owned(
                msg![ret: Id; msg![ret: Id; class(c"MTLFunctionConstantValues"), "alloc"], "init"],
            )?;
            msg![values.id(), "setConstantValue:type:atIndex:",
                std::ptr::from_ref(&alpha_test).cast::<c_void>() => *const c_void,
                DATA_TYPE_BOOL => NSUInteger, 0 => NSUInteger];
            let mut error: Id = std::ptr::null_mut();
            let function = msg![ret: Id; library, "newFunctionWithName:constantValues:error:",
                entry.id() => Id, values.id() => Id, &mut error => *mut Id];
            match Owned::from_owned(function) {
                Some(function) => function,
                None => {
                    log(&format!(
                        "shader '{name}' has no entry point: {}",
                        error_text(error)
                    ));
                    variant.failed = true;
                    return None;
                }
            }
        };
        let id = function.id();
        variant.functions[slot] = Some(function);
        Some(id)
    }

    // ------------------------------------------------------------------
    // Pipelines and fixed state objects
    // ------------------------------------------------------------------

    fn blend_key(state: &State) -> [u32; 7] {
        let r = &state.render_states;
        if r[rs::ALPHABLENDENABLE] == 0 {
            return [0; 7];
        }
        let (src_alpha, dst_alpha, op_alpha) = if r[rs::SEPARATEALPHABLENDENABLE] != 0 {
            (
                r[rs::SRCBLENDALPHA],
                r[rs::DESTBLENDALPHA],
                r[rs::BLENDOPALPHA],
            )
        } else {
            (r[rs::SRCBLEND], r[rs::DESTBLEND], r[rs::BLENDOP])
        };
        [
            1,
            r[rs::SRCBLEND],
            r[rs::DESTBLEND],
            r[rs::BLENDOP],
            src_alpha,
            dst_alpha,
            op_alpha,
        ]
    }

    fn pipeline(&mut self, state: &State, alpha_test: bool, shadow_samplers: u32) -> Option<Id> {
        // SAFETY: as for textures.
        let (vs, ps, decl) = unsafe {
            (
                (state.vertex_shader as *const Shader).as_ref()?,
                (state.pixel_shader as *const Shader).as_ref()?,
                (state.vertex_declaration as *const VertexDeclaration).as_ref()?,
            )
        };
        let pass = self.pass.as_ref()?;

        let mut strides = [0u32; state::STREAMS];
        for element in &decl.elements {
            if let Some(stream) = state.streams.get(usize::from(element.stream)) {
                strides[usize::from(element.stream)] = if stream.buffer != 0 {
                    stream.stride
                } else {
                    u32::MAX
                };
            }
        }

        let key = PipelineKey {
            vs: vs.id,
            ps: ps.id,
            shadow_samplers,
            alpha_test,
            decl: decl.id,
            strides,
            color_format: pass.color_format,
            depth_format: pass.depth_format,
            stencil: pass.stencil,
            blend: Self::blend_key(state),
            write_mask: state.render_states[rs::COLORWRITEENABLE] & 0xF,
        };
        if let Some(found) = self.pipelines.get(&key) {
            return found.as_ref().map(Owned::id);
        }

        let built = self.build_pipeline(state, &key);
        let id = built.as_ref().map(Owned::id);
        self.pipelines.insert(key, built);
        id
    }

    fn build_pipeline(&mut self, state: &State, key: &PipelineKey) -> Option<Owned> {
        let vertex_function = self.shader_function(state.vertex_shader, false, 0)?;
        let fragment_function =
            self.shader_function(state.pixel_shader, key.alpha_test, key.shadow_samplers)?;
        // SAFETY: as for textures; both were checked by the caller.
        let (vs, ps, decl) = unsafe {
            (
                &*(state.vertex_shader as *const Shader),
                &*(state.pixel_shader as *const Shader),
                &*(state.vertex_declaration as *const VertexDeclaration),
            )
        };

        // SAFETY: descriptors are owned or autoreleased within the caller's
        // pool, every selector is the descriptor's own, and the pipeline
        // follows the `new` rule.
        unsafe {
            let vertex_descriptor =
                msg![ret: Id; class(c"MTLVertexDescriptor"), "vertexDescriptor"];
            let attributes = msg![ret: Id; vertex_descriptor, "attributes"];
            let layouts = msg![ret: Id; vertex_descriptor, "layouts"];
            let mut missing = false;
            let mut used_streams = [false; state::STREAMS];

            for input in &vs.translated.inputs {
                let attribute = msg![ret: Id; attributes, "objectAtIndexedSubscript:", NSUInteger::from(input.register) => NSUInteger];
                let element = decl.elements.iter().find(|e| {
                    e.usage == input.usage
                        && e.usage_index == input.usage_index
                        && vertex_format(e.kind) != VERTEX_FORMAT_INVALID
                        && key
                            .strides
                            .get(usize::from(e.stream))
                            .is_some_and(|s| *s != u32::MAX)
                });
                match element {
                    Some(element) => {
                        msg![attribute, "setFormat:", vertex_format(element.kind) => NSUInteger];
                        msg![attribute, "setOffset:", NSUInteger::from(element.offset) => NSUInteger];
                        msg![attribute, "setBufferIndex:", NSUInteger::from(element.stream) => NSUInteger];
                        used_streams[usize::from(element.stream)] = true;
                    }
                    None => {
                        msg![attribute, "setFormat:", VERTEX_FORMAT_FLOAT4 => NSUInteger];
                        msg![attribute, "setOffset:", 0 => NSUInteger];
                        msg![attribute, "setBufferIndex:", SLOT_MISSING_STREAM => NSUInteger];
                        missing = true;
                    }
                }
            }

            for (stream, used) in used_streams.iter().enumerate() {
                if !used {
                    continue;
                }
                let layout = msg![ret: Id; layouts, "objectAtIndexedSubscript:", stream as NSUInteger => NSUInteger];
                let stride = key.strides[stream];
                if stride == 0 {
                    // Direct3D's zero stride reads one vertex for the whole draw.
                    msg![layout, "setStepFunction:", VERTEX_STEP_CONSTANT => NSUInteger];
                    msg![layout, "setStepRate:", 0 => NSUInteger];
                    msg![layout, "setStride:", 4 => NSUInteger];
                } else {
                    msg![layout, "setStepFunction:", VERTEX_STEP_PER_VERTEX => NSUInteger];
                    msg![layout, "setStepRate:", 1 => NSUInteger];
                    msg![layout, "setStride:", NSUInteger::from(stride) => NSUInteger];
                }
            }
            if missing {
                let layout = msg![ret: Id; layouts, "objectAtIndexedSubscript:", SLOT_MISSING_STREAM => NSUInteger];
                msg![layout, "setStepFunction:", VERTEX_STEP_CONSTANT => NSUInteger];
                msg![layout, "setStepRate:", 0 => NSUInteger];
                msg![layout, "setStride:", 16 => NSUInteger];
            }

            let desc = Owned::from_owned(
                msg![ret: Id; msg![ret: Id; class(c"MTLRenderPipelineDescriptor"), "alloc"], "init"],
            )?;
            msg![desc.id(), "setVertexFunction:", vertex_function => Id];
            msg![desc.id(), "setFragmentFunction:", fragment_function => Id];
            msg![desc.id(), "setVertexDescriptor:", vertex_descriptor => Id];
            msg![desc.id(), "setDepthAttachmentPixelFormat:", key.depth_format => NSUInteger];
            if key.stencil {
                msg![desc.id(), "setStencilAttachmentPixelFormat:", key.depth_format => NSUInteger];
            }

            let attachments = msg![ret: Id; desc.id(), "colorAttachments"];
            let attachment =
                msg![ret: Id; attachments, "objectAtIndexedSubscript:", 0 => NSUInteger];
            msg![attachment, "setPixelFormat:", key.color_format => NSUInteger];
            msg![attachment, "setWriteMask:", color_write_mask(key.write_mask) => NSUInteger];
            if key.blend[0] != 0 {
                msg![attachment, "setBlendingEnabled:", true => bool];
                msg![attachment, "setSourceRGBBlendFactor:", blend_factor(key.blend[1]) => NSUInteger];
                msg![attachment, "setDestinationRGBBlendFactor:", blend_factor(key.blend[2]) => NSUInteger];
                msg![attachment, "setRgbBlendOperation:", blend_op(key.blend[3]) => NSUInteger];
                msg![attachment, "setSourceAlphaBlendFactor:", blend_factor(key.blend[4]) => NSUInteger];
                msg![attachment, "setDestinationAlphaBlendFactor:", blend_factor(key.blend[5]) => NSUInteger];
                msg![attachment, "setAlphaBlendOperation:", blend_op(key.blend[6]) => NSUInteger];
            }

            let mut error: Id = std::ptr::null_mut();
            let pipeline = msg![ret: Id; self.mtl.id(), "newRenderPipelineStateWithDescriptor:error:",
                desc.id() => Id, &mut error => *mut Id];
            let pipeline = Owned::from_owned(pipeline);
            if pipeline.is_none() {
                log(&format!(
                    "pipeline for '{}' + '{}' failed: {}",
                    vs.name,
                    ps.name,
                    error_text(error)
                ));
            }
            pipeline
        }
    }

    fn utility_pipeline(&mut self, fragment: &'static str, write_color: bool) -> Option<Id> {
        let pass = self.pass.as_ref()?;
        let key = UtilityKey {
            fragment,
            color_format: pass.color_format,
            depth_format: pass.depth_format,
            stencil: pass.stencil,
            write_color,
        };
        if let Some(found) = self.utility_pipelines.get(&key) {
            return found.as_ref().map(Owned::id);
        }

        if self.utility_library.is_none() {
            let source = ns_string(UTILITY_MSL)?;
            // SAFETY: as in `shader_function`.
            unsafe {
                let mut error: Id = std::ptr::null_mut();
                let library = msg![ret: Id; self.mtl.id(), "newLibraryWithSource:options:error:",
                    source.id() => Id, std::ptr::null_mut::<c_void>() => Id, &mut error => *mut Id];
                self.utility_library = Owned::from_owned(library);
                if self.utility_library.is_none() {
                    log(&format!(
                        "the device's own shaders did not compile: {}",
                        error_text(error)
                    ));
                }
            }
        }
        let library = self.utility_library.as_ref()?.id();

        // SAFETY: as in `build_pipeline`.
        let built = unsafe {
            let vertex_name = ns_string("quad_vs")?;
            let fragment_name = ns_string(fragment)?;
            let vertex_function = Owned::from_owned(
                msg![ret: Id; library, "newFunctionWithName:", vertex_name.id() => Id],
            )?;
            let fragment_function = Owned::from_owned(
                msg![ret: Id; library, "newFunctionWithName:", fragment_name.id() => Id],
            )?;

            let desc = Owned::from_owned(
                msg![ret: Id; msg![ret: Id; class(c"MTLRenderPipelineDescriptor"), "alloc"], "init"],
            )?;
            msg![desc.id(), "setVertexFunction:", vertex_function.id() => Id];
            msg![desc.id(), "setFragmentFunction:", fragment_function.id() => Id];
            msg![desc.id(), "setDepthAttachmentPixelFormat:", key.depth_format => NSUInteger];
            if key.stencil {
                msg![desc.id(), "setStencilAttachmentPixelFormat:", key.depth_format => NSUInteger];
            }
            let attachments = msg![ret: Id; desc.id(), "colorAttachments"];
            let attachment =
                msg![ret: Id; attachments, "objectAtIndexedSubscript:", 0 => NSUInteger];
            msg![attachment, "setPixelFormat:", key.color_format => NSUInteger];
            msg![attachment, "setWriteMask:", (if write_color { 0xF } else { 0 }) => NSUInteger];

            let mut error: Id = std::ptr::null_mut();
            let pipeline = Owned::from_owned(
                msg![ret: Id; self.mtl.id(), "newRenderPipelineStateWithDescriptor:error:",
                desc.id() => Id, &mut error => *mut Id],
            );
            if pipeline.is_none() {
                log(&format!(
                    "utility pipeline '{fragment}' failed: {}",
                    error_text(error)
                ));
            }
            pipeline
        };
        let id = built.as_ref().map(Owned::id);
        self.utility_pipelines.insert(key, built);
        id
    }

    fn depth_state(&mut self, key: DepthKey) -> Option<Id> {
        if let Some(found) = self.depth_states.get(&key) {
            return Some(found.id());
        }
        // SAFETY: descriptors are owned for the block and the state object
        // follows the `new` rule.
        let built = unsafe {
            let desc = Owned::from_owned(
                msg![ret: Id; msg![ret: Id; class(c"MTLDepthStencilDescriptor"), "alloc"], "init"],
            )?;
            let [enable, write, func] = key.depth;
            msg![desc.id(), "setDepthCompareFunction:", (if enable != 0 { compare(func) } else { COMPARE_ALWAYS }) => NSUInteger];
            msg![desc.id(), "setDepthWriteEnabled:", (enable != 0 && write != 0) => bool];

            if key.stencil[0] != 0 {
                let [_, func, fail, zfail, pass, read, write, two_sided, ccw_func, ccw_fail, ccw_zfail, ccw_pass] =
                    key.stencil;
                let face = |func: u32, fail: u32, zfail: u32, pass: u32| -> Option<Owned> {
                    let face = Owned::from_owned(
                        msg![ret: Id; msg![ret: Id; class(c"MTLStencilDescriptor"), "alloc"], "init"],
                    )?;
                    msg![face.id(), "setStencilCompareFunction:", compare(func) => NSUInteger];
                    msg![face.id(), "setStencilFailureOperation:", stencil_op(fail) => NSUInteger];
                    msg![face.id(), "setDepthFailureOperation:", stencil_op(zfail) => NSUInteger];
                    msg![face.id(), "setDepthStencilPassOperation:", stencil_op(pass) => NSUInteger];
                    msg![face.id(), "setReadMask:", read => u32];
                    msg![face.id(), "setWriteMask:", write => u32];
                    Some(face)
                };
                // Clockwise is the front face, so the CCW set is the back's.
                let front = face(func, fail, zfail, pass)?;
                let back = if two_sided != 0 {
                    face(ccw_func, ccw_fail, ccw_zfail, ccw_pass)?
                } else {
                    face(func, fail, zfail, pass)?
                };
                msg![desc.id(), "setFrontFaceStencil:", front.id() => Id];
                msg![desc.id(), "setBackFaceStencil:", back.id() => Id];
            }
            Owned::from_owned(
                msg![ret: Id; self.mtl.id(), "newDepthStencilStateWithDescriptor:", desc.id() => Id],
            )?
        };
        let id = built.id();
        self.depth_states.insert(key, built);
        Some(id)
    }

    fn sampler(&mut self, key: SamplerKey) -> Option<Id> {
        if let Some(found) = self.samplers.get(&key) {
            return Some(found.id());
        }
        // SAFETY: as in `depth_state`.
        let built = unsafe {
            let desc = Owned::from_owned(
                msg![ret: Id; msg![ret: Id; class(c"MTLSamplerDescriptor"), "alloc"], "init"],
            )?;
            let [mag, min, mip] = key.filter;
            let filter = |d3d: u32| {
                if d3d >= 2 {
                    SAMPLER_FILTER_LINEAR
                } else {
                    SAMPLER_FILTER_NEAREST
                }
            };
            msg![desc.id(), "setMagFilter:", filter(mag) => NSUInteger];
            msg![desc.id(), "setMinFilter:", filter(min) => NSUInteger];
            msg![desc.id(), "setMipFilter:", (match mip {
                0 => SAMPLER_MIP_NONE,
                1 => SAMPLER_MIP_NEAREST,
                _ => SAMPLER_MIP_LINEAR,
            }) => NSUInteger];
            msg![desc.id(), "setSAddressMode:", address_mode(key.address[0]) => NSUInteger];
            msg![desc.id(), "setTAddressMode:", address_mode(key.address[1]) => NSUInteger];
            msg![desc.id(), "setRAddressMode:", address_mode(key.address[2]) => NSUInteger];
            msg![desc.id(), "setMaxAnisotropy:", NSUInteger::from(key.anisotropy.clamp(1, 16)) => NSUInteger];
            msg![desc.id(), "setLodMinClamp:", key.max_mip as f32 => f32];
            if key.compare {
                // Lit where the reference depth is no farther than the map's.
                msg![desc.id(), "setCompareFunction:", COMPARE_LESS_EQUAL => NSUInteger];
            }
            Owned::from_owned(
                msg![ret: Id; self.mtl.id(), "newSamplerStateWithDescriptor:", desc.id() => Id],
            )?
        };
        let id = built.id();
        self.samplers.insert(key, built);
        Some(id)
    }

    // ------------------------------------------------------------------
    // Drawing
    // ------------------------------------------------------------------

    fn ring_alloc(&mut self, length: usize) -> Option<(Id, usize, *mut u8)> {
        let length = length.next_multiple_of(64);
        let exhausted = self.ring.current.is_none() || self.ring.offset + length > self.ring.chunk;
        if exhausted {
            if let Some(full) = self.ring.current.take() {
                self.ring.used.push(full);
            }
            let frame = self.frame;
            let reusable = self
                .ring
                .free
                .iter()
                .position(|b| frame.saturating_sub(b.retired) >= FRAMES_IN_FLIGHT);
            let next = match reusable {
                Some(index) => self.ring.free.swap_remove(index),
                None => {
                    let (buf, contents) =
                        self.new_metal_buffer(self.ring.chunk.max(length) as u32)?;
                    RingBuffer {
                        buf,
                        contents,
                        retired: 0,
                    }
                }
            };
            self.ring.current = Some(next);
            self.ring.offset = 0;
        }
        let current = self.ring.current.as_ref()?;
        let offset = self.ring.offset;
        self.ring.offset += length;
        // SAFETY: `offset + length` is within the chunk, checked above.
        Some((current.buf.id(), offset, unsafe {
            current.contents.add(offset)
        }))
    }

    fn ring_end_frame(&mut self) {
        let frame = self.frame;
        if let Some(mut current) = self.ring.current.take() {
            current.retired = frame;
            self.ring.free.push(current);
        }
        for mut used in self.ring.used.drain(..) {
            used.retired = frame;
            self.ring.free.push(used);
        }
        self.ring.offset = 0;
    }

    /// Binds everything a draw reads. Returns false when the draw cannot be
    /// made, which is always a state the log has already named.
    fn prepare_draw(&mut self, state: &State) -> bool {
        if state.vertex_shader == 0 || state.pixel_shader == 0 || state.vertex_declaration == 0 {
            return false;
        }
        if !self.ensure_pass(state) {
            return false;
        }

        let r = &state.render_states;
        let alpha_test = r[rs::ALPHATESTENABLE] != 0;

        // The samplers this draw reads as shadow maps: the engine says so with
        // a sampler state, and it only means anything over a depth texture.
        let mut shadow_samplers = 0u32;
        // SAFETY: as for textures; checked non-null above.
        let ps_samplers = unsafe { &(*(state.pixel_shader as *const Shader)).translated.samplers };
        for (stage, kind) in ps_samplers.iter().enumerate() {
            if *kind == SamplerKind::D2
                && state.sampler_states[stage][samp::SHADOWFILTER] != 0
                && Self::texture(state.textures[stage]).is_some_and(|t| t.info.depth)
            {
                shadow_samplers |= 1 << stage;
            }
        }

        let Some(pipeline) = self.pipeline(state, alpha_test, shadow_samplers) else {
            return false;
        };

        let has_depth = self
            .pass
            .as_ref()
            .is_some_and(|p| p.depth_format != format::mtlfmt::INVALID);
        let has_stencil = self.pass.as_ref().is_some_and(|p| p.stencil);
        let depth_key = DepthKey {
            depth: if has_depth {
                [r[rs::ZENABLE], r[rs::ZWRITEENABLE], r[rs::ZFUNC]]
            } else {
                [0; 3]
            },
            stencil: if has_stencil && r[rs::STENCILENABLE] != 0 {
                [
                    1,
                    r[rs::STENCILFUNC],
                    r[rs::STENCILFAIL],
                    r[rs::STENCILZFAIL],
                    r[rs::STENCILPASS],
                    r[rs::STENCILMASK],
                    r[rs::STENCILWRITEMASK],
                    r[rs::TWOSIDEDSTENCILMODE],
                    r[rs::CCW_STENCILFUNC],
                    r[rs::CCW_STENCILFAIL],
                    r[rs::CCW_STENCILZFAIL],
                    r[rs::CCW_STENCILPASS],
                ]
            } else {
                [0; 12]
            },
        };
        let Some(depth_state) = self.depth_state(depth_key) else {
            return false;
        };

        // SAFETY: as for textures; `pipeline` has just dereferenced both.
        let (vs, ps) = unsafe {
            (
                &*(state.vertex_shader as *const Shader),
                &*(state.pixel_shader as *const Shader),
            )
        };

        // Samplers and textures, resolved before the encoder is borrowed.
        let mut texture_binds: [(Id, Id); state::SAMPLERS] =
            [(std::ptr::null_mut(), std::ptr::null_mut()); state::SAMPLERS];
        for (stage, kind) in ps.translated.samplers.iter().enumerate() {
            if *kind == SamplerKind::None {
                continue;
            }
            let ss = &state.sampler_states[stage];
            let wanted = match kind {
                SamplerKind::Cube => TextureKind::Cube,
                SamplerKind::Volume => TextureKind::Volume,
                _ => TextureKind::D2,
            };
            let compare = shadow_samplers & (1 << stage) != 0;
            // A depth texture under a plain sampler would be a type error in
            // Metal, so it reads as nothing bound.
            let bound = Self::texture(state.textures[stage])
                .filter(|t| t.kind == wanted && (compare || !t.info.depth));
            let view = match bound {
                Some(texture) => texture.view(ss[samp::SRGBTEXTURE] != 0),
                None => match wanted {
                    TextureKind::Cube => self.dummy_cube.as_ref(),
                    TextureKind::Volume => self.dummy_3d.as_ref(),
                    TextureKind::D2 => self.dummy_2d.as_ref(),
                }
                .map_or(std::ptr::null_mut(), Owned::id),
            };
            let anisotropic = ss[samp::MINFILTER] == 3 || ss[samp::MAGFILTER] == 3;
            let key = SamplerKey {
                address: [ss[samp::ADDRESSU], ss[samp::ADDRESSV], ss[samp::ADDRESSW]],
                filter: [
                    ss[samp::MAGFILTER],
                    ss[samp::MINFILTER],
                    ss[samp::MIPFILTER],
                ],
                anisotropy: if anisotropic {
                    ss[samp::MAXANISOTROPY].max(1)
                } else {
                    1
                },
                max_mip: ss[samp::MAXMIPLEVEL],
                compare,
            };
            let Some(sampler) = self.sampler(key) else {
                return false;
            };
            texture_binds[stage] = (view, sampler);
        }

        // Constants.
        let vs_count = (vs.translated.float_constants as usize).clamp(1, state::VS_FLOAT_CONSTANTS);
        let ps_count = (ps.translated.float_constants as usize).clamp(1, state::PS_FLOAT_CONSTANTS);
        let extra_size = std::mem::size_of::<Extra>();
        let Some((vs_buf, vs_off, vs_ptr)) = self.ring_alloc(vs_count * 16) else {
            return false;
        };
        let Some((ps_buf, ps_off, ps_ptr)) = self.ring_alloc(ps_count * 16) else {
            return false;
        };
        let Some((vx_buf, vx_off, vx_ptr)) = self.ring_alloc(extra_size) else {
            return false;
        };
        let Some((px_buf, px_off, px_ptr)) = self.ring_alloc(extra_size) else {
            return false;
        };

        let viewport = state.viewport;
        let mut extra = Extra {
            ints: state.vs_ints,
            clip_plane0: if r[rs::CLIPPLANEENABLE] & 1 != 0 {
                state.clip_plane0
            } else {
                [0.0; 4]
            },
            pos_fixup: [
                1.0 / viewport.width.max(1) as f32,
                -1.0 / viewport.height.max(1) as f32,
                0.0,
                0.0,
            ],
            bools: state.vs_bools,
            alpha_func: r[rs::ALPHAFUNC],
            alpha_ref: (r[rs::ALPHAREF] & 0xFF) as f32 / 255.0,
            flags: 0,
        };
        // SAFETY: each destination was allocated with at least the length
        // copied, and the sources are plain arrays inside the state block.
        unsafe {
            std::ptr::copy_nonoverlapping(
                state.vs_floats.as_ptr().cast::<u8>(),
                vs_ptr,
                vs_count * 16,
            );
            std::ptr::copy_nonoverlapping(
                state.ps_floats.as_ptr().cast::<u8>(),
                ps_ptr,
                ps_count * 16,
            );
            std::ptr::copy_nonoverlapping(
                std::ptr::from_ref(&extra).cast::<u8>(),
                vx_ptr,
                extra_size,
            );
            extra.ints = state.ps_ints;
            extra.bools = state.ps_bools;
            std::ptr::copy_nonoverlapping(
                std::ptr::from_ref(&extra).cast::<u8>(),
                px_ptr,
                extra_size,
            );
        }

        // Streams the declaration draws from.
        // SAFETY: as for textures.
        let decl = unsafe { &*(state.vertex_declaration as *const VertexDeclaration) };
        let mut stream_binds: [(Id, u64); state::STREAMS] =
            [(std::ptr::null_mut(), 0); state::STREAMS];
        let frame = self.frame;
        for element in &decl.elements {
            let index = usize::from(element.stream);
            let Some(stream) = state.streams.get(index) else {
                continue;
            };
            // SAFETY: as for textures.
            if let Some(buffer) = unsafe { (stream.buffer as *mut Buffer).as_mut() } {
                buffer.last_used = frame;
                stream_binds[index] = (buffer.buf.id(), u64::from(stream.offset));
            }
        }
        let missing_stream = self
            .missing_stream
            .as_ref()
            .map_or(std::ptr::null_mut(), Owned::id);

        let cull = match r[rs::CULLMODE] {
            2 => CULL_FRONT, // D3DCULL_CW culls the clockwise, which is the front
            3 => CULL_BACK,  // D3DCULL_CCW
            _ => CULL_NONE,
        };
        let fill = if r[rs::FILLMODE] == 2 {
            FILL_MODE_LINES
        } else {
            FILL_MODE_FILL
        };

        let start_counting = match &mut self.counting {
            Some((slot, applied)) if !*applied => {
                *applied = true;
                Some(*slot)
            }
            _ => None,
        };

        let Some(pass) = self.pass.as_mut() else {
            return false;
        };
        let encoder = pass.encoder.id();
        let bound = &mut pass.bound;
        if let Some(slot) = start_counting {
            // SAFETY: the encoder is open and the offset is a whole slot
            // inside the visibility buffer its pass was given.
            unsafe {
                msg![encoder, "setVisibilityResultMode:offset:",
                    VISIBILITY_COUNTING => NSUInteger, (slot * 8) as NSUInteger => NSUInteger];
            }
        }

        // Clamp to the target: Metal faults a scissor that leaves it.
        let (tw, th) = (pass.width, pass.height);
        let scissor = if r[rs::SCISSORTESTENABLE] != 0 {
            let left = (state.scissor.left.max(0) as u32).min(tw);
            let top = (state.scissor.top.max(0) as u32).min(th);
            let right = (state.scissor.right.max(0) as u32).clamp(left, tw);
            let bottom = (state.scissor.bottom.max(0) as u32).clamp(top, th);
            MTLScissorRect {
                x: left.into(),
                y: top.into(),
                width: (right - left).into(),
                height: (bottom - top).into(),
            }
        } else {
            MTLScissorRect {
                x: 0,
                y: 0,
                width: tw.into(),
                height: th.into(),
            }
        };
        let mtl_viewport = MTLViewport {
            origin_x: f64::from(viewport.x),
            origin_y: f64::from(viewport.y),
            width: f64::from(viewport.width),
            height: f64::from(viewport.height),
            znear: f64::from(viewport.min_z),
            zfar: f64::from(viewport.max_z),
        };

        // SAFETY: the encoder is open, every object bound is live for at
        // least the command buffer, and the by-value structures match the
        // framework's layout.
        unsafe {
            if bound.pipeline != Id0(pipeline) {
                msg![encoder, "setRenderPipelineState:", pipeline => Id];
                bound.pipeline = Id0(pipeline);
            }
            if bound.depth_state != Id0(depth_state) {
                msg![encoder, "setDepthStencilState:", depth_state => Id];
                bound.depth_state = Id0(depth_state);
            }
            let stencil_ref = r[rs::STENCILREF];
            if bound.stencil_ref != Some(stencil_ref) {
                msg![encoder, "setStencilReferenceValue:", stencil_ref => u32];
                bound.stencil_ref = Some(stencil_ref);
            }
            if bound.cull != Some(cull) {
                msg![encoder, "setCullMode:", cull => NSUInteger];
                bound.cull = Some(cull);
            }
            if bound.fill != Some(fill) {
                msg![encoder, "setTriangleFillMode:", fill => NSUInteger];
                bound.fill = Some(fill);
            }
            let bias = (r[rs::DEPTHBIAS], r[rs::SLOPESCALEDEPTHBIAS]);
            if bound.bias != Some(bias) {
                // Direct3D states the bias in depth units and Metal in
                // multiples of the smallest step the depth format resolves.
                let scale = if pass.depth_format == format::mtlfmt::DEPTH16_UNORM {
                    65_536.0
                } else {
                    16_777_216.0
                };
                msg![encoder, "setDepthBias:slopeScale:clamp:",
                    f32::from_bits(bias.0) * scale => f32, f32::from_bits(bias.1) => f32, 0.0 => f32];
                bound.bias = Some(bias);
            }
            if bound.viewport != Some(mtl_viewport) {
                msg![encoder, "setViewport:", mtl_viewport => MTLViewport];
                bound.viewport = Some(mtl_viewport);
            }
            if bound.scissor != Some(scissor) {
                msg![encoder, "setScissorRect:", scissor => MTLScissorRect];
                bound.scissor = Some(scissor);
            }

            for (index, (buffer, offset)) in stream_binds.iter().enumerate() {
                if buffer.is_null() {
                    continue;
                }
                if bound.vertex_buffers[index] != (Id0(*buffer), *offset) {
                    if bound.vertex_buffers[index].0 == Id0(*buffer) {
                        msg![encoder, "setVertexBufferOffset:atIndex:", *offset => NSUInteger, index as NSUInteger => NSUInteger];
                    } else {
                        msg![encoder, "setVertexBuffer:offset:atIndex:", *buffer => Id, *offset => NSUInteger, index as NSUInteger => NSUInteger];
                    }
                    bound.vertex_buffers[index] = (Id0(*buffer), *offset);
                }
            }
            let missing_slot = SLOT_MISSING_STREAM as usize;
            if bound.vertex_buffers[missing_slot].0 != Id0(missing_stream)
                && !missing_stream.is_null()
            {
                msg![encoder, "setVertexBuffer:offset:atIndex:", missing_stream => Id, 0 => NSUInteger, SLOT_MISSING_STREAM => NSUInteger];
                bound.vertex_buffers[missing_slot] = (Id0(missing_stream), 0);
            }

            msg![encoder, "setVertexBuffer:offset:atIndex:", vs_buf => Id, vs_off as NSUInteger => NSUInteger, SLOT_VS_FLOATS => NSUInteger];
            msg![encoder, "setVertexBuffer:offset:atIndex:", vx_buf => Id, vx_off as NSUInteger => NSUInteger, SLOT_VS_EXTRA => NSUInteger];
            msg![encoder, "setFragmentBuffer:offset:atIndex:", ps_buf => Id, ps_off as NSUInteger => NSUInteger, SLOT_PS_FLOATS => NSUInteger];
            msg![encoder, "setFragmentBuffer:offset:atIndex:", px_buf => Id, px_off as NSUInteger => NSUInteger, SLOT_PS_EXTRA => NSUInteger];

            for (stage, (view, sampler)) in texture_binds.iter().enumerate() {
                if view.is_null() {
                    continue;
                }
                if bound.textures[stage] != Id0(*view) {
                    msg![encoder, "setFragmentTexture:atIndex:", *view => Id, stage as NSUInteger => NSUInteger];
                    bound.textures[stage] = Id0(*view);
                }
                if bound.samplers[stage] != Id0(*sampler) {
                    msg![encoder, "setFragmentSamplerState:atIndex:", *sampler => Id, stage as NSUInteger => NSUInteger];
                    bound.samplers[stage] = Id0(*sampler);
                }
            }
        }
        true
    }

    pub fn draw(
        &mut self,
        state: &State,
        primitive_type: u32,
        start_vertex: u32,
        primitive_count: u32,
    ) {
        let _pool = AutoreleasePool::new();
        let Some((kind, count)) = primitive(primitive_type, primitive_count) else {
            self.warn_once(format!("primitive type {primitive_type} is not drawn"));
            return;
        };
        if count == 0 || !self.prepare_draw(state) {
            return;
        }
        let Some(pass) = &self.pass else { return };
        self.stats_draws += 1;
        // SAFETY: the encoder is open and fully bound.
        unsafe {
            msg![pass.encoder.id(), "drawPrimitives:vertexStart:vertexCount:",
                kind => NSUInteger, NSUInteger::from(start_vertex) => NSUInteger, count => NSUInteger];
        }
    }

    pub fn draw_indexed(
        &mut self,
        state: &State,
        primitive_type: u32,
        base_vertex: i32,
        start_index: u32,
        primitive_count: u32,
    ) {
        let _pool = AutoreleasePool::new();
        let Some((kind, count)) = primitive(primitive_type, primitive_count) else {
            self.warn_once(format!(
                "indexed primitive type {primitive_type} is not drawn"
            ));
            return;
        };
        // SAFETY: as for textures.
        let Some(indices) = (unsafe { (state.index_buffer as *mut Buffer).as_mut() }) else {
            return;
        };
        let index_size = u64::from(indices.index_size.max(2));
        let first = u64::from(start_index) * index_size;
        if count == 0 || first + count * index_size > u64::from(indices.size) {
            return;
        }
        indices.last_used = self.frame;
        let index_buffer = indices.buf.id();
        let index_type = if index_size == 4 {
            INDEX_TYPE_UINT32
        } else {
            INDEX_TYPE_UINT16
        };

        if !self.prepare_draw(state) {
            return;
        }
        let Some(pass) = &self.pass else { return };
        self.stats_draws += 1;
        // SAFETY: the encoder is open and fully bound, and the index range
        // was checked against the buffer.
        unsafe {
            msg![pass.encoder.id(),
                "drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:instanceCount:baseVertex:baseInstance:",
                kind => NSUInteger, count => NSUInteger, index_type => NSUInteger, index_buffer => Id,
                first => NSUInteger, 1 => NSUInteger, i64::from(base_vertex) => i64, 0 => NSUInteger];
        }
    }

    // ------------------------------------------------------------------
    // Clear, copy, present
    // ------------------------------------------------------------------

    /// What a stored byte has to be given as for an sRGB attachment to store
    /// it back unchanged. Direct3D's clear ignores sRGB write; Metal's does
    /// not, so the colour is decoded here to be encoded again there.
    fn srgb_to_linear(value: f64) -> f64 {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn clear(
        &mut self,
        state: &State,
        rects: &[Rect],
        flags: u32,
        color: u32,
        z: f32,
        stencil: u32,
    ) {
        let _pool = AutoreleasePool::new();
        let key = Self::pass_key(state);
        let Some(target) = Self::texture(key.color.texture) else {
            return;
        };
        let (tw, th, _) = target.level_size(key.color.level);

        let mut rgba = [
            f64::from((color >> 16) & 0xFF) / 255.0,
            f64::from((color >> 8) & 0xFF) / 255.0,
            f64::from(color & 0xFF) / 255.0,
            f64::from((color >> 24) & 0xFF) / 255.0,
        ];
        if target.info.opaque_alpha {
            rgba[3] = 1.0;
        }
        if key.srgb {
            for channel in &mut rgba[..3] {
                *channel = Self::srgb_to_linear(*channel);
            }
        }

        // Direct3D clears the viewport, further cut by the scissor.
        let viewport = state.viewport;
        let mut area = Rect {
            left: viewport.x as i32,
            top: viewport.y as i32,
            right: (viewport.x + viewport.width) as i32,
            bottom: (viewport.y + viewport.height) as i32,
        };
        if state.render_states[rs::SCISSORTESTENABLE] != 0 {
            area.left = area.left.max(state.scissor.left);
            area.top = area.top.max(state.scissor.top);
            area.right = area.right.min(state.scissor.right);
            area.bottom = area.bottom.min(state.scissor.bottom);
        }
        area.left = area.left.max(0);
        area.top = area.top.max(0);
        area.right = area.right.min(tw as i32);
        area.bottom = area.bottom.min(th as i32);

        let whole = rects.is_empty()
            && area.left == 0
            && area.top == 0
            && area.right == tw as i32
            && area.bottom == th as i32;
        if whole {
            self.end_pass();
            let mut pending = match self.pending_clear.take() {
                Some(pending) if pending.key == key => pending,
                Some(other) => {
                    if self.begin_pass(other.key, Some(&other)) {
                        self.end_pass();
                    }
                    PendingClear {
                        key,
                        ..PendingClear::default()
                    }
                }
                None => PendingClear {
                    key,
                    ..PendingClear::default()
                },
            };
            if flags & state::D3DCLEAR_TARGET != 0 {
                pending.color = Some(rgba);
            }
            if flags & state::D3DCLEAR_ZBUFFER != 0 {
                pending.depth = Some(f64::from(z));
            }
            if flags & state::D3DCLEAR_STENCIL != 0 {
                pending.stencil = Some(stencil);
            }
            self.pending_clear = Some(pending);
            return;
        }

        if !self.ensure_pass(state) {
            return;
        }
        let write_color = flags & state::D3DCLEAR_TARGET != 0;
        let Some(pipeline) = self.utility_pipeline("clear_fs", write_color) else {
            return;
        };
        let has_depth = self
            .pass
            .as_ref()
            .is_some_and(|p| p.depth_format != format::mtlfmt::INVALID);
        let has_stencil = self.pass.as_ref().is_some_and(|p| p.stencil);
        let clear_depth = has_depth && flags & state::D3DCLEAR_ZBUFFER != 0;
        let clear_stencil = has_stencil && flags & state::D3DCLEAR_STENCIL != 0;
        let depth_key = DepthKey {
            // "Always" with writes on stores the quad's depth; with writes off
            // the plane is left alone.
            depth: [u32::from(clear_depth), 1, 8],
            stencil: if clear_stencil {
                [1, 8, 1, 1, 3, 0xFF, 0xFF, 0, 8, 1, 1, 3]
            } else {
                [0; 12]
            },
        };
        let Some(depth_state) = self.depth_state(depth_key) else {
            return;
        };

        let single = [area];
        let rects = if rects.is_empty() { &single[..] } else { rects };
        let Some(pass) = self.pass.as_mut() else {
            return;
        };
        let encoder = pass.encoder.id();
        let (tw, th) = (f32::from(pass.width as u16), f32::from(pass.height as u16));
        // SAFETY: the encoder is open; the argument block is copied by the
        // call; the cached bindings are dropped because this overrides them.
        unsafe {
            msg![encoder, "setRenderPipelineState:", pipeline => Id];
            msg![encoder, "setDepthStencilState:", depth_state => Id];
            msg![encoder, "setStencilReferenceValue:", stencil => u32];
            msg![encoder, "setCullMode:", CULL_NONE => NSUInteger];
            msg![encoder, "setTriangleFillMode:", FILL_MODE_FILL => NSUInteger];
            msg![encoder, "setDepthBias:slopeScale:clamp:", 0.0 => f32, 0.0 => f32, 0.0 => f32];
            let full = MTLViewport {
                origin_x: 0.0,
                origin_y: 0.0,
                width: f64::from(tw),
                height: f64::from(th),
                znear: 0.0,
                zfar: 1.0,
            };
            msg![encoder, "setViewport:", full => MTLViewport];
            let scissor = MTLScissorRect {
                x: 0,
                y: 0,
                width: pass.width.into(),
                height: pass.height.into(),
            };
            msg![encoder, "setScissorRect:", scissor => MTLScissorRect];

            for rect in rects {
                let left = rect.left.clamp(0, pass.width as i32) as f32;
                let right = rect.right.clamp(0, pass.width as i32) as f32;
                let top = rect.top.clamp(0, pass.height as i32) as f32;
                let bottom = rect.bottom.clamp(0, pass.height as i32) as f32;
                if right <= left || bottom <= top {
                    continue;
                }
                let args = QuadArgs {
                    dst: [
                        left / tw * 2.0 - 1.0,
                        1.0 - top / th * 2.0,
                        right / tw * 2.0 - 1.0,
                        1.0 - bottom / th * 2.0,
                    ],
                    src: [0.0; 4],
                    color: [
                        rgba[0] as f32,
                        rgba[1] as f32,
                        rgba[2] as f32,
                        rgba[3] as f32,
                    ],
                    depth: z.clamp(0.0, 1.0),
                    _pad: [0.0; 3],
                };
                let bytes = std::ptr::from_ref(&args).cast::<c_void>();
                let length = std::mem::size_of::<QuadArgs>() as NSUInteger;
                msg![encoder, "setVertexBytes:length:atIndex:", bytes => *const c_void, length => NSUInteger, 0 => NSUInteger];
                msg![encoder, "setFragmentBytes:length:atIndex:", bytes => *const c_void, length => NSUInteger, 0 => NSUInteger];
                msg![encoder, "drawPrimitives:vertexStart:vertexCount:",
                    PRIMITIVE_TRIANGLE_STRIP => NSUInteger, 0 => NSUInteger, 4 => NSUInteger];
            }
        }
        pass.bound = Bound::default();
    }

    /// Draws `source` into a rectangle of the open pass's target.
    fn draw_blit(&mut self, source: Id, src: [f32; 4], dst: Rect, linear: bool) {
        let Some(pipeline) = self.utility_pipeline("blit_fs", true) else {
            return;
        };
        let Some(depth_state) = self.depth_state(DepthKey {
            depth: [0; 3],
            stencil: [0; 12],
        }) else {
            return;
        };
        let filter = if linear { 2 } else { 1 };
        let Some(sampler) = self.sampler(SamplerKey {
            address: [1; 3],
            filter: [filter, filter, 0],
            anisotropy: 1,
            max_mip: 0,
            compare: false,
        }) else {
            return;
        };
        let Some(pass) = self.pass.as_mut() else {
            return;
        };
        let encoder = pass.encoder.id();
        let (tw, th) = (pass.width as f32, pass.height as f32);
        let args = QuadArgs {
            dst: [
                dst.left as f32 / tw * 2.0 - 1.0,
                1.0 - dst.top as f32 / th * 2.0,
                dst.right as f32 / tw * 2.0 - 1.0,
                1.0 - dst.bottom as f32 / th * 2.0,
            ],
            src,
            color: [0.0; 4],
            depth: 0.0,
            _pad: [0.0; 3],
        };
        // SAFETY: as in `clear`.
        unsafe {
            msg![encoder, "setRenderPipelineState:", pipeline => Id];
            msg![encoder, "setDepthStencilState:", depth_state => Id];
            msg![encoder, "setCullMode:", CULL_NONE => NSUInteger];
            msg![encoder, "setTriangleFillMode:", FILL_MODE_FILL => NSUInteger];
            let full = MTLViewport {
                origin_x: 0.0,
                origin_y: 0.0,
                width: f64::from(tw),
                height: f64::from(th),
                znear: 0.0,
                zfar: 1.0,
            };
            msg![encoder, "setViewport:", full => MTLViewport];
            let scissor = MTLScissorRect {
                x: 0,
                y: 0,
                width: pass.width.into(),
                height: pass.height.into(),
            };
            msg![encoder, "setScissorRect:", scissor => MTLScissorRect];
            let bytes = std::ptr::from_ref(&args).cast::<c_void>();
            msg![encoder, "setVertexBytes:length:atIndex:", bytes => *const c_void,
                std::mem::size_of::<QuadArgs>() as NSUInteger => NSUInteger, 0 => NSUInteger];
            msg![encoder, "setFragmentTexture:atIndex:", source => Id, 0 => NSUInteger];
            msg![encoder, "setFragmentSamplerState:atIndex:", sampler => Id, 0 => NSUInteger];
            msg![encoder, "drawPrimitives:vertexStart:vertexCount:",
                PRIMITIVE_TRIANGLE_STRIP => NSUInteger, 0 => NSUInteger, 4 => NSUInteger];
        }
        pass.bound = Bound::default();
    }

    pub fn stretch_rect(
        &mut self,
        source: SurfaceRef,
        source_rect: Option<Rect>,
        destination: SurfaceRef,
        destination_rect: Option<Rect>,
        filter: u32,
    ) {
        let _pool = AutoreleasePool::new();
        let (Some(src), Some(dst)) = (
            Self::texture(source.texture),
            Self::texture(destination.texture),
        ) else {
            return;
        };
        if src.info.depth || dst.info.depth {
            self.warn_once(String::from(
                "a depth surface was copied, which is not supported",
            ));
            return;
        }
        let (sw, sh, _) = src.level_size(source.level);
        let (dw, dh, _) = dst.level_size(destination.level);
        let whole = |w: u32, h: u32| Rect {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        };
        let from = source_rect.unwrap_or(whole(sw, sh));
        let to = destination_rect.unwrap_or(whole(dw, dh));
        if from.right <= from.left
            || from.bottom <= from.top
            || to.right <= to.left
            || to.bottom <= to.top
        {
            return;
        }

        self.end_pass();
        self.flush_pending_clear();

        let same_size = from.right - from.left == to.right - to.left
            && from.bottom - from.top == to.bottom - to.top;
        let inside = from.left >= 0
            && from.top >= 0
            && from.right <= sw as i32
            && from.bottom <= sh as i32
            && to.left >= 0
            && to.top >= 0
            && to.right <= dw as i32
            && to.bottom <= dh as i32;
        if same_size && inside && src.info.metal == dst.info.metal {
            let command_buffer = self.command_buffer();
            if command_buffer.is_null() {
                return;
            }
            // SAFETY: both textures are live, the regions were checked to lie
            // inside their levels, and the encoder is ended in this pool.
            unsafe {
                let blit = msg![ret: Id; command_buffer, "blitCommandEncoder"];
                msg![blit, "copyFromTexture:sourceSlice:sourceLevel:sourceOrigin:sourceSize:toTexture:destinationSlice:destinationLevel:destinationOrigin:",
                    src.tex.id() => Id, NSUInteger::from(source.face) => NSUInteger, NSUInteger::from(source.level) => NSUInteger,
                    MTLOrigin { x: from.left as u64, y: from.top as u64, z: 0 } => MTLOrigin,
                    MTLSize { width: (from.right - from.left) as u64, height: (from.bottom - from.top) as u64, depth: 1 } => MTLSize,
                    dst.tex.id() => Id, NSUInteger::from(destination.face) => NSUInteger, NSUInteger::from(destination.level) => NSUInteger,
                    MTLOrigin { x: to.left as u64, y: to.top as u64, z: 0 } => MTLOrigin];
                msg![blit, "endEncoding"];
            }
            return;
        }

        // Different sizes or formats: draw the source into the destination.
        // Both are taken through their plain views so bytes pass unconverted.
        if source.level != 0 || src.kind != TextureKind::D2 {
            self.warn_once(String::from(
                "a scaled copy from a mip level or cube face is not supported",
            ));
            return;
        }
        let key = PassKey {
            color: destination,
            depth: SurfaceRef::default(),
            srgb: false,
        };
        if !self.begin_pass(key, None) {
            return;
        }
        let uv = [
            from.left as f32 / sw as f32,
            from.top as f32 / sh as f32,
            from.right as f32 / sw as f32,
            from.bottom as f32 / sh as f32,
        ];
        let source_view = src.tex.id();
        self.draw_blit(source_view, uv, to, filter >= 2);
        self.end_pass();
    }

    pub fn read_render_target(&mut self, source: SurfaceRef, destination: SurfaceRef) {
        self.stretch_rect(source, None, destination, None, 1);
        let _pool = AutoreleasePool::new();
        self.flush(true);
    }

    /// Writes the back buffer as a 32-bit BMP, which is BGRA rows top to
    /// bottom and so exactly what the texture holds.
    fn write_shot(&mut self, back_buffer: SurfaceRef, path: &std::path::Path) {
        self.flush(true);
        let Some(back) = Self::texture(back_buffer.texture) else {
            return;
        };
        let (width, height, _) = back.level_size(0);
        let pitch = width as usize * 4;
        let mut pixels = vec![0u8; pitch * height as usize];
        let region = MTLRegion {
            origin: MTLOrigin::default(),
            size: MTLSize {
                width: width.into(),
                height: height.into(),
                depth: 1,
            },
        };
        // SAFETY: the buffer is the size the region and stride describe, the
        // texture is CPU-visible, and the GPU has finished with it.
        unsafe {
            msg![back.tex.id(), "getBytes:bytesPerRow:bytesPerImage:fromRegion:mipmapLevel:slice:",
                pixels.as_mut_ptr().cast::<c_void>() => *mut c_void,
                pitch as NSUInteger => NSUInteger, pixels.len() as NSUInteger => NSUInteger,
                region => MTLRegion, 0 => NSUInteger, 0 => NSUInteger];
        }
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }

        let mut file = Vec::with_capacity(54 + pixels.len());
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(&54u32.to_le_bytes());
        file.extend_from_slice(&40u32.to_le_bytes());
        file.extend_from_slice(&(width as i32).to_le_bytes());
        file.extend_from_slice(&(-(height as i32)).to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&32u16.to_le_bytes());
        file.extend_from_slice(&[0; 24]);
        file.extend_from_slice(&pixels);
        if let Err(error) = std::fs::write(path, file) {
            log(&format!("could not write {}: {error}", path.display()));
        }
    }

    pub fn present(&mut self, back_buffer: SurfaceRef) {
        let _pool = AutoreleasePool::new();
        self.end_pass();
        self.flush_pending_clear();
        self.sync_drawable_size();

        if let Some((dir, every)) = self.shots.clone() {
            if self.frame % every == 0 {
                let path = dir.join(format!("frame-{:06}.bmp", self.frame));
                self.write_shot(back_buffer, &path);
            }
        }

        let layer = self.layer.as_ref().map(Owned::id);
        if let (Some(layer), Some(back)) = (layer, Self::texture(back_buffer.texture)) {
            let (bw, bh, _) = back.level_size(0);
            let source = back.tex.id();
            let command_buffer = self.command_buffer();
            // SAFETY: the drawable is autoreleased and used within this pool;
            // the pass that draws into its texture is ended before presenting.
            unsafe {
                let drawable = msg![ret: Id; layer, "nextDrawable"];
                if !drawable.is_null() && !command_buffer.is_null() {
                    let target = msg![ret: Id; drawable, "texture"];
                    let width = msg![ret: NSUInteger; target, "width"] as u32;
                    let height = msg![ret: NSUInteger; target, "height"] as u32;

                    let desc =
                        msg![ret: Id; class(c"MTLRenderPassDescriptor"), "renderPassDescriptor"];
                    let attachments = msg![ret: Id; desc, "colorAttachments"];
                    let attachment =
                        msg![ret: Id; attachments, "objectAtIndexedSubscript:", 0 => NSUInteger];
                    msg![attachment, "setTexture:", target => Id];
                    msg![attachment, "setLoadAction:", LOAD_ACTION_CLEAR => NSUInteger];
                    msg![attachment, "setStoreAction:", STORE_ACTION_STORE => NSUInteger];
                    let encoder = msg![ret: Id; command_buffer, "renderCommandEncoderWithDescriptor:", desc => Id];
                    if let Some(encoder) = Owned::retain(encoder) {
                        self.pass = Some(Pass {
                            encoder,
                            key: PassKey::default(),
                            color_format: format::mtlfmt::BGRA8_UNORM,
                            depth_format: format::mtlfmt::INVALID,
                            stencil: false,
                            width,
                            height,
                            bound: Bound::default(),
                        });
                        // A back buffer within a few pixels of the drawable is
                        // copied pixel for pixel and centred, losing or
                        // leaving a row or two, rather than being resampled:
                        // a full-screen window on a notched display is not
                        // quite the height the screen reports as usable.
                        const NEARLY: u32 = 16;
                        let nearly = bw.abs_diff(width) <= NEARLY && bh.abs_diff(height) <= NEARLY;
                        let (area, linear) = if nearly {
                            let left = (width as i32 - bw as i32) / 2;
                            let top = (height as i32 - bh as i32) / 2;
                            (
                                Rect {
                                    left,
                                    top,
                                    right: left + bw as i32,
                                    bottom: top + bh as i32,
                                },
                                false,
                            )
                        } else {
                            (
                                Rect {
                                    left: 0,
                                    top: 0,
                                    right: width as i32,
                                    bottom: height as i32,
                                },
                                true,
                            )
                        };
                        self.draw_blit(source, [0.0, 0.0, 1.0, 1.0], area, linear);
                        self.end_pass();
                    }
                    msg![command_buffer, "presentDrawable:", drawable => Id];
                }
            }
        }

        self.flush(false);
        self.ring_end_frame();
        self.frame += 1;
        if self.frame % 600 == 0 {
            let [begun, counted, waiting, answered, visible] = self.stats_queries;
            log(&format!(
                "frame {}: {} draws since the last report, {} pipelines, {} samplers; queries {begun} begun, {counted} counted, {waiting} polled early, {answered} answered, {visible} nonzero",
                self.frame,
                self.stats_draws,
                self.pipelines.len(),
                self.samplers.len()
            ));
            self.stats_draws = 0;
            self.stats_queries = [0; 5];
        }
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    pub fn create_query(&mut self) -> Box<Query> {
        Box::new(Query {
            pending: None,
            result: None,
        })
    }

    /// BEGIN claims a slot and has the next draw start counting into it; END
    /// stops the count and records where the answer will be. The pass may
    /// not be open yet at BEGIN, which is why counting starts at the draw.
    pub fn issue_query(&mut self, handle: Handle, flags: u32) {
        // SAFETY: as for textures.
        let Some(query) = (unsafe { (handle as *mut Query).as_mut() }) else {
            return;
        };
        if flags & state::D3DISSUE_BEGIN != 0 {
            let _pool = AutoreleasePool::new();
            // The slot belongs to the open command buffer, so there must be one.
            self.command_buffer();
            let slot = self.next_slot.min(VISIBILITY_SLOTS - 1);
            self.next_slot += 1;
            self.counting = Some((slot, false));
            self.stats_queries[0] += 1;
            query.pending = None;
            query.result = None;
        }
        if flags & state::D3DISSUE_END != 0 {
            match self.counting.take() {
                Some((slot, applied)) => {
                    if applied {
                        self.stats_queries[1] += 1;
                        if let Some(pass) = &self.pass {
                            // SAFETY: the encoder is open.
                            unsafe {
                                msg![pass.encoder.id(), "setVisibilityResultMode:offset:",
                                    VISIBILITY_DISABLED => NSUInteger, 0 => NSUInteger];
                            }
                        }
                    }
                    query.pending = Some((self.serial, slot));
                }
                None => query.result = Some(0),
            }
        }
    }

    /// The query's sample count, or `None` while the GPU has not written it.
    /// With `flush` the work is submitted and waited for, which is what
    /// Direct3D's D3DGETDATA_FLUSH asks.
    pub fn query_data(&mut self, handle: Handle, flush: bool) -> Option<u32> {
        // SAFETY: as for textures.
        let query = unsafe { (handle as *mut Query).as_mut() }?;
        if let Some(result) = query.result {
            return Some(result);
        }
        let (serial, slot) = query.pending?;

        if flush && serial == self.serial && self.command_buffer.is_some() {
            let _pool = AutoreleasePool::new();
            self.flush(true);
        }

        let Some(submission) = self.submissions.iter().find(|s| s.serial == serial) else {
            // Polled for so late that the record is gone.
            query.pending = None;
            query.result = Some(0);
            return query.result;
        };
        let Some(buffer) = submission.command_buffer.as_ref() else {
            self.stats_queries[2] += 1;
            return None;
        };
        // SAFETY: `status` and `waitUntilCompleted` are plain calls on a live,
        // committed command buffer, and the slot is inside the count array.
        let count = unsafe {
            if msg![ret: NSUInteger; buffer.id(), "status"] < COMMAND_BUFFER_COMPLETED {
                if !flush {
                    self.stats_queries[2] += 1;
                    return None;
                }
                msg![buffer.id(), "waitUntilCompleted"];
            }
            *submission.counts.add(slot)
        };
        self.stats_queries[3] += 1;
        if count != 0 {
            self.stats_queries[4] += 1;
        }
        query.pending = None;
        query.result = Some(count.min(u64::from(u32::MAX)) as u32);
        query.result
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _pool = AutoreleasePool::new();
        self.flush(true);
    }
}

/// The main display in pixels, which is what "native resolution" means on a
/// display whose points are not pixels: the area a full-screen window covers.
pub fn display_info() -> Option<(u32, u32, u32, f32, u64, String)> {
    let _pool = AutoreleasePool::new();
    // SAFETY: NSScreen's class methods and properties, with `frame` returning
    // a CGRect of four doubles; the Metal device is owned for the block.
    unsafe {
        let screen = msg![ret: Id; class(c"NSScreen"), "mainScreen"];
        if screen.is_null() {
            return None;
        }
        let frame = msg![ret: CGRect; screen, "frame"];
        let scale = msg![ret: f64; screen, "backingScaleFactor"];
        let scale = if scale > 0.0 { scale } else { 1.0 };
        // A full-screen window stops short of a camera housing, so what it
        // can show is the screen less the safe area's top inset.
        let insets = msg![ret: NSEdgeInsets; screen, "safeAreaInsets"];
        let usable_height = (frame.height - insets.top.max(0.0)).max(1.0);
        let refresh = msg![ret: i64; screen, "maximumFramesPerSecond"];

        let (memory, name) = match Owned::from_owned(MTLCreateSystemDefaultDevice()) {
            Some(device) => (
                msg![ret: u64; device.id(), "recommendedMaxWorkingSetSize"],
                crate::objc::string_from_ns(msg![ret: Id; device.id(), "name"]),
            ),
            None => (0, String::from("Metal")),
        };
        Some((
            (frame.width * scale).round() as u32,
            (usable_height * scale).round() as u32,
            refresh.clamp(0, 1000) as u32,
            scale as f32,
            memory,
            name,
        ))
    }
}
