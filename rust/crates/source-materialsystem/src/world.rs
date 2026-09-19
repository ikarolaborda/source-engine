//! A loaded map, held on the GPU, ready to be drawn from anywhere in it.
//!
//! Everything here already existed as a test that rendered a shipped map
//! offscreen and asserted on the pixels. That proved the chain from BSP to
//! frame, but it left the chain living in a test, where the engine cannot
//! reach it. This is the same chain as something the running game owns: a
//! map is loaded once into buffers and textures, and each frame picks what
//! is visible from where the player stands and draws it.
//!
//! The split matters for what comes next. The engine's frame loop has no
//! business knowing how lightmaps are packed or which batches a visibility
//! row keeps, and the renderer has no business knowing where the player is.
//! So the engine hands in an eye each frame and gets a presented frame
//! back, which is the whole of the interface between them.

use crate::MaterialSystem;
use source_bsp::{
    Bsp, Frustum, LightmapAtlas, Lightmaps, Materials, Placement, Surfaces, TexturedGeometry,
    VisibleFaces,
};
use source_filesystem::{Position, SearchPaths};
use source_render::{
    view_projection, Buffer, ClearColor, Device, Eye, IndexFormat, Indices, Lens, Pipeline,
    Swapchain, Texture, TextureFormat, TriangleList, VertexAttribute, Vertices,
};
use std::collections::HashSet;
use std::fmt;

/// Samples each surface's material and modulates it by the map's baked
/// lighting.
///
/// The lightmap is sampled clamped rather than repeating, because a
/// surface's samples are one block of a packed image and wrapping would
/// reach into an unrelated surface's lighting. The doubling is the
/// overbright the format's samples are stored against, which is what keeps
/// a fully lit surface at its own albedo instead of half of it.
const LIT_SHADER: &str = "
    #include <metal_stdlib>
    using namespace metal;

    struct Surface {
        float4 position [[position]];
        float2 texcoord;
        float2 luxel;
    };

    vertex Surface world_vertex(const device packed_float3 *positions [[buffer(0)]],
                                const device packed_float2 *texcoords [[buffer(2)]],
                                const device packed_float2 *luxels [[buffer(3)]],
                                constant float4x4 &view_projection [[buffer(1)]],
                                uint index [[vertex_id]]) {
        Surface out;
        out.position = view_projection * float4(positions[index], 1.0);
        out.texcoord = texcoords[index];
        out.luxel = luxels[index];
        return out;
    }

    fragment float4 world_fragment(Surface in [[stage_in]],
                                   texture2d<float> base [[texture(0)]],
                                   texture2d<float> lighting [[texture(1)]]) {
        constexpr sampler tiling(address::repeat, filter::linear, mip_filter::linear);
        constexpr sampler packed(address::clamp_to_edge, filter::linear);
        float3 albedo = base.sample(tiling, in.texcoord).rgb;
        float3 light = lighting.sample(packed, in.luxel).rgb * 2.0;
        return float4(albedo * light, 1.0);
    }
";

/// How far a frame sees, as a multiple of the map's own longest axis.
///
/// A map is drawn whole rather than faded out at a distance, so the far
/// plane only has to clear the geometry; making it tighter would clip the
/// far side of a long map and making it looser costs depth precision.
const FAR_PLANE_REACH: f32 = 2.0;

/// The horizontal field of view the engine's default matches.
const DEFAULT_FOV: f32 = 90.0;

/// A map loaded onto the GPU, with everything a frame needs to draw it.
pub struct Scene {
    world: source_bsp::World,
    surfaces: Surfaces,
    names: Materials,
    geometry: TexturedGeometry,
    /// The brush models the map's entities place, kept so that each frame's
    /// visibility row can be widened to the models the player can see.
    placements: Vec<(usize, Placement)>,
    materials: MaterialSystem,
    pipeline: Pipeline,
    lightmap: Texture,
    positions: Buffer,
    texcoords: Buffer,
    luxels: Buffer,
    indices: Buffer,
    /// The far plane, from the map's own bounds.
    far: f32,
}

/// What a frame did, so a caller can tell a drawn frame from an empty one
/// without reading back pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    /// Draw calls issued, one per material with visible surfaces.
    pub batches: usize,
    /// Indices in those draws, which is three per triangle.
    pub indices: usize,
}

impl Scene {
    /// Loads a map and everything it draws with onto `device`.
    ///
    /// The map's own archive is mounted ahead of the game's, because the
    /// compiler writes the map's generated materials, chiefly its cubemaps,
    /// into the BSP rather than beside it.
    pub fn load(device: &Device, paths: &mut SearchPaths, map: &str) -> Result<Self, Error> {
        let path = if map.ends_with(".bsp") {
            map.to_owned()
        } else {
            format!("maps/{map}.bsp")
        };
        let bytes = paths.read(&path, None).map_err(|error| Error::Read {
            path: path.clone(),
            message: error.to_string(),
        })?;
        let bsp = Bsp::parse(&bytes).map_err(|error| Error::Map {
            path: path.clone(),
            message: error.to_string(),
        })?;
        let stem = path
            .rsplit('/')
            .next()
            .unwrap_or(&path)
            .trim_end_matches(".bsp")
            .to_owned();
        paths
            .mount_pak(bsp.pakfile(), &stem, "GAME", Position::Head)
            .map_err(|error| Error::Read {
                path: format!("{path} (embedded archive)"),
                message: error.to_string(),
            })?;

        let map_error = |error: source_bsp::Error| Error::Map {
            path: path.clone(),
            message: error.to_string(),
        };
        let surfaces = Surfaces::parse(&bsp).map_err(map_error)?;
        let names = Materials::parse(&bsp).map_err(map_error)?;
        let lighting = Lightmaps::parse(&bsp).map_err(map_error)?;
        let world = source_bsp::World::parse(&bsp).map_err(map_error)?;
        let entities = source_bsp::Entities::parse(&bsp).map_err(map_error)?;

        // The world model is drawn where it is stored; the map's other
        // models are its doors, lifts and trigger volumes, each cut out
        // about an origin of its own and drawn where its entity stands.
        let placements = entities.brush_placements();
        let mut groups: Vec<(Placement, Vec<usize>)> =
            vec![(Placement::IDENTITY, world.world_faces().collect())];
        for (model, placement) in &placements {
            groups.push((*placement, world.models()[*model].faces().collect()));
        }
        let lit_faces: Vec<usize> = groups
            .iter()
            .flat_map(|(_, faces)| faces.iter().copied())
            .collect();
        let atlas =
            LightmapAtlas::pack(&surfaces, &names, &lighting, lit_faces).map_err(map_error)?;
        let geometry = surfaces
            .triangulate_placed(&names, Some(&atlas), groups)
            .map_err(map_error)?;

        let library = device
            .compile_library(LIT_SHADER)
            .map_err(|error| Error::Device(error.to_string()))?;
        let pipeline = device
            .create_depth_pipeline(&library, "world_vertex", "world_fragment")
            .map_err(|error| Error::Device(error.to_string()))?;
        let lightmap = device
            .create_texture(
                atlas.width(),
                atlas.height(),
                TextureFormat::Bgra8Unorm,
                atlas.pixels(),
            )
            .map_err(|error| Error::Device(error.to_string()))?;

        // Every material the map's batches name, loaded once each. A
        // material that will not bind is not fatal: a handful in each map
        // name textures the engine renders rather than stores, and the
        // surfaces that use them are simply not drawn.
        let mut materials = MaterialSystem::new();
        let mut seen = HashSet::new();
        for batch in &geometry.batches {
            let Some(name) = names.name(batch.texdata) else {
                continue;
            };
            if !seen.insert(name.to_owned()) {
                continue;
            }
            let _ = materials.bind(device, paths, name);
        }

        let upload = |values: &[u8]| {
            device
                .create_buffer(values)
                .map_err(|error| Error::Device(error.to_string()))
        };
        let positions = upload(as_bytes(&geometry.positions))?;
        let texcoords = upload(as_bytes(&geometry.texcoords))?;
        let luxels = upload(as_bytes(&geometry.lightmap_coords))?;
        let indices = upload(as_bytes(&geometry.indices))?;

        let mut low = [f32::MAX; 3];
        let mut high = [f32::MIN; 3];
        for position in &geometry.positions {
            for axis in 0..3 {
                low[axis] = low[axis].min(position[axis]);
                high[axis] = high[axis].max(position[axis]);
            }
        }
        let span = (high[0] - low[0])
            .max(high[1] - low[1])
            .max(high[2] - low[2])
            .max(1000.0);

        Ok(Self {
            world,
            surfaces,
            names,
            geometry,
            placements,
            materials,
            pipeline,
            lightmap,
            positions,
            texcoords,
            luxels,
            indices,
            far: span * FAR_PLANE_REACH,
        })
    }

    /// Draws what can be seen from `eye` and presents it.
    ///
    /// What is drawn is chosen twice over: by the map's own visibility set,
    /// which the compiler worked out ahead of time, and by the view's own
    /// bounding planes. A map holds far more world than any one place in it
    /// can see, and drawing the rest is work no pixel depends on.
    pub fn present(
        &self,
        device: &Device,
        swapchain: &Swapchain,
        eye: Eye,
    ) -> Result<Drawn, Error> {
        let (width, height) = swapchain.size();
        if width == 0 || height == 0 {
            return Err(Error::Device("the window has no drawable area".to_owned()));
        }
        let lens = Lens {
            horizontal_fov: DEFAULT_FOV,
            aspect: width as f32 / height as f32,
            near: 1.0,
            far: self.far,
        };
        let transform =
            view_projection(eye, lens).map_err(|error| Error::Device(error.to_string()))?;

        // A point outside the map's visibility set, which is where the
        // player is during a level change or when noclipped into the void,
        // has no row to select by. The map is still drawn from there, with
        // only the view's own planes deciding, rather than dropping the
        // frame.
        let batches = match VisibleFaces::select(&self.world, &self.surfaces, eye.position) {
            Ok(visible) => {
                let visible = visible.placing(&self.world, &self.placements);
                let frustum = Frustum::new(transform.frustum_planes());
                self.geometry.visible_batches(&visible, Some(&frustum))
            }
            Err(_) => self.geometry.batches.clone(),
        };

        let draws: Vec<TriangleList<'_>> = batches
            .iter()
            .filter_map(|batch| {
                let name = self.names.name(batch.texdata)?;
                let texture = self.materials.get(name)?.texture.as_ref()?;
                Some(
                    TriangleList::new(
                        &self.pipeline,
                        Vertices::Buffered {
                            buffer: &self.positions,
                            count: self.geometry.positions.len(),
                            stride: std::mem::size_of::<[f32; 3]>(),
                        },
                    )
                    .with_texture(texture)
                    .with_coordinates(VertexAttribute::packed::<[f32; 2]>(&self.texcoords))
                    .with_indices(Indices {
                        buffer: &self.indices,
                        count: batch.index_count,
                        format: IndexFormat::Uint32,
                        first: batch.first_index,
                    })
                    .with_uniforms(transform.as_bytes())
                    .with_lightmap(
                        &self.lightmap,
                        VertexAttribute::packed::<[f32; 2]>(&self.luxels),
                    ),
                )
            })
            .collect();

        // Counted from the batches rather than the draws, because a batch
        // whose material never bound produces no draw and so contributes
        // nothing to either figure.
        // A run can be asked to keep the frames it presents, which is the
        // only way to see what reached the screen on a machine where
        // nothing may record the display. These are the presented
        // drawable's own pixels rather than a second render of the same
        // view, so they cannot differ from what was shown.
        let recording = std::env::var_os("SOURCE_METAL_SHOT_DIR")
            .filter(|_| SHOTS_TAKEN.load(std::sync::atomic::Ordering::Relaxed) < MOST_SHOTS_KEPT);
        if let Some(directory) = recording {
            let frame = self
                .device_capture(device, swapchain, &draws)
                .map_err(|error| Error::Device(error.to_string()))?;
            let index = SHOTS_TAKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::path::Path::new(&directory).join(format!("frame-{index:04}.ppm"));
            write_ppm(&path, &frame, width, height);
            return Ok(Drawn {
                batches: draws.len(),
                indices: batches.iter().map(|batch| batch.index_count).sum(),
            });
        }

        let drawn = Drawn {
            batches: draws.len(),
            indices: batches
                .iter()
                .filter(|batch| {
                    self.names
                        .name(batch.texdata)
                        .and_then(|name| self.materials.get(name))
                        .is_some_and(|binding| binding.texture.is_some())
                })
                .map(|batch| batch.index_count)
                .sum(),
        };
        device
            .present(
                swapchain,
                ClearColor {
                    red: 0.0,
                    green: 0.0,
                    blue: 0.0,
                    alpha: 1.0,
                },
                &draws,
            )
            .map_err(|error| Error::Device(error.to_string()))?;
        Ok(drawn)
    }

    /// Presents and keeps the pixels, for a run that was asked to record
    /// what it drew.
    fn device_capture(
        &self,
        device: &Device,
        swapchain: &Swapchain,
        draws: &[TriangleList<'_>],
    ) -> Result<source_render::Readback, source_render::DeviceError> {
        device.present_capturing(
            swapchain,
            ClearColor {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            },
            draws,
        )
    }

    /// The triangles the whole map holds, which is what a frame draws from.
    pub fn triangle_count(&self) -> usize {
        self.geometry.indices.len() / 3
    }

    /// How many of the map's materials resolved to a texture.
    pub fn bound_materials(&self) -> usize {
        self.materials.len()
    }
}

/// How many frames a recording run has kept, so each lands in its own file.
static SHOTS_TAKEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Reading a frame back waits for the GPU, so a run that recorded every
/// frame would no longer be running at the speed it is meant to measure.
/// Enough to see the view move is enough.
const MOST_SHOTS_KEPT: usize = 12;

/// Writes a presented frame out as a portable pixmap.
///
/// Netpbm rather than PNG because it needs no encoder, and a frame that
/// cannot be written is skipped rather than failing the run: recording is
/// something a run was asked to do alongside its real work, not instead
/// of it.
fn write_ppm(path: &std::path::Path, frame: &source_render::Readback, width: u32, height: u32) {
    use std::io::Write;
    let Ok(file) = std::fs::File::create(path) else {
        return;
    };
    let mut out = std::io::BufWriter::new(file);
    if write!(out, "P6\n{width} {height}\n255\n").is_err() {
        return;
    }
    for y in 0..height {
        for x in 0..width {
            // The drawable is BGRA, so the channels are reordered here
            // rather than in the shader, which writes what Metal expects.
            let Some(pixel) = frame.pixel(x, y) else {
                return;
            };
            if out.write_all(&[pixel[2], pixel[1], pixel[0]]).is_err() {
                return;
            }
        }
    }
}

/// Reads the bytes of a slice of plain data.
fn as_bytes<T>(values: &[T]) -> &[u8] {
    // SAFETY: the callers pass arrays of `f32` and `u32`, which have no
    // padding and no invalid bit patterns, so their bytes are readable.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

/// Why a map could not be made ready to draw.
#[derive(Debug)]
pub enum Error {
    Read { path: String, message: String },
    Map { path: String, message: String },
    Device(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } => write!(f, "reading {path}: {message}"),
            Self::Map { path, message } => write!(f, "map {path}: {message}"),
            Self::Device(message) => write!(f, "device: {message}"),
        }
    }
}

impl std::error::Error for Error {}
