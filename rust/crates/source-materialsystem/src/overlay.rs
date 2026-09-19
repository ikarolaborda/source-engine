//! The engine's two-dimensional drawing, gathered for a frame and
//! composited over the world.
//!
//! Everything the player reads rather than inhabits arrives here: the HUD's
//! health and ammunition, the panels the menus are built from, and the text
//! drawn into both. It reaches this module as screen-space rectangles
//! because that is what it already is by the time the engine's surface has
//! finished with it, so nothing here needs to know what a panel or a glyph
//! was, only where it goes and what to sample.

use std::collections::HashMap;

use source_render::{
    Device, IndexFormat, Indices, Pipeline, Texture, TextureFormat, TriangleList, VertexAttribute,
    Vertices,
};

/// Draws a screen-space rectangle, sampling a texture and tinting it.
///
/// Positions arrive in pixels with the origin at the top left, which is
/// how the engine's own surface addresses the screen, and are mapped here
/// rather than at every call site that produces one.
const OVERLAY_SHADER: &str = "
    #include <metal_stdlib>
    using namespace metal;

    struct Fragment {
        float4 position [[position]];
        float2 texcoord;
        float4 tint;
    };

    vertex Fragment overlay_vertex(const device packed_float2 *positions [[buffer(0)]],
                                   const device packed_float2 *texcoords [[buffer(2)]],
                                   const device packed_float4 *tints [[buffer(4)]],
                                   constant float4 &viewport [[buffer(1)]],
                                   uint index [[vertex_id]]) {
        float2 pixel = positions[index];
        // Pixels to clip space, with the vertical flipped because the
        // screen is addressed downwards and clip space is not. Held at the
        // near plane so it passes the comparison the world is drawn under.
        float2 clip = float2(pixel.x / viewport.x * 2.0 - 1.0,
                             1.0 - pixel.y / viewport.y * 2.0);
        Fragment out;
        out.position = float4(clip, 0.0, 1.0);
        out.texcoord = texcoords[index];
        out.tint = tints[index];
        return out;
    }

    fragment float4 overlay_fragment(Fragment in [[stage_in]],
                                     texture2d<float> base [[texture(0)]]) {
        constexpr sampler clamped(address::clamp_to_edge, filter::linear);
        return base.sample(clamped, in.texcoord) * in.tint;
    }
";

/// One rectangle the engine has asked for, in the order it asked.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    /// Left, top, right and bottom edges in pixels.
    pub bounds: [f32; 4],
    /// The texture coordinates of those same corners.
    pub coords: [f32; 4],
    /// Red, green, blue and alpha, each from zero to one, multiplying
    /// whatever is sampled.
    pub tint: [f32; 4],
    /// The texture to sample, or `None` to draw the tint flat.
    pub texture: Option<u32>,
}

/// The engine's two-dimensional output for one frame.
pub struct Overlay {
    pipeline: Pipeline,
    /// Textures the engine has handed over, by the identifier it names
    /// them with. The engine allocates these itself and reuses them across
    /// frames, so they are kept until it replaces them.
    textures: HashMap<u32, Texture>,
    /// A single opaque white texel, so a rectangle with no texture is the
    /// same draw as one with a texture rather than a second pipeline.
    white: Texture,
    quads: Vec<Quad>,
    frame: Option<Frame>,
}

/// One frame's geometry, uploaded together.
struct Frame {
    positions: source_render::Buffer,
    texcoords: source_render::Buffer,
    tints: source_render::Buffer,
    indices: source_render::Buffer,
    vertex_count: usize,
    /// The texture each run samples and the indices it covers. Runs are
    /// consecutive rather than gathered by texture, because two rectangles
    /// that overlap have to be composited in the order they were asked
    /// for and gathering them would reorder that.
    runs: Vec<(Option<u32>, u32, u32)>,
}

impl Overlay {
    /// Prepares the device to draw two-dimensional output.
    pub fn new(device: &Device) -> Result<Self, Error> {
        let library = device
            .compile_library(OVERLAY_SHADER)
            .map_err(|error| Error::Device(error.to_string()))?;
        Ok(Self {
            pipeline: device
                .create_blended_pipeline(&library, "overlay_vertex", "overlay_fragment")
                .map_err(|error| Error::Device(error.to_string()))?,
            textures: HashMap::new(),
            white: device
                .create_texture(1, 1, TextureFormat::Bgra8Unorm, &[255, 255, 255, 255])
                .map_err(|error| Error::Device(error.to_string()))?,
            quads: Vec::new(),
            frame: None,
        })
    }

    /// Accepts a texture the engine has rasterised itself, replacing any
    /// it had already given the same identifier.
    ///
    /// Glyphs arrive this way: the engine renders a font's characters into
    /// a sheet and hands over the pixels, so nothing here has to know how
    /// to rasterise text.
    pub fn set_texture(
        &mut self,
        device: &Device,
        id: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), Error> {
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() < expected {
            return Err(Error::ShortTexture {
                id,
                given: rgba.len(),
                expected,
            });
        }
        // The engine rasterises in red-first order and the device stores
        // blue first, so the two are reordered here rather than by asking
        // the shader to sample one and mean the other, which would then
        // apply to every texture and not just the ones that arrive this
        // way.
        let mut bgra = Vec::with_capacity(expected);
        for texel in rgba[..expected].chunks_exact(4) {
            bgra.extend_from_slice(&[texel[2], texel[1], texel[0], texel[3]]);
        }
        let texture = device
            .create_texture(width, height, TextureFormat::Bgra8Unorm, &bgra)
            .map_err(|error| Error::Device(error.to_string()))?;
        self.textures.insert(id, texture);
        Ok(())
    }

    /// Whether a texture identifier has been given pixels.
    pub fn has_texture(&self, id: u32) -> bool {
        self.textures.contains_key(&id)
    }

    /// Adds a rectangle to the frame being gathered.
    pub fn push(&mut self, quad: Quad) {
        self.quads.push(quad);
    }

    /// Discards the frame being gathered, which is how a frame starts.
    pub fn clear(&mut self) {
        self.quads.clear();
        self.frame = None;
    }

    /// Rectangles gathered so far.
    pub fn len(&self) -> usize {
        self.quads.len()
    }

    /// Whether nothing has been gathered.
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// Uploads the gathered rectangles so they can be drawn.
    ///
    /// Separate from producing the draws because the draws borrow these
    /// buffers, and the buffers are rebuilt every frame: a user interface
    /// changes whenever anything it reports changes, so there is nothing
    /// to be kept from one frame to the next.
    pub fn upload(&mut self, device: &Device, width: f32, height: f32) -> Result<(), Error> {
        self.frame = None;
        if self.quads.is_empty() || width <= 0.0 || height <= 0.0 {
            return Ok(());
        }

        let mut positions: Vec<[f32; 2]> = Vec::with_capacity(self.quads.len() * 4);
        let mut texcoords: Vec<[f32; 2]> = Vec::with_capacity(self.quads.len() * 4);
        let mut tints: Vec<[f32; 4]> = Vec::with_capacity(self.quads.len() * 4);
        let mut indices: Vec<u32> = Vec::with_capacity(self.quads.len() * 6);
        let mut runs: Vec<(Option<u32>, u32, u32)> = Vec::new();

        for quad in &self.quads {
            let [left, top, right, bottom] = quad.bounds;
            let [u0, v0, u1, v1] = quad.coords;
            let base = positions.len() as u32;
            for (x, y, u, v) in [
                (left, top, u0, v0),
                (right, top, u1, v0),
                (right, bottom, u1, v1),
                (left, bottom, u0, v1),
            ] {
                positions.push([x, y]);
                texcoords.push([u, v]);
                tints.push(quad.tint);
            }
            let first = indices.len() as u32;
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            // A texture the engine named but never gave pixels for is
            // drawn as the flat tint rather than skipped, so a missing
            // sheet shows as a block where it belongs instead of leaving
            // a hole with nothing to explain it.
            let texture = quad.texture.filter(|id| self.textures.contains_key(id));
            match runs.last_mut() {
                Some((last, _, count)) if *last == texture => *count += 6,
                _ => runs.push((texture, first, 6)),
            }
        }

        let upload = |bytes: &[u8]| {
            device
                .create_buffer(bytes)
                .map_err(|error| Error::Device(error.to_string()))
        };
        self.frame = Some(Frame {
            vertex_count: positions.len(),
            positions: upload(as_bytes(&positions))?,
            texcoords: upload(as_bytes(&texcoords))?,
            tints: upload(as_bytes(&tints))?,
            indices: upload(as_bytes(&indices))?,
            runs,
        });
        Ok(())
    }

    /// The draws for the frame last uploaded, to be issued after the world
    /// so they sit over it.
    pub fn draws<'a>(&'a self, viewport: &'a [u8]) -> Vec<TriangleList<'a>> {
        let Some(frame) = &self.frame else {
            return Vec::new();
        };
        frame
            .runs
            .iter()
            .map(|(texture, first, count)| {
                let texture = texture
                    .and_then(|id| self.textures.get(&id))
                    .unwrap_or(&self.white);
                TriangleList::new(
                    &self.pipeline,
                    Vertices::Buffered {
                        buffer: &frame.positions,
                        count: frame.vertex_count,
                        stride: std::mem::size_of::<[f32; 2]>(),
                    },
                )
                .with_texture(texture)
                .with_coordinates(VertexAttribute::packed::<[f32; 2]>(&frame.texcoords))
                .with_shade(VertexAttribute::packed::<[f32; 4]>(&frame.tints))
                .with_indices(Indices {
                    buffer: &frame.indices,
                    count: *count as usize,
                    format: IndexFormat::Uint32,
                    first: *first as usize,
                })
                .with_uniforms(viewport)
            })
            .collect()
    }
}

/// The viewport a frame's rectangles are addressed against, as the shader
/// reads it.
pub fn viewport(width: f32, height: f32) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (slot, value) in [width, height, 0.0, 0.0].iter().enumerate() {
        bytes[slot * 4..slot * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn as_bytes<T>(values: &[T]) -> &[u8] {
    // SAFETY: the arrays of `f32` and `u32` this is called with have no
    // padding and no invalid bit patterns, and the slice produced borrows
    // the same memory for the same lifetime.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}

/// What can go wrong gathering or drawing two-dimensional output.
#[derive(Debug)]
pub enum Error {
    /// The device refused something.
    Device(String),
    /// A texture was offered fewer pixels than its size calls for.
    ShortTexture {
        /// The identifier the engine named it with.
        id: u32,
        /// Bytes offered.
        given: usize,
        /// Bytes its width and height call for.
        expected: usize,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Device(message) => write!(formatter, "the device refused: {message}"),
            Self::ShortTexture {
                id,
                given,
                expected,
            } => write!(
                formatter,
                "texture {id} was offered {given} bytes and needs {expected}"
            ),
        }
    }
}

impl std::error::Error for Error {}
