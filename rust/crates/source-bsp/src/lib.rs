//! Bounds-checked Source BSP header/lump parser and deterministic repacker.

use source_binary::{Reader, Writer};
use std::fmt;

pub const BSP_IDENT: u32 = 0x5053_4256; // "VBSP" in little endian
pub const BSP_LUMP_COUNT: usize = 64;
pub const BSP_HEADER_SIZE: usize = 4 + 4 + BSP_LUMP_COUNT * 16 + 4;
pub const LUMP_PLANES: usize = 1;
pub const LUMP_VERTEXES: usize = 3;
pub const LUMP_VISIBILITY: usize = 4;
pub const LUMP_NODES: usize = 5;
pub const LUMP_TEXDATA: usize = 2;
pub const LUMP_TEXINFO: usize = 6;
pub const LUMP_TEXDATA_STRING_DATA: usize = 43;
pub const LUMP_TEXDATA_STRING_TABLE: usize = 44;
pub const LUMP_FACES: usize = 7;
pub const LUMP_LEAVES: usize = 10;
pub const LUMP_EDGES: usize = 12;
pub const LUMP_SURFEDGES: usize = 13;
pub const LUMP_LEAFFACES: usize = 16;
pub const LUMP_ENTITIES: usize = 0;
pub const LUMP_MODELS: usize = 14;
/// The ZIP archive a map carries its own content in, chiefly the cubemap
/// materials the compiler generates per surface position.
pub const LUMP_PAKFILE: usize = 40;
/// The lump holding lumps of its own, one per thing the game rather than
/// the compiler put in the map. The static props are one of them.
pub const LUMP_GAME_LUMP: usize = 35;
/// The baked lighting of the world's surfaces.
pub const LUMP_LIGHTING: usize = 8;
/// The same, as compiled for high dynamic range. A map built for it carries
/// both lumps, and this one is what it was authored against.
pub const LUMP_LIGHTING_HDR: usize = 53;
/// Which of the ambient samples below belong to each leaf.
pub const LUMP_LEAF_AMBIENT_INDEX: usize = 52;
pub const LUMP_LEAF_AMBIENT_INDEX_HDR: usize = 51;
/// The light arriving at points inside the map's open leaves, from every
/// direction, which is how anything that is not a world surface is lit.
pub const LUMP_LEAF_AMBIENT_LIGHTING: usize = 56;
pub const LUMP_LEAF_AMBIENT_LIGHTING_HDR: usize = 55;

const PLANE_SIZE: usize = 20;
const NODE_SIZE: usize = 32;
const MODEL_SIZE: usize = 48;
const LEAF_V0_SIZE: usize = 56;
const LEAF_V1_SIZE: usize = 32;
const VERTEX_SIZE: usize = 12;
const EDGE_SIZE: usize = 4;
const SURFEDGE_SIZE: usize = 4;
const FACE_SIZE: usize = 56;
const LEAFFACE_SIZE: usize = 2;
const TEXINFO_SIZE: usize = 72;
const TEXDATA_SIZE: usize = 32;
const TEXDATA_STRING_TABLE_SIZE: usize = 4;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_lump_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 2 * 1024 * 1024 * 1024usize,
            max_lump_size: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    InvalidIdent(u32),
    UnsupportedVersion(i32),
    FileTooLarge(usize),
    InvalidLump {
        index: usize,
        offset: i32,
        length: i32,
    },
    LumpTooLarge {
        index: usize,
        length: usize,
    },
    CompressedCoreLump(usize),
    InvalidRecordSize {
        lump: usize,
        length: usize,
        record_size: usize,
    },
    UnsupportedLumpVersion {
        lump: usize,
        version: i32,
    },
    InvalidPlaneIndex {
        node: usize,
        plane: i32,
    },
    InvalidChildIndex {
        node: usize,
        child: i32,
    },
    InvalidCluster(i32),
    InvalidVisibilityHeader,
    InvalidVisibilityOffset(i32),
    InvalidVisibilityRun,
    MissingWorldTree,
    TraversalCycle,
    NonFinitePoint,
    SizeOverflow,
    /// A face names a run of surface edges that is not inside the lump.
    InvalidFaceRange {
        face: usize,
        first: i32,
        count: u16,
    },
    InvalidSurfedge {
        surfedge: usize,
        edge: i32,
    },
    InvalidEdgeVertex {
        edge: usize,
        vertex: u16,
    },
    InvalidFaceIndex(usize),
    /// Fewer than three edges cannot enclose a surface.
    DegenerateFace {
        face: usize,
        edges: u16,
    },
    NonFiniteVertex(usize),
    /// A face names a texinfo the lump does not hold.
    InvalidTexInfo(i16),
    /// A texinfo names a texdata the lump does not hold.
    InvalidTexData(i32),
    /// The lighting lump's length is not a whole number of samples.
    InvalidLightingLump(usize),
    /// A face's lightmap starts at an offset that is not a whole sample.
    InvalidLightOffset(i32),
    /// A face states a lightmap extent that cannot be stored.
    InvalidLightmapExtent {
        size: [i32; 2],
    },
    /// A face's lightmap runs past the end of the lighting lump.
    LightmapOutOfRange {
        offset: i32,
        length: usize,
    },
    /// The map's lightmaps do not fit the packed image.
    LightmapAtlasTooLarge {
        blocks: usize,
        limit: u32,
    },
    /// The entity lump is not the brace-delimited key and value text the
    /// format states.
    InvalidEntityText,
    /// A game lump points outside the file it is part of.
    InvalidGameLumpRange {
        offset: usize,
        length: usize,
    },
    /// A lump's records do not divide the bytes it holds, so the stride
    /// being read is wrong.
    InvalidLumpLength {
        lump: usize,
        length: usize,
    },
    /// A leaf points at ambient samples the lump does not hold.
    InvalidAmbientRange {
        first: usize,
        count: usize,
        samples: usize,
    },
    /// The static prop records do not divide the bytes the lump holds for
    /// them, so the stride its version implies is wrong.
    InvalidStaticPropStride {
        version: i32,
        stride: usize,
        count: usize,
        length: usize,
    },
    /// A static prop names a model the dictionary does not hold.
    InvalidStaticPropModel {
        index: usize,
        model: usize,
    },
    /// A static prop names a run of the shared leaf table that is not there.
    InvalidStaticPropLeaves {
        index: usize,
        first: usize,
        count: usize,
    },
    /// A leaf names a run of the leaf-face lump that the lump does not hold.
    InvalidLeafFaceRun {
        first: usize,
        end: usize,
    },
    /// A texdata names a string the table does not hold, or one that runs
    /// off the end of the string data.
    InvalidMaterialName(u32),
    /// A texture projection that is not a number, or a material of no size,
    /// neither of which yields a usable texture coordinate.
    UnusableProjection(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::InvalidIdent(value) => write!(f, "invalid BSP identifier 0x{value:08x}"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported BSP version {version}"),
            Self::FileTooLarge(size) => {
                write!(f, "BSP is larger than the configured limit: {size}")
            }
            Self::InvalidLump {
                index,
                offset,
                length,
            } => write!(f, "BSP lump {index} has invalid range {offset}+{length}"),
            Self::LumpTooLarge { index, length } => {
                write!(
                    f,
                    "BSP lump {index} is larger than the configured limit: {length}"
                )
            }
            Self::CompressedCoreLump(index) => {
                write!(f, "BSP core lump {index} is compressed")
            }
            Self::InvalidRecordSize {
                lump,
                length,
                record_size,
            } => write!(
                f,
                "BSP lump {lump} length {length} is not a multiple of {record_size}"
            ),
            Self::UnsupportedLumpVersion { lump, version } => {
                write!(f, "BSP lump {lump} has unsupported version {version}")
            }
            Self::InvalidPlaneIndex { node, plane } => {
                write!(f, "BSP node {node} references invalid plane {plane}")
            }
            Self::InvalidChildIndex { node, child } => {
                write!(f, "BSP node {node} references invalid child {child}")
            }
            Self::InvalidCluster(cluster) => write!(f, "invalid BSP cluster {cluster}"),
            Self::InvalidVisibilityHeader => write!(f, "invalid BSP visibility header"),
            Self::InvalidVisibilityOffset(offset) => {
                write!(f, "invalid BSP visibility offset {offset}")
            }
            Self::InvalidVisibilityRun => write!(f, "invalid BSP visibility RLE run"),
            Self::MissingWorldTree => write!(f, "BSP has no world tree"),
            Self::TraversalCycle => write!(f, "BSP world tree contains a cycle"),
            Self::NonFinitePoint => write!(f, "BSP query point must be finite"),
            Self::SizeOverflow => write!(f, "BSP size cannot be represented by the format"),
            Self::InvalidFaceRange { face, first, count } => write!(
                f,
                "BSP face {face} names surface edges {first}+{count}, which is outside the lump"
            ),
            Self::InvalidSurfedge { surfedge, edge } => write!(
                f,
                "BSP surface edge {surfedge} references invalid edge {edge}"
            ),
            Self::InvalidEdgeVertex { edge, vertex } => {
                write!(f, "BSP edge {edge} references invalid vertex {vertex}")
            }
            Self::InvalidFaceIndex(face) => write!(f, "invalid BSP face index {face}"),
            Self::DegenerateFace { face, edges } => {
                write!(f, "BSP face {face} has only {edges} edges")
            }
            Self::NonFiniteVertex(index) => write!(f, "BSP vertex {index} is not finite"),
            Self::InvalidTexInfo(index) => write!(f, "invalid BSP texinfo index {index}"),
            Self::InvalidTexData(index) => write!(f, "invalid BSP texdata index {index}"),
            Self::InvalidMaterialName(offset) => {
                write!(f, "invalid BSP material name at offset {offset}")
            }
            Self::UnusableProjection(index) => {
                write!(f, "BSP texinfo {index} has no usable texture projection")
            }
            Self::InvalidLightingLump(length) => {
                write!(
                    f,
                    "BSP lighting lump of {length} bytes is not whole samples"
                )
            }
            Self::InvalidLightOffset(offset) => {
                write!(f, "BSP face has an unusable lightmap offset {offset}")
            }
            Self::InvalidLightmapExtent { size } => {
                write!(
                    f,
                    "BSP face has an unusable lightmap extent {}x{}",
                    size[0], size[1]
                )
            }
            Self::LightmapOutOfRange { offset, length } => {
                write!(
                    f,
                    "BSP face's {length} lightmap bytes at {offset} run past the lighting lump"
                )
            }
            Self::LightmapAtlasTooLarge { blocks, limit } => {
                write!(
                    f,
                    "{blocks} lightmap blocks do not pack within {limit} rows"
                )
            }
            Self::InvalidEntityText => write!(f, "invalid BSP entity text"),
            Self::InvalidGameLumpRange { offset, length } => write!(
                f,
                "game lump at {offset} for {length} bytes runs past the map"
            ),
            Self::InvalidLumpLength { lump, length } => write!(
                f,
                "lump {lump} holds {length} bytes, which its records do not divide"
            ),
            Self::InvalidAmbientRange {
                first,
                count,
                samples,
            } => write!(
                f,
                "a leaf claims {count} ambient samples from {first}, past the {samples} stored"
            ),
            Self::InvalidStaticPropStride {
                version,
                stride,
                count,
                length,
            } => write!(
                f,
                "static prop lump version {version} implies a {stride}-byte record, \
                 which does not divide {length} bytes over {count} props"
            ),
            Self::InvalidStaticPropModel { index, model } => write!(
                f,
                "static prop {index} names model {model}, which the dictionary does not hold"
            ),
            Self::InvalidStaticPropLeaves {
                index,
                first,
                count,
            } => write!(
                f,
                "static prop {index} names leaves {first}..{}, which the lump does not hold",
                first + count
            ),
            Self::InvalidLeafFaceRun { first, end } => {
                write!(
                    f,
                    "BSP leaf names leaf faces {first}..{end}, which the lump does not hold"
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    pub normal: [f32; 3],
    pub distance: f32,
    pub kind: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    pub plane: i32,
    pub children: [i32; 2],
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub first_face: u16,
    pub face_count: u16,
    pub area: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leaf {
    pub contents: i32,
    pub cluster: i16,
    pub area: u16,
    pub flags: u8,
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub first_leaf_face: u16,
    pub leaf_face_count: u16,
    pub first_leaf_brush: u16,
    pub leaf_brush_count: u16,
    pub water_data_id: i16,
}

/// One of the map's brush models.
///
/// The first is the world itself, the one the visibility set and the tree
/// are about. The rest are the map's brushwork that moves or is switched:
/// doors, lifts, breakable walls. Each is a range of surfaces and its own
/// subtree, positioned by whichever entity owns it, so none of their
/// surfaces appear in a leaf of the world's tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Model {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub origin: [f32; 3],
    pub head_node: i32,
    pub first_face: i32,
    pub face_count: i32,
}

impl Model {
    /// The surfaces this model holds.
    pub fn faces(&self) -> std::ops::Range<usize> {
        let first = self.first_face.max(0) as usize;
        first..first + self.face_count.max(0) as usize
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityKind {
    PotentiallyVisible = 0,
    PotentiallyAudible = 1,
}

impl TryFrom<u32> for VisibilityKind {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            0 => Ok(Self::PotentiallyVisible),
            1 => Ok(Self::PotentiallyAudible),
            _ => Err(Error::InvalidVisibilityHeader),
        }
    }
}

#[derive(Debug, Clone)]
struct Visibility {
    offsets: Vec<[i32; 2]>,
    bytes: Vec<u8>,
}

/// One world surface, as the format stores it.
///
/// A face does not hold its own vertices: it names a run of surface edges,
/// each of which names an edge, each of which names two vertices. The run is
/// walked in order so the polygon comes out wound the way the map compiler
/// wound it, which is what decides which side of the surface faces out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    pub plane: u16,
    /// Which side of the plane the surface faces, as the format's flag.
    pub side: u8,
    pub first_surfedge: i32,
    pub surfedge_count: u16,
    pub texinfo: i16,
    /// The displacement this face is the boundary of, or negative for none.
    /// This is sixteen bits wide and is followed by a second sixteen-bit
    /// field, so reading the pair as one number leaves a real displacement
    /// looking like a negative one.
    pub displacement: i16,
    /// Which light style each of the face's lightmap layers belongs to.
    /// `0` is the static lighting every lit surface has, and `255` marks a
    /// layer that is not present.
    pub styles: [u8; 4],
    /// Where this face's lightmap samples begin in the lighting lump, or
    /// negative for a face with no lightmap at all.
    pub light_offset: i32,
    /// The lightmap's origin on the surface, in luxels.
    pub lightmap_mins: [i32; 2],
    /// The lightmap's extent, in luxels, one less than the samples stored:
    /// a size of zero still has one sample, because the samples are at the
    /// corners of the luxel grid rather than inside it.
    pub lightmap_size: [i32; 2],
}

/// The style value marking a lightmap layer that is not stored.
pub const LIGHT_STYLE_NONE: u8 = 255;

impl Face {
    /// Whether this surface is the visible side of a displacement, whose
    /// geometry lives in a separate lump rather than in this face's edges.
    pub fn is_displacement(&self) -> bool {
        self.displacement >= 0
    }

    /// How many lightmap samples this face stores per style.
    ///
    /// The grid is one wider and one taller than the extent, because the
    /// samples sit at the corners of the luxels rather than in their middles.
    pub fn lightmap_sample_count(&self) -> Option<usize> {
        let width = usize::try_from(self.lightmap_size[0].checked_add(1)?).ok()?;
        let height = usize::try_from(self.lightmap_size[1].checked_add(1)?).ok()?;
        width.checked_mul(height)
    }

    /// How many light styles this face stores a layer for.
    ///
    /// A lit surface always has the static layer, and may have up to three
    /// more for lights that switch or flicker. Only the first is used here,
    /// since the rest need the runtime's light style animation.
    pub fn light_style_count(&self) -> usize {
        self.styles
            .iter()
            .take_while(|style| **style != LIGHT_STYLE_NONE)
            .count()
    }

    pub fn has_lightmap(&self) -> bool {
        self.light_offset >= 0 && self.light_style_count() != 0
    }
}

/// How a surface's texture is laid across it, and which material that is.
///
/// The projection is stated as two planes: a point's `u` is its distance
/// along the first and `v` along the second, both in texels, which is why
/// they have to be divided by the material's own size to become the `0..1`
/// coordinates a sampler takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexInfo {
    /// `[s, t][x, y, z, offset]`, in texels per world unit.
    pub texture_vectors: [[f32; 4]; 2],
    /// The same form, in luxels per world unit, projecting a point onto the
    /// surface's lightmap. Luxels are far coarser than texels, which is why
    /// a lightmap for a whole wall is a handful of samples.
    pub lightmap_vectors: [[f32; 4]; 2],
    pub flags: i32,
    pub texdata: i32,
}

/// `SURF_SKY`, `SURF_NODRAW`, `SURF_SKIP` and `SURF_HINT`: surfaces the
/// compiler leaves in the lump but that are never drawn as world geometry.
const SURF_SKY: i32 = 0x0004;
const SURF_NODRAW: i32 = 0x0080;
const SURF_HINT: i32 = 0x0100;
const SURF_SKIP: i32 = 0x0200;

impl TexInfo {
    /// Whether this surface is one the renderer draws at all. The sky is
    /// drawn by the sky box rather than as a surface, and the rest are
    /// compiler annotations that exist only in the map source.
    pub fn is_drawn(&self) -> bool {
        self.flags & (SURF_SKY | SURF_NODRAW | SURF_HINT | SURF_SKIP) == 0
    }

    /// The texel coordinate of a world point on this surface.
    pub fn texels(&self, point: [f32; 3]) -> [f32; 2] {
        let project = |vector: [f32; 4]| {
            point[0] * vector[0] + point[1] * vector[1] + point[2] * vector[2] + vector[3]
        };
        [
            project(self.texture_vectors[0]),
            project(self.texture_vectors[1]),
        ]
    }

    /// Where a point on the surface falls on its lightmap, in luxels.
    ///
    /// These are absolute, so a face's own `lightmap_mins` still has to be
    /// subtracted to index the samples it stores.
    pub fn luxels(&self, point: [f32; 3]) -> [f32; 2] {
        let project = |vector: [f32; 4]| {
            point[0] * vector[0] + point[1] * vector[1] + point[2] * vector[2] + vector[3]
        };
        [
            project(self.lightmap_vectors[0]),
            project(self.lightmap_vectors[1]),
        ]
    }
}

/// The material a group of surfaces is drawn with, and the size the texture
/// coordinates above are stated against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexData {
    pub reflectivity: [f32; 3],
    pub name: u32,
    pub width: i32,
    pub height: i32,
    pub view_width: i32,
    pub view_height: i32,
}

/// Indexed triangles for a set of world surfaces.
///
/// The positions are shared between the triangles that meet at them, which is
/// how the format stores them and why this is worth building as an indexed
/// draw rather than an expanded vertex list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Geometry {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl Geometry {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Owned world surface geometry, parsed and cross-checked once so a draw can
/// index it without revalidating.
///
/// Every index the format stores is checked against the lump it points into
/// at parse time. The alternative is checking during traversal, which is the
/// hot path, and a map that fails those checks is not renderable at all.
#[derive(Debug, Clone)]
pub struct Surfaces {
    vertices: Vec<[f32; 3]>,
    edges: Vec<[u16; 2]>,
    surfedges: Vec<i32>,
    faces: Vec<Face>,
    leaf_faces: Vec<u16>,
}

impl Surfaces {
    pub fn parse(bsp: &Bsp<'_>) -> Result<Self> {
        let vertices = parse_vertices(core_lump(bsp, LUMP_VERTEXES)?)?;
        let edges = parse_edges(core_lump(bsp, LUMP_EDGES)?)?;
        let surfedges = parse_surfedges(core_lump(bsp, LUMP_SURFEDGES)?)?;
        let faces = parse_faces(core_lump(bsp, LUMP_FACES)?)?;
        let leaf_faces = parse_leaf_faces(core_lump(bsp, LUMP_LEAFFACES)?)?;

        for (index, edge) in edges.iter().enumerate() {
            for vertex in *edge {
                if usize::from(vertex) >= vertices.len() {
                    return Err(Error::InvalidEdgeVertex {
                        edge: index,
                        vertex,
                    });
                }
            }
        }
        // A surface edge is a signed edge index: the sign carries the
        // direction the edge is walked, so the magnitude is what has to be in
        // range and zero is only valid as a positive.
        for (index, surfedge) in surfedges.iter().enumerate() {
            let magnitude = surfedge.unsigned_abs() as usize;
            if magnitude >= edges.len() {
                return Err(Error::InvalidSurfedge {
                    surfedge: index,
                    edge: *surfedge,
                });
            }
        }
        for (index, face) in faces.iter().enumerate() {
            if face.is_displacement() {
                continue;
            }
            let first =
                usize::try_from(face.first_surfedge).map_err(|_| Error::InvalidFaceRange {
                    face: index,
                    first: face.first_surfedge,
                    count: face.surfedge_count,
                })?;
            let end = first.checked_add(usize::from(face.surfedge_count)).ok_or(
                Error::InvalidFaceRange {
                    face: index,
                    first: face.first_surfedge,
                    count: face.surfedge_count,
                },
            )?;
            if end > surfedges.len() {
                return Err(Error::InvalidFaceRange {
                    face: index,
                    first: face.first_surfedge,
                    count: face.surfedge_count,
                });
            }
        }
        for leaf_face in &leaf_faces {
            if usize::from(*leaf_face) >= faces.len() {
                return Err(Error::InvalidFaceIndex(usize::from(*leaf_face)));
            }
        }

        Ok(Self {
            vertices,
            edges,
            surfedges,
            faces,
            leaf_faces,
        })
    }

    pub fn vertices(&self) -> &[[f32; 3]] {
        &self.vertices
    }

    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    pub fn leaf_faces(&self) -> &[u16] {
        &self.leaf_faces
    }

    /// The vertex indices around `face`, in the order the compiler wound them.
    pub fn face_loop(&self, face: usize) -> Result<Vec<u16>> {
        let entry = self.faces.get(face).ok_or(Error::InvalidFaceIndex(face))?;
        if entry.surfedge_count < 3 {
            return Err(Error::DegenerateFace {
                face,
                edges: entry.surfedge_count,
            });
        }
        let first = entry.first_surfedge as usize;
        let mut loop_vertices = Vec::with_capacity(usize::from(entry.surfedge_count));
        for offset in 0..usize::from(entry.surfedge_count) {
            let surfedge = self.surfedges[first + offset];
            let edge = self.edges[surfedge.unsigned_abs() as usize];
            // A negative surface edge means the edge is traversed backwards,
            // so its second vertex is the one this step arrives at. Ignoring
            // the sign produces a polygon whose edges zig-zag rather than
            // enclose the surface.
            loop_vertices.push(if surfedge >= 0 { edge[0] } else { edge[1] });
        }
        Ok(loop_vertices)
    }

    /// Indexed triangles for `faces`, sharing one position list.
    ///
    /// Each polygon is fanned from its first vertex, which is valid because
    /// the compiler emits convex faces; a concave face would need a general
    /// triangulation and does not occur in the world lump.
    pub fn triangulate<I>(&self, faces: I) -> Result<Geometry>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut geometry = Geometry::default();
        let mut remapped = vec![u32::MAX; self.vertices.len()];
        for face in faces {
            let entry = self.faces.get(face).ok_or(Error::InvalidFaceIndex(face))?;
            // Displacements carry their geometry in their own lump, so the
            // edge loop here describes only the surface they replace.
            if entry.is_displacement() {
                continue;
            }
            let loop_vertices = self.face_loop(face)?;
            let mut fan = Vec::with_capacity(loop_vertices.len());
            for vertex in loop_vertices {
                let slot = usize::from(vertex);
                if remapped[slot] == u32::MAX {
                    let position = self.vertices[slot];
                    if !position.iter().all(|value| value.is_finite()) {
                        return Err(Error::NonFiniteVertex(slot));
                    }
                    remapped[slot] =
                        u32::try_from(geometry.positions.len()).map_err(|_| Error::SizeOverflow)?;
                    geometry.positions.push(position);
                }
                fan.push(remapped[slot]);
            }
            for corner in 1..fan.len() - 1 {
                geometry.indices.push(fan[0]);
                geometry.indices.push(fan[corner]);
                geometry.indices.push(fan[corner + 1]);
            }
        }
        Ok(geometry)
    }

    /// The faces a world draw actually shows, with the material each is
    /// drawn with.
    ///
    /// Shared so that anything working per drawn surface, such as packing
    /// their lightmaps, sees exactly the set that will be triangulated
    /// rather than a filter that has drifted from it.
    fn drawn_faces<I>(
        &self,
        materials: &Materials,
        faces: I,
    ) -> Result<Vec<(usize, usize, usize, TexInfo)>>
    where
        I: IntoIterator<Item = usize>,
    {
        let mut drawn = Vec::new();
        for face in faces {
            let entry = self.faces.get(face).ok_or(Error::InvalidFaceIndex(face))?;
            if entry.is_displacement() {
                continue;
            }
            let Some((texinfo_index, texinfo)) = materials.face_texinfo(entry)? else {
                continue;
            };
            if !texinfo.is_drawn() {
                continue;
            }
            // `Materials::parse` has already established that a drawn
            // texinfo names a texdata the lump holds.
            drawn.push((texinfo.texdata as usize, face, texinfo_index, texinfo));
        }
        Ok(drawn)
    }

    /// Indexed triangles with texture coordinates, grouped per material.
    ///
    /// Positions are only shared between surfaces that also share a texinfo,
    /// because two surfaces meeting at a corner project their textures
    /// differently there and so need different vertices even though they
    /// occupy the same point.
    pub fn triangulate_textured<I>(
        &self,
        materials: &Materials,
        faces: I,
    ) -> Result<TexturedGeometry>
    where
        I: IntoIterator<Item = usize>,
    {
        self.triangulate_lit(materials, None, faces)
    }

    /// The same, with each vertex also carrying where it falls in a packed
    /// lightmap, so the world can be drawn with its baked lighting.
    ///
    /// Supplying an atlas costs vertices: two surfaces meeting at a corner
    /// share it only while they share a texture projection, and they never
    /// share a lightmap block, so the corner has to be stored once per
    /// surface instead of once per projection.
    pub fn triangulate_lit<I>(
        &self,
        materials: &Materials,
        lightmaps: Option<&LightmapAtlas>,
        faces: I,
    ) -> Result<TexturedGeometry>
    where
        I: IntoIterator<Item = usize>,
    {
        self.triangulate_placed(materials, lightmaps, [(Placement::IDENTITY, faces)])
    }

    /// The same, for surfaces that are stored around an origin of their own
    /// and have to be put where the map places them.
    ///
    /// The first group is the world, which the visibility set describes.
    /// Every group after it is a brush model standing wherever its entity
    /// says, and the visibility set has nothing to say about those, because
    /// the leaves list surfaces where they are stored rather than where they
    /// end up.
    pub fn triangulate_placed<G, I>(
        &self,
        materials: &Materials,
        lightmaps: Option<&LightmapAtlas>,
        groups: G,
    ) -> Result<TexturedGeometry>
    where
        G: IntoIterator<Item = (Placement, I)>,
        I: IntoIterator<Item = usize>,
    {
        // Gathered and grouped first, so each material's indices come out in
        // one run and the caller draws once per material.
        let mut placements = Vec::new();
        let mut drawn = Vec::new();
        for (placement, faces) in groups {
            let group = placements.len();
            placements.push(placement);
            for face in self.drawn_faces(materials, faces)? {
                drawn.push((face, group));
            }
        }
        drawn.sort_by_key(|((texdata, face, ..), group)| (*texdata, *group, *face));

        let mut geometry = TexturedGeometry::default();
        // Keyed by the surface as well as the projection when lightmaps are
        // in play, because a corner's lighting comes from the surface it is
        // being drawn as part of, and by the group, because the same stored
        // corner lands somewhere different under each placement.
        let mut shared: std::collections::HashMap<(u16, usize, usize, usize), u32> =
            std::collections::HashMap::new();
        let mut batch: Option<Batch> = None;

        for ((texdata, face, texinfo_index, texinfo), group) in drawn {
            let placement = placements[group];
            match &mut batch {
                Some(open) if open.texdata == texdata => {}
                slot => {
                    if let Some(finished) = slot.take() {
                        geometry.batches.push(finished);
                    }
                    *slot = Some(Batch {
                        texdata,
                        first_index: geometry.indices.len(),
                        index_count: 0,
                    });
                }
            }

            let size = materials.texdatas[texdata];
            let entry = &self.faces[face];
            let block = lightmaps.and_then(|atlas| atlas.placement(face));
            let loop_vertices = self.face_loop(face)?;
            let mut fan = Vec::with_capacity(loop_vertices.len());
            for vertex in loop_vertices {
                let slot = usize::from(vertex);
                // Two surfaces share a corner only when they also share a
                // texinfo, because the same point carries a different
                // texture coordinate under a different projection.
                let shared_key = (
                    vertex,
                    texinfo_index,
                    if lightmaps.is_some() { face } else { 0 },
                    group,
                );
                if let Some(existing) = shared.get(&shared_key) {
                    fan.push(*existing);
                    continue;
                }
                // Both projections are defined against the surface as it is
                // stored, so the transform goes on the position alone: a
                // placed door keeps the texture and lighting it was
                // compiled with wherever it is put.
                let stored = self.vertices[slot];
                if !stored.iter().all(|value| value.is_finite()) {
                    return Err(Error::NonFiniteVertex(slot));
                }
                let position = placement.apply(stored);
                let texels = texinfo.texels(stored);
                let index =
                    u32::try_from(geometry.positions.len()).map_err(|_| Error::SizeOverflow)?;
                geometry.positions.push(position);
                geometry.texcoords.push([
                    texels[0] / size.width as f32,
                    texels[1] / size.height as f32,
                ]);
                if let Some(atlas) = lightmaps {
                    // A surface with no baked lighting still needs a
                    // coordinate, and is pointed at the unwritten first
                    // sample so it draws unlit rather than picking up a
                    // neighbour's light.
                    let coordinate = match block {
                        Some(block) => {
                            let luxels = texinfo.luxels(stored);
                            atlas.coordinates(
                                block,
                                [
                                    luxels[0] - entry.lightmap_mins[0] as f32,
                                    luxels[1] - entry.lightmap_mins[1] as f32,
                                ],
                            )
                        }
                        None => [0.0, 0.0],
                    };
                    geometry.lightmap_coords.push(coordinate);
                }
                shared.insert(shared_key, index);
                fan.push(index);
            }

            let first_index = geometry.indices.len();
            for corner in 1..fan.len() - 1 {
                geometry.indices.push(fan[0]);
                geometry.indices.push(fan[corner]);
                geometry.indices.push(fan[corner + 1]);
            }
            let mut mins = [f32::INFINITY; 3];
            let mut maxs = [f32::NEG_INFINITY; 3];
            for vertex in &fan {
                let point = geometry.positions[*vertex as usize];
                for axis in 0..3 {
                    mins[axis] = mins[axis].min(point[axis]);
                    maxs[axis] = maxs[axis].max(point[axis]);
                }
            }
            geometry.runs.push(FaceRun {
                face,
                group,
                batch: geometry.batches.len(),
                first_index,
                index_count: geometry.indices.len() - first_index,
                mins,
                maxs,
            });
            if let Some(open) = &mut batch {
                open.index_count = geometry.indices.len() - open.first_index;
            }
        }
        if let Some(finished) = batch {
            geometry.batches.push(finished);
        }

        Ok(geometry)
    }
}

/// The materials a map's surfaces are drawn with.
///
/// Three lumps have to agree for a surface to name a material: a texinfo
/// points at a texdata, which points into a table of offsets, which point
/// into one run of concatenated names. Each of those is checked here rather
/// than while drawing, because a map that fails them cannot be drawn at all.
#[derive(Debug, Clone)]
pub struct Materials {
    texinfos: Vec<TexInfo>,
    texdatas: Vec<TexData>,
    names: Vec<String>,
}

impl Materials {
    pub fn parse(bsp: &Bsp<'_>) -> Result<Self> {
        let texinfos = parse_texinfos(core_lump(bsp, LUMP_TEXINFO)?)?;
        let texdatas = parse_texdatas(core_lump(bsp, LUMP_TEXDATA)?)?;
        let offsets = parse_texdata_string_table(core_lump(bsp, LUMP_TEXDATA_STRING_TABLE)?)?;
        let name_bytes = core_lump(bsp, LUMP_TEXDATA_STRING_DATA)?;

        let mut names = Vec::with_capacity(offsets.len());
        for offset in &offsets {
            names.push(read_material_name(name_bytes, *offset)?);
        }

        for (index, texinfo) in texinfos.iter().enumerate() {
            // A negative texdata is how the format spells "no material",
            // which only the surfaces that are never drawn may carry.
            if texinfo.texdata < 0 {
                if texinfo.is_drawn() {
                    return Err(Error::InvalidTexData(texinfo.texdata));
                }
                continue;
            }
            if texinfo.texdata as usize >= texdatas.len() {
                return Err(Error::InvalidTexData(texinfo.texdata));
            }
            if !texinfo.is_drawn() {
                continue;
            }
            // Only the surfaces that are drawn need a usable projection; the
            // rest are allowed to carry whatever the compiler left behind.
            let finite = texinfo
                .texture_vectors
                .iter()
                .flatten()
                .all(|value| value.is_finite());
            let texdata = texdatas[texinfo.texdata as usize];
            if !finite || texdata.width <= 0 || texdata.height <= 0 {
                return Err(Error::UnusableProjection(index));
            }
        }

        for texdata in &texdatas {
            if texdata.name as usize >= names.len() {
                return Err(Error::InvalidMaterialName(texdata.name));
            }
        }

        Ok(Self {
            texinfos,
            texdatas,
            names,
        })
    }

    pub fn texinfos(&self) -> &[TexInfo] {
        &self.texinfos
    }

    pub fn texdatas(&self) -> &[TexData] {
        &self.texdatas
    }

    /// The material path a texdata names, as it appears in the map, which is
    /// what the filesystem resolves under `materials/` with a `.vmt` suffix.
    pub fn name(&self, texdata: usize) -> Option<&str> {
        let entry = self.texdatas.get(texdata)?;
        self.names.get(entry.name as usize).map(String::as_str)
    }

    /// The texinfo a face uses, or `None` where it names none.
    fn face_texinfo(&self, face: &Face) -> Result<Option<(usize, TexInfo)>> {
        if face.texinfo < 0 {
            return Ok(None);
        }
        let index = face.texinfo as usize;
        let texinfo = *self
            .texinfos
            .get(index)
            .ok_or(Error::InvalidTexInfo(face.texinfo))?;
        Ok(Some((index, texinfo)))
    }
}

/// Indexed triangles grouped into one run per material.
///
/// A draw can only bind one texture, so the indices are ordered so that every
/// surface sharing a material is contiguous and the whole world is drawn in
/// as many draws as it has materials rather than as it has surfaces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TexturedGeometry {
    pub positions: Vec<[f32; 3]>,
    /// Normalized against each material's own size, so they are the `0..1`
    /// coordinates a sampler takes rather than the texels the lump stores.
    pub texcoords: Vec<[f32; 2]>,
    /// Where each vertex falls in the packed lightmap, empty when the
    /// geometry was built without one.
    pub lightmap_coords: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    /// Which surface each run of the index buffer came from, in the order
    /// the indices were written.
    ///
    /// This is what lets a frame draw a subset of the world without
    /// rebuilding its buffers: the geometry is uploaded once, and a frame
    /// selects runs out of it.
    pub runs: Vec<FaceRun>,
}

/// One surface's run of the shared index buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceRun {
    pub face: usize,
    /// Which placement group this run was built under: zero for the world,
    /// and one per brush model after it, in the order they were given.
    ///
    /// The visibility set's leaf lists describe the world alone, because
    /// they name a brush model's surfaces where they are stored rather than
    /// where the entity puts them, so a placed run is culled by the leaf its
    /// model stands in instead of by the leaves that list it.
    pub group: usize,
    /// Which of the geometry's batches this run belongs to, and so which
    /// material it is drawn with.
    pub batch: usize,
    pub first_index: usize,
    pub index_count: usize,
    /// The box the run's own vertices fall in.
    ///
    /// This is what a view culls by, rather than the box of the leaf the
    /// surface is listed in. A surface is listed in the leaves it starts
    /// in and reaches well past them: on a shipped map by as much as a
    /// thousand units, which is a wall's worth of hole in a frame.
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
}

/// One material's run of indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Batch {
    pub texdata: usize,
    pub first_index: usize,
    pub index_count: usize,
}

impl TexturedGeometry {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// The runs of the index buffer a frame draws: the surfaces the map's
    /// visibility set reaches, narrowed to those the view can hold.
    ///
    /// The two answer different questions and neither replaces the other.
    /// The visibility set says what is not behind a wall, which is about
    /// the map and settled when it was compiled. The view says what is not
    /// off the edge of the screen, which changes as the camera turns.
    ///
    /// Neighbouring visible surfaces in one material come out as a single
    /// run, because their indices were written next to each other. A frame
    /// that sees the whole map therefore draws exactly the batches the
    /// geometry was grouped into, and one that sees part of it draws the
    /// same triangles in the same order, split where the surfaces between
    /// them were dropped. Preserving the order matters: the depth buffer
    /// settles which surface is in front, but two coplanar surfaces are
    /// settled by which was drawn last.
    pub fn visible_batches(&self, visible: &VisibleFaces, frustum: Option<&Frustum>) -> Vec<Batch> {
        let mut runs: Vec<Batch> = Vec::new();
        for run in &self.runs {
            let reachable = if run.group == 0 {
                visible.contains(run.face)
            } else {
                visible.admits(run.group)
            };
            if run.index_count == 0 || !reachable {
                continue;
            }
            if frustum.is_some_and(|frustum| frustum.excludes(run.mins, run.maxs)) {
                continue;
            }
            let texdata = self.batches[run.batch].texdata;
            match runs.last_mut() {
                Some(open)
                    if open.texdata == texdata
                        && open.first_index + open.index_count == run.first_index =>
                {
                    open.index_count += run.index_count;
                }
                _ => runs.push(Batch {
                    texdata,
                    first_index: run.first_index,
                    index_count: run.index_count,
                }),
            }
        }
        runs
    }
}

/// One of the map's entities: a bag of key and value pairs naming what it
/// is, where it is and what it does.
///
/// A key can appear more than once, which is how an entity lists several
/// outputs firing on the same event, so the pairs are kept in the order the
/// map states them rather than folded into a map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entity {
    pairs: Vec<(String, String)>,
}

impl Entity {
    /// The first value for a key, which is the one the engine reads for
    /// every key but an output.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> {
        self.pairs
            .iter()
            .filter(move |(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn classname(&self) -> Option<&str> {
        self.get("classname")
    }

    /// Where the entity sits, if it says.
    ///
    /// Written as three numbers in one value, which is how every vector
    /// key in a map is written.
    pub fn origin(&self) -> Option<[f32; 3]> {
        self.vector("origin")
    }

    pub fn vector(&self, key: &str) -> Option<[f32; 3]> {
        let mut parts = self.get(key)?.split_ascii_whitespace();
        let mut out = [0.0f32; 3];
        for slot in &mut out {
            *slot = parts.next()?.parse().ok()?;
        }
        parts.next().is_none().then_some(out)
    }

    /// The brush model this entity draws and collides with, if it is one of
    /// the map's own brushes rather than a point in space.
    ///
    /// Written as `*` and the model's number, which is what distinguishes a
    /// door cut from the map from one loaded out of a model file.
    pub fn brush_model(&self) -> Option<usize> {
        self.get("model")?.strip_prefix('*')?.parse().ok()
    }

    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    /// Where this entity stands, as a transform to put its brush model into
    /// the world.
    pub fn placement(&self) -> Placement {
        Placement {
            origin: self.origin().unwrap_or([0.0; 3]),
            angles: self.vector("angles").unwrap_or([0.0; 3]),
        }
    }
}

/// Where a brush model stands in the world.
///
/// A map's own brushes are not all stored where they are seen. The compiler
/// cuts each brush entity out of the world and stores it around an origin of
/// its own, so a door's surfaces sit near zero and the entity that places it
/// carries the position they belong at. Drawing those surfaces as they are
/// stored piles every door, lift and trigger volume in the map onto its
/// origin, so the transform is not optional.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin: [f32; 3],
    /// Pitch, yaw and roll in degrees, which is the order a map writes them
    /// rather than the order they are applied in.
    pub angles: [f32; 3],
}

impl Default for Placement {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Placement {
    /// The world's own placement: it is stored where it stands.
    pub const IDENTITY: Self = Self {
        origin: [0.0; 3],
        angles: [0.0; 3],
    };

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Put a point stored in the model into the world.
    pub fn apply(&self, point: [f32; 3]) -> [f32; 3] {
        let rotated = self.rotate(point);
        std::array::from_fn(|axis| rotated[axis] + self.origin[axis])
    }

    /// Turn a direction the way this placement turns the thing it places,
    /// without moving it, which is what a normal needs: a normal says which
    /// way a surface faces and has no position to move.
    pub fn rotate(&self, vector: [f32; 3]) -> [f32; 3] {
        let [pitch, yaw, roll] = self.angles;
        if pitch == 0.0 && yaw == 0.0 && roll == 0.0 {
            return vector;
        }
        // Yaw about Z, then pitch about Y, then roll about X, which is
        // how the engine reads the three numbers a map writes.
        let (sy, cy) = yaw.to_radians().sin_cos();
        let (sp, cp) = pitch.to_radians().sin_cos();
        let (sr, cr) = roll.to_radians().sin_cos();
        let rows = [
            [cp * cy, sr * sp * cy - cr * sy, cr * sp * cy + sr * sy],
            [cp * sy, sr * sp * sy + cr * cy, cr * sp * sy - sr * cy],
            [-sp, sr * cp, cr * cp],
        ];
        std::array::from_fn(|axis| {
            rows[axis][0] * vector[0] + rows[axis][1] * vector[1] + rows[axis][2] * vector[2]
        })
    }
}

/// Everything the map places in itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entities(Vec<Entity>);

impl Entities {
    pub fn parse(bsp: &Bsp<'_>) -> Result<Self> {
        Self::read(core_lump(bsp, LUMP_ENTITIES)?)
    }

    /// Reads the lump's own text, which is a run of brace-delimited blocks
    /// of quoted key and value pairs.
    ///
    /// The lump is terminated by a nul and often padded past it, so
    /// everything from the first nul on is ignored. Quoted values are taken
    /// literally: the format has no escape, and a value holding a backslash
    /// is a Windows content path rather than the start of one.
    fn read(bytes: &[u8]) -> Result<Self> {
        let text = bytes.split(|byte| *byte == 0).next().unwrap_or(bytes);
        let text = std::str::from_utf8(text).map_err(|_| Error::InvalidEntityText)?;

        let mut entities = Vec::new();
        let mut open: Option<Entity> = None;
        let mut rest = text;
        loop {
            let trimmed = rest.trim_start();
            let Some(next) = trimmed.chars().next() else {
                break;
            };
            match next {
                '{' => {
                    if open.is_some() {
                        return Err(Error::InvalidEntityText);
                    }
                    open = Some(Entity::default());
                    rest = &trimmed[1..];
                }
                '}' => {
                    entities.push(open.take().ok_or(Error::InvalidEntityText)?);
                    rest = &trimmed[1..];
                }
                '"' => {
                    let entity = open.as_mut().ok_or(Error::InvalidEntityText)?;
                    let (key, after) = quoted(trimmed)?;
                    let (value, after) = quoted(after.trim_start())?;
                    entity.pairs.push((key.to_owned(), value.to_owned()));
                    rest = after;
                }
                _ => return Err(Error::InvalidEntityText),
            }
        }
        if open.is_some() {
            return Err(Error::InvalidEntityText);
        }
        Ok(Self(entities))
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Entity> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The map's own settings, which the compiler writes as the first
    /// entity and which names the map's sky and its lighting.
    /// Every brush model the map places, with where it stands.
    ///
    /// Ordered by model so a caller draws them in the order the map stores
    /// them. A model no entity claims is left out: without an entity there
    /// is nothing to say where it goes, and the compiler does not emit one.
    pub fn brush_placements(&self) -> Vec<(usize, Placement)> {
        let mut placed: Vec<(usize, Placement)> = self
            .iter()
            .filter_map(|entity| Some((entity.brush_model()?, entity.placement())))
            .filter(|(model, _)| *model != 0)
            .collect();
        placed.sort_by_key(|(model, _)| *model);
        placed.dedup_by_key(|(model, _)| *model);
        placed
    }

    pub fn worldspawn(&self) -> Option<&Entity> {
        self.0
            .first()
            .filter(|entity| entity.classname() == Some("worldspawn"))
    }

    pub fn by_classname<'a>(&'a self, classname: &'a str) -> impl Iterator<Item = &'a Entity> {
        self.0
            .iter()
            .filter(move |entity| entity.classname() == Some(classname))
    }
}

/// The text inside the leading quote, and what follows the closing one.
fn quoted(text: &str) -> Result<(&str, &str)> {
    let rest = text.strip_prefix('"').ok_or(Error::InvalidEntityText)?;
    let end = rest.find('"').ok_or(Error::InvalidEntityText)?;
    Ok((&rest[..end], &rest[end + 1..]))
}

/// A view's six bounding planes, each `[a, b, c, d]` with a point inside the
/// view where `a*x + b*y + c*z + d` is not negative.
///
/// Taken as data rather than derived here, because the planes have to be the
/// ones the transform being drawn with actually produces. Rebuilding them
/// from an eye and a lens would let the two drift, and a frustum that is
/// narrower than what is drawn removes surfaces that should be on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum([[f32; 4]; 6]);

impl Frustum {
    pub fn new(planes: [[f32; 4]; 6]) -> Self {
        Self(planes)
    }

    pub fn planes(&self) -> &[[f32; 4]; 6] {
        &self.0
    }

    /// Whether a box lies wholly outside the view.
    ///
    /// Called with a surface's own box rather than the box of the leaf it
    /// sits in. A leaf's box is the cheaper test and the one an engine
    /// reaches for first, but it is not sound here: a surface is listed in
    /// the leaves it starts in and reaches past them.
    ///
    /// A box is outside only when one plane has all of it behind, which is
    /// tested by taking the box's corner furthest along each plane's normal:
    /// if even that corner is behind, none of the box is in front. The
    /// converse does not hold, so a box straddling two planes' outsides
    /// without being behind either is kept. That is the usual conservative
    /// answer, and the right way to be wrong: keeping a surface that cannot
    /// be seen costs a little work, dropping one that can leaves a hole.
    pub fn excludes(&self, mins: [f32; 3], maxs: [f32; 3]) -> bool {
        self.0.iter().any(|plane| {
            let furthest: f32 = (0..3)
                .map(|axis| {
                    let extent = if plane[axis] >= 0.0 {
                        maxs[axis]
                    } else {
                        mins[axis]
                    };
                    plane[axis] * extent
                })
                .sum();
            furthest + plane[3] < 0.0
        })
    }
}

/// The surfaces the map's visibility set reaches from somewhere in it.
///
/// A map holds every surface in it, and the compiler worked out ahead of
/// time which parts of it can be seen from where. Drawing all of it wastes
/// most of a frame on surfaces behind walls, which is what the visibility
/// set exists to avoid.
#[derive(Debug, Clone)]
pub struct VisibleFaces {
    seen: Vec<bool>,
    faces: usize,
    /// How many leaves the visibility set admitted, which is what tells a
    /// caller whether it is doing anything at all.
    visible_leaves: usize,
    /// The viewpoint's own row of the visibility set, kept so brush models
    /// can be tested against it after the fact.
    row: Option<Vec<u8>>,
    /// Which placement groups the view admits, empty until a caller says
    /// what it placed, which admits all of them.
    groups: Vec<bool>,
}

impl VisibleFaces {
    /// Selects the surfaces of the world model reachable from a point.
    ///
    /// This speaks for the world model alone, which is what the tree and
    /// the visibility set are built over. The map's other brush models are
    /// doors, lifts and trigger volumes: they sit in no leaf, their
    /// geometry is stored about their own origin rather than in the world,
    /// and the entity owning one says where it is and whether it is drawn
    /// at all. Deciding those needs the entities, so they are left out
    /// here rather than guessed at.
    ///
    /// A point outside the world, in a leaf the compiler gave no cluster,
    /// yields everything rather than nothing: that is where the engine falls
    /// back too, because a camera that has left the map has no visibility
    /// row to read and drawing nothing would be worse than drawing too much.
    pub fn select(world: &World, surfaces: &Surfaces, from: [f32; 3]) -> Result<Self> {
        let faces = surfaces.faces().len();
        let leaf = world.point_leaf(from)?;
        let cluster = world.leaves()[leaf].cluster;

        let row = if cluster < 0 {
            None
        } else {
            Some(world.visibility(cluster as usize, VisibilityKind::PotentiallyVisible)?)
        };

        let mut seen = vec![false; faces];
        let mut count = 0usize;
        let mut visible_leaves = 0usize;
        for entry in world.leaves() {
            if let (Some(row), true) = (&row, entry.cluster >= 0) {
                let cluster = entry.cluster as usize;
                if row[cluster / 8] & (1 << (cluster & 7)) == 0 {
                    continue;
                }
            } else if row.is_some() && entry.cluster < 0 {
                // A leaf with no cluster is solid, and holds no surface the
                // visibility set could reach.
                continue;
            }
            visible_leaves += 1;

            let first = usize::from(entry.first_leaf_face);
            let end = first
                .checked_add(usize::from(entry.leaf_face_count))
                .ok_or(Error::SizeOverflow)?;
            let listed = surfaces
                .leaf_faces()
                .get(first..end)
                .ok_or(Error::InvalidLeafFaceRun { first, end })?;
            for face in listed {
                let face = usize::from(*face);
                let slot = seen.get_mut(face).ok_or(Error::InvalidFaceIndex(face))?;
                if !*slot {
                    *slot = true;
                    count += 1;
                }
            }
        }

        Ok(Self {
            seen,
            faces: count,
            visible_leaves,
            row,
            groups: Vec::new(),
        })
    }

    /// Narrow the brush models a frame draws to those standing somewhere the
    /// view can see.
    ///
    /// Given in the order the geometry was built with, so group one is the
    /// first placement and so on. Without this every placed model is kept
    /// and left to the view frustum, which is correct but draws a map's
    /// worth of doors through walls.
    pub fn placing(mut self, world: &World, placements: &[(usize, Placement)]) -> Self {
        self.groups = vec![true; placements.len() + 1];
        let Some(row) = &self.row else {
            return self;
        };
        for (group, (model, placement)) in placements.iter().enumerate() {
            let Some(entry) = world.models().get(*model) else {
                continue;
            };
            let middle: [f32; 3] =
                std::array::from_fn(|axis| (entry.mins[axis] + entry.maxs[axis]) / 2.0);
            // Where the model ends up, not where it is stored: a door's
            // stored position is inside the map's solid, which sees nothing.
            let standing = placement.apply(middle);
            let cluster = world
                .point_leaf(standing)
                .ok()
                .and_then(|leaf| world.leaves().get(leaf))
                .map(|leaf| leaf.cluster);
            self.groups[group + 1] = match cluster {
                // A model whose middle lands in solid is still drawn: it is
                // most likely a door sitting in its own frame, and dropping
                // it would take a visible surface out of the frame.
                Some(cluster) if cluster >= 0 => {
                    let cluster = cluster as usize;
                    row[cluster / 8] & (1 << (cluster & 7)) != 0
                }
                _ => true,
            };
        }
        self
    }

    /// Whether a placement group has anything to draw in this view.
    pub fn admits(&self, group: usize) -> bool {
        self.groups.get(group).copied().unwrap_or(true)
    }

    /// Every surface in the map, for drawing without culling.
    pub fn everything(surfaces: &Surfaces) -> Self {
        let faces = surfaces.faces().len();
        Self {
            seen: vec![true; faces],
            faces,
            visible_leaves: 0,
            row: None,
            groups: Vec::new(),
        }
    }

    pub fn contains(&self, face: usize) -> bool {
        self.seen.get(face).copied().unwrap_or(false)
    }

    /// How many surfaces were selected.
    pub fn len(&self) -> usize {
        self.faces
    }

    pub fn is_empty(&self) -> bool {
        self.faces == 0
    }

    /// How many surfaces the map holds, selected or not.
    pub fn total(&self) -> usize {
        self.seen.len()
    }

    /// How many leaves the visibility set admitted.
    pub fn visible_leaves(&self) -> usize {
        self.visible_leaves
    }

    pub fn faces(&self) -> impl Iterator<Item = usize> + '_ {
        self.seen
            .iter()
            .enumerate()
            .filter(|(_, seen)| **seen)
            .map(|(face, _)| face)
    }
}

/// The baked lighting a map stores for its world surfaces.
///
/// Each sample is three bytes of colour and a shared exponent, so its linear
/// value is the byte scaled by two to that power. That is how the format
/// holds light brighter than white in a byte, and why a sample cannot simply
/// be copied into an eight-bit texture.
#[derive(Debug, Clone)]
pub struct Lightmaps<'a> {
    samples: &'a [u8],
    high_range: bool,
}

/// Bytes per stored lightmap sample: red, green, blue, exponent.
pub const LIGHTMAP_SAMPLE_SIZE: usize = 4;

impl<'a> Lightmaps<'a> {
    /// Reads a map's baked lighting, preferring the high-range lump where
    /// the map carries one.
    ///
    /// A map compiled for high range stores both, and the two disagree: the
    /// standard-range lump of such a map is the fallback the original
    /// hardware used, so taking it would light the world dimmer than the
    /// map was authored to be.
    pub fn parse(bsp: &Bsp<'a>) -> Result<Self> {
        let high = bsp.lump(LUMP_LIGHTING_HDR).unwrap_or(&[]);
        let (samples, high_range) = if high.is_empty() {
            (bsp.lump(LUMP_LIGHTING).unwrap_or(&[]), false)
        } else {
            (high, true)
        };
        if samples.len() % LIGHTMAP_SAMPLE_SIZE != 0 {
            return Err(Error::InvalidLightingLump(samples.len()));
        }
        Ok(Self {
            samples,
            high_range,
        })
    }

    /// Whether these came from the high-range lump.
    pub fn is_high_range(&self) -> bool {
        self.high_range
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len() / LIGHTMAP_SAMPLE_SIZE
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// One face's samples for its first light style.
    ///
    /// The later styles belong to lights that switch or flicker and need the
    /// runtime to animate them, so only the static layer is read here. A
    /// face with no lightmap at all yields `None`.
    pub fn face_samples(&self, face: &Face) -> Result<Option<&'a [u8]>> {
        if !face.has_lightmap() {
            return Ok(None);
        }
        let count = face
            .lightmap_sample_count()
            .ok_or(Error::InvalidLightmapExtent {
                size: face.lightmap_size,
            })?;
        let start = usize::try_from(face.light_offset)
            .map_err(|_| Error::InvalidLightOffset(face.light_offset))?;
        // The offset is in bytes rather than samples, which is why a face
        // whose offset is not a whole sample is refused rather than read at
        // a shifted alignment.
        if start % LIGHTMAP_SAMPLE_SIZE != 0 {
            return Err(Error::InvalidLightOffset(face.light_offset));
        }
        let length = count
            .checked_mul(LIGHTMAP_SAMPLE_SIZE)
            .ok_or(Error::SizeOverflow)?;
        let end = start.checked_add(length).ok_or(Error::SizeOverflow)?;
        self.samples
            .get(start..end)
            .map(Some)
            .ok_or(Error::LightmapOutOfRange {
                offset: face.light_offset,
                length,
            })
    }
}

/// One face's block of samples within a packed lightmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightmapPlacement {
    /// Where the block's first sample sits in the packed image.
    pub origin: [u32; 2],
    /// How many samples the block holds along each axis.
    pub extent: [u32; 2],
}

/// Every drawn surface's lightmap gathered into one image.
///
/// A map's surfaces each carry a handful of samples, and binding nine
/// thousand tiny textures would cost more than drawing the world. Packing
/// them into one image instead means a surface's lighting is reached by a
/// coordinate, which is what lets the world draw a material at a time.
#[derive(Debug, Clone)]
pub struct LightmapAtlas {
    width: u32,
    height: u32,
    /// Blue-green-red-alpha, which is the one uncompressed layout the
    /// renderer takes.
    pixels: Vec<u8>,
    placements: std::collections::HashMap<usize, LightmapPlacement>,
}

/// The packed image's width. Wide enough that a map's surfaces pack into a
/// few hundred rows, and within the size every Metal device accepts.
const LIGHTMAP_ATLAS_WIDTH: u32 = 2048;
/// The tallest image that will be built before packing is refused, rather
/// than quietly producing one a device may not accept.
const LIGHTMAP_ATLAS_MAX_HEIGHT: u32 = 8192;
/// Samples left between neighbouring blocks, so that filtering across a
/// block's edge cannot reach into an unrelated surface's lighting.
const LIGHTMAP_ATLAS_PADDING: u32 = 1;

impl LightmapAtlas {
    /// Packs the lighting of every surface a world draw shows.
    ///
    /// The faces are filtered exactly as the triangulation filters them, so
    /// each drawn surface has a placement and nothing is packed that is
    /// never drawn.
    pub fn pack<I>(
        surfaces: &Surfaces,
        materials: &Materials,
        lighting: &Lightmaps<'_>,
        faces: I,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = usize>,
    {
        let drawn = surfaces.drawn_faces(materials, faces)?;

        // Packed tallest-first into rows, which leaves far less waste than
        // taking them in face order, where one large block in a row of
        // small ones sets the height of the whole row.
        let mut blocks: Vec<(usize, u32, u32)> = Vec::new();
        for (_, face, ..) in &drawn {
            let entry = &surfaces.faces[*face];
            if lighting.face_samples(entry)?.is_none() {
                continue;
            }
            let extent = [
                u32::try_from(entry.lightmap_size[0] + 1).map_err(|_| {
                    Error::InvalidLightmapExtent {
                        size: entry.lightmap_size,
                    }
                })?,
                u32::try_from(entry.lightmap_size[1] + 1).map_err(|_| {
                    Error::InvalidLightmapExtent {
                        size: entry.lightmap_size,
                    }
                })?,
            ];
            if extent[0] + LIGHTMAP_ATLAS_PADDING > LIGHTMAP_ATLAS_WIDTH {
                return Err(Error::InvalidLightmapExtent {
                    size: entry.lightmap_size,
                });
            }
            blocks.push((*face, extent[0], extent[1]));
        }
        blocks.sort_by_key(|(face, width, height)| {
            (std::cmp::Reverse(*height), std::cmp::Reverse(*width), *face)
        });

        let mut placements = std::collections::HashMap::with_capacity(blocks.len());
        let mut pen = [0u32; 2];
        let mut row_height = 0u32;
        for (face, width, height) in &blocks {
            if pen[0] + width > LIGHTMAP_ATLAS_WIDTH {
                pen[0] = 0;
                pen[1] += row_height + LIGHTMAP_ATLAS_PADDING;
                row_height = 0;
            }
            if pen[1] + height > LIGHTMAP_ATLAS_MAX_HEIGHT {
                return Err(Error::LightmapAtlasTooLarge {
                    blocks: blocks.len(),
                    limit: LIGHTMAP_ATLAS_MAX_HEIGHT,
                });
            }
            placements.insert(
                *face,
                LightmapPlacement {
                    origin: pen,
                    extent: [*width, *height],
                },
            );
            pen[0] += width + LIGHTMAP_ATLAS_PADDING;
            row_height = row_height.max(*height);
        }

        let height = (pen[1] + row_height).max(1);
        let width = LIGHTMAP_ATLAS_WIDTH;
        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];

        for (face, placement) in &placements {
            let entry = &surfaces.faces[*face];
            let samples = lighting
                .face_samples(entry)?
                .expect("only faces with samples were placed");
            for row in 0..placement.extent[1] {
                for column in 0..placement.extent[0] {
                    let sample = (row * placement.extent[0] + column) as usize;
                    let stored = &samples[sample * LIGHTMAP_SAMPLE_SIZE..][..LIGHTMAP_SAMPLE_SIZE];
                    let colour = decode_sample(stored);
                    let x = placement.origin[0] + column;
                    let y = placement.origin[1] + row;
                    let at = ((y as usize) * (width as usize) + x as usize) * 4;
                    // Blue first, matching the renderer's uncompressed
                    // layout and its readback.
                    pixels[at] = colour[2];
                    pixels[at + 1] = colour[1];
                    pixels[at + 2] = colour[0];
                    pixels[at + 3] = 0xff;
                }
            }
        }

        Ok(Self {
            width,
            height,
            pixels,
            placements,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn len(&self) -> usize {
        self.placements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    pub fn placement(&self, face: usize) -> Option<LightmapPlacement> {
        self.placements.get(&face).copied()
    }

    /// Where a point on a face falls in the packed image, as the `0..1`
    /// coordinates a sampler takes.
    ///
    /// The half-sample offset puts the coordinate at a sample's middle
    /// rather than its corner, so that filtering between samples stays
    /// inside the face's own block.
    fn coordinates(&self, placement: LightmapPlacement, local: [f32; 2]) -> [f32; 2] {
        let axis = |value: f32, origin: u32, extent: u32, size: u32| {
            let clamped = value.clamp(0.0, (extent - 1) as f32);
            (origin as f32 + clamped + 0.5) / size as f32
        };
        [
            axis(
                local[0],
                placement.origin[0],
                placement.extent[0],
                self.width,
            ),
            axis(
                local[1],
                placement.origin[1],
                placement.extent[1],
                self.height,
            ),
        ]
    }
}

/// Turns one stored sample into the eight-bit colour a lightmap texture
/// holds, as the engine's own conversion produces it.
///
/// The stored value is a byte scaled by two to the exponent and by a further
/// 1/255, which puts a fully lit white surface at one and leaves room above
/// it: light brighter than white is what makes a lamp read as a lamp. Eight
/// bits cannot hold that, so the engine divides by the overbright before
/// storing and the shader multiplies it back, and the value is written in
/// the screen's gamma rather than linearly because that is where the extra
/// precision is needed.
fn decode_sample(stored: &[u8]) -> [u8; 3] {
    let exponent = stored[3] as i8;
    let scale = (exponent as f32).exp2() / 255.0;
    let mut out = [0u8; 3];
    for channel in 0..3 {
        let linear = f32::from(stored[channel]) * scale;
        let encoded = linear
            .clamp(0.0, LIGHTMAP_OVERBRIGHT)
            .powf(1.0 / LIGHTMAP_GAMMA)
            / LIGHTMAP_OVERBRIGHT;
        out[channel] = (encoded.min(1.0) * 255.0 + 0.5) as u8;
    }
    out
}

/// The screen gamma the engine writes lightmap samples against, which is the
/// floor it clamps the setting to.
const LIGHTMAP_GAMMA: f32 = 2.2;

/// How far past white a stored sample is allowed to reach.
///
/// A packed lightmap holds a sample divided by this, so whatever samples it
/// has to be multiplied by it again. That is the engine's `overbright`, and
/// the two have to agree or the world draws at the wrong brightness.
pub const LIGHTMAP_OVERBRIGHT: f32 = 2.0;

/// Owned collision-world subset used by host-side point and visibility queries.
#[derive(Debug, Clone)]
pub struct World {
    planes: Vec<Plane>,
    nodes: Vec<Node>,
    leaves: Vec<Leaf>,
    cluster_count: usize,
    models: Vec<Model>,
    visibility: Option<Visibility>,
}

impl World {
    pub fn parse(bsp: &Bsp<'_>) -> Result<Self> {
        let planes = parse_planes(core_lump(bsp, LUMP_PLANES)?)?;
        let nodes = parse_nodes(core_lump(bsp, LUMP_NODES)?)?;
        let leaf_version = bsp.header.lumps[LUMP_LEAVES].version;
        let leaves = parse_leaves(core_lump(bsp, LUMP_LEAVES)?, leaf_version)?;
        if planes.is_empty() || leaves.is_empty() {
            return Err(Error::MissingWorldTree);
        }

        for (node_index, node) in nodes.iter().enumerate() {
            if node.plane < 0 || node.plane as usize >= planes.len() {
                return Err(Error::InvalidPlaneIndex {
                    node: node_index,
                    plane: node.plane,
                });
            }
            for child in node.children {
                let valid = if child >= 0 {
                    (child as usize) < nodes.len()
                } else {
                    let leaf = -i64::from(child) - 1;
                    leaf >= 0 && (leaf as usize) < leaves.len()
                };
                if !valid {
                    return Err(Error::InvalidChildIndex {
                        node: node_index,
                        child,
                    });
                }
            }
        }

        let derived_cluster_count = leaves
            .iter()
            .filter_map(|leaf| usize::try_from(leaf.cluster).ok())
            .max()
            .map_or(0, |cluster| cluster + 1);
        let visibility = parse_visibility(core_lump(bsp, LUMP_VISIBILITY)?)?;
        let cluster_count = visibility
            .as_ref()
            .map_or(derived_cluster_count, |vis| vis.offsets.len());
        for leaf in &leaves {
            if leaf.cluster >= 0 && leaf.cluster as usize >= cluster_count {
                return Err(Error::InvalidCluster(i32::from(leaf.cluster)));
            }
        }

        let models = parse_models(core_lump(bsp, LUMP_MODELS)?)?;
        let world = Self {
            planes,
            nodes,
            leaves,
            models,
            cluster_count,
            visibility,
        };
        if world.visibility.is_some() {
            for cluster in 0..world.cluster_count {
                world.visibility(cluster, VisibilityKind::PotentiallyVisible)?;
                world.visibility(cluster, VisibilityKind::PotentiallyAudible)?;
            }
        }
        Ok(world)
    }

    pub fn planes(&self) -> &[Plane] {
        &self.planes
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn leaves(&self) -> &[Leaf] {
        &self.leaves
    }

    pub fn models(&self) -> &[Model] {
        &self.models
    }

    /// The surfaces the world's own tree accounts for.
    ///
    /// Everything outside this belongs to a brush model an entity places,
    /// which the tree says nothing about.
    pub fn world_faces(&self) -> std::ops::Range<usize> {
        self.models.first().map_or(0..0, Model::faces)
    }

    pub fn cluster_count(&self) -> usize {
        self.cluster_count
    }

    pub fn point_leaf(&self, point: [f32; 3]) -> Result<usize> {
        if point.iter().any(|component| !component.is_finite()) {
            return Err(Error::NonFinitePoint);
        }
        if self.nodes.is_empty() {
            return if self.leaves.len() == 1 {
                Ok(0)
            } else {
                Err(Error::MissingWorldTree)
            };
        }
        let mut node_index = 0usize;
        for _ in 0..=self.nodes.len() {
            let node = self.nodes.get(node_index).ok_or(Error::InvalidChildIndex {
                node: node_index,
                child: node_index as i32,
            })?;
            let plane = &self.planes[node.plane as usize];
            let distance = point[0] * plane.normal[0]
                + point[1] * plane.normal[1]
                + point[2] * plane.normal[2]
                - plane.distance;
            let child = node.children[usize::from(distance < 0.0)];
            if child >= 0 {
                node_index = child as usize;
            } else {
                return Ok((-i64::from(child) - 1) as usize);
            }
        }
        Err(Error::TraversalCycle)
    }

    pub fn visibility(&self, cluster: usize, kind: VisibilityKind) -> Result<Vec<u8>> {
        if cluster >= self.cluster_count {
            return Err(Error::InvalidCluster(
                i32::try_from(cluster).unwrap_or(i32::MAX),
            ));
        }
        let row_bytes = self.cluster_count.div_ceil(8);
        let Some(visibility) = &self.visibility else {
            return Ok(vec![0xff; row_bytes]);
        };
        let offset = visibility.offsets[cluster][kind as usize];
        if offset == -1 {
            return Ok(vec![0xff; row_bytes]);
        }
        decode_visibility(&visibility.bytes, offset, row_bytes)
    }

    pub fn cluster_visible(&self, from: usize, to: usize) -> Result<bool> {
        if to >= self.cluster_count {
            return Err(Error::InvalidCluster(i32::try_from(to).unwrap_or(i32::MAX)));
        }
        let row = self.visibility(from, VisibilityKind::PotentiallyVisible)?;
        Ok(row[to / 8] & (1 << (to & 7)) != 0)
    }
}

/// The props a map places once and never moves.
///
/// These are the crates, lamps, railings and signs a level is dressed with,
/// and a map holds far more of them than it holds brush entities. They are
/// not in the entity lump: the compiler moves them into a lump of its own
/// so the engine can load them as one table, keyed by a dictionary of model
/// names the props index rather than each naming its own.
#[derive(Debug, Clone, Default)]
pub struct StaticProps {
    version: i32,
    names: Vec<String>,
    leaves: Vec<u16>,
    props: Vec<StaticProp>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticProp {
    /// Which of the dictionary's model names this prop draws.
    pub model: usize,
    pub origin: [f32; 3],
    /// Pitch, yaw and roll in degrees, as a map writes them.
    pub angles: [f32; 3],
    pub skin: i32,
    /// The leaves this prop stands in, as a run of the shared leaf table,
    /// which is what decides whether a view can see it.
    pub first_leaf: usize,
    pub leaf_count: usize,
}

/// The four bytes naming the static prop lump inside the game lump.
const GAME_LUMP_STATIC_PROPS: i32 = i32::from_be_bytes(*b"sprp");
const GAME_LUMP_ENTRY_SIZE: usize = 16;
/// Bytes one model name takes in the dictionary.
/// The light arriving at a point from each of the six axial directions.
///
/// This is how Source lights everything that is not a world surface. A
/// world surface has a lightmap because the compiler knew where it was and
/// which way it faced; a prop, a player or a thrown crate does not, so the
/// compiler instead records what arrives at points inside each open leaf
/// and leaves the shading to be worked out from whichever way a surface
/// turns out to face.
///
/// The order is the one the compiler writes: `+x, -x, +y, -y, +z, -z`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LightCube {
    pub faces: [[f32; 3]; 6],
}

impl LightCube {
    /// The light arriving on a surface facing `normal`.
    ///
    /// A normal faces at most three of the six directions, and the square
    /// of each component is how much of it each contributes, which sums to
    /// one for a unit normal and so neither brightens nor darkens a
    /// surface for being turned at an angle.
    pub fn shade(&self, normal: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for (axis, component) in normal.iter().enumerate() {
            let face = self.faces[axis * 2 + usize::from(*component < 0.0)];
            let weight = component * component;
            for (channel, value) in out.iter_mut().enumerate() {
                *value += weight * face[channel];
            }
        }
        out
    }

    /// The brightest any direction is, which is what says whether a point
    /// is lit at all.
    pub fn peak(&self) -> f32 {
        self.faces
            .iter()
            .flat_map(|face| face.iter())
            .fold(0.0f32, |peak, value| peak.max(*value))
    }
}

/// One of the compiler's measurements of the light inside a leaf.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbientSample {
    pub cube: LightCube,
    /// Where in the leaf it was taken, as a fraction of the leaf's box on
    /// each axis. The compiler stores this as a byte per axis, which is
    /// enough because it is only ever used to weigh one sample against
    /// another.
    pub fraction: [f32; 3],
}

/// The light inside the map's open leaves, which is what lights everything
/// the compiler could not bake a lightmap for.
#[derive(Debug, Clone, Default)]
pub struct AmbientLighting {
    samples: Vec<AmbientSample>,
    /// Per leaf, where its samples start and how many it has.
    index: Vec<(usize, usize)>,
    hdr: bool,
}

const AMBIENT_SAMPLE_SIZE: usize = 28;
const AMBIENT_INDEX_SIZE: usize = 4;

impl AmbientLighting {
    /// Reads the high-range lumps where the map carries them, as the engine
    /// does for a map built against them, and the standard ones otherwise.
    ///
    /// Falls back to the leaf lump itself, because a map compiled before
    /// the ambient lumps existed carries one cube inside each leaf record
    /// instead, which is what Half-Life 2's own maps do: their version-0
    /// leaves are the version-1 ones with a cube and two bytes of padding
    /// on the end. Reading only the separate lumps would light every prop
    /// in the shipped campaign black.
    pub fn parse(bsp: &Bsp<'_>) -> Result<Self> {
        for (hdr, lighting, index) in [
            (
                true,
                LUMP_LEAF_AMBIENT_LIGHTING_HDR,
                LUMP_LEAF_AMBIENT_INDEX_HDR,
            ),
            (false, LUMP_LEAF_AMBIENT_LIGHTING, LUMP_LEAF_AMBIENT_INDEX),
        ] {
            let lighting = bsp.lump(lighting).unwrap_or(&[]);
            let index = bsp.lump(index).unwrap_or(&[]);
            if lighting.is_empty() || index.is_empty() {
                continue;
            }
            return Self::from_lumps(lighting, index, hdr);
        }
        if bsp.header.lumps[LUMP_LEAVES].version == 0 {
            return Self::from_leaves(bsp.lump(LUMP_LEAVES).unwrap_or(&[]));
        }
        Ok(Self::default())
    }

    /// One sample per leaf, taken from the cube each version-0 leaf record
    /// carries, placed at the middle of the leaf because that record says
    /// nothing about where in the leaf it was measured.
    fn from_leaves(bytes: &[u8]) -> Result<Self> {
        if bytes.len() % LEAF_V0_SIZE != 0 {
            return Err(Error::InvalidLumpLength {
                lump: LUMP_LEAVES,
                length: bytes.len(),
            });
        }
        let mut samples = Vec::with_capacity(bytes.len() / LEAF_V0_SIZE);
        let mut index = Vec::with_capacity(bytes.len() / LEAF_V0_SIZE);
        for (leaf, record) in bytes.chunks_exact(LEAF_V0_SIZE).enumerate() {
            // The cube sits after the thirty bytes a version-1 leaf is,
            // and before the two of padding that end the record.
            let cube = &record[30..54];
            let mut faces = [[0.0f32; 3]; 6];
            for (face, stored) in faces.iter_mut().zip(cube.chunks_exact(4)) {
                *face = decode_light(stored);
            }
            index.push((leaf, 1));
            samples.push(AmbientSample {
                cube: LightCube { faces },
                fraction: [0.5; 3],
            });
        }
        Ok(Self {
            samples,
            index,
            hdr: false,
        })
    }

    fn from_lumps(lighting: &[u8], index: &[u8], hdr: bool) -> Result<Self> {
        if lighting.len() % AMBIENT_SAMPLE_SIZE != 0 {
            return Err(Error::InvalidLumpLength {
                lump: LUMP_LEAF_AMBIENT_LIGHTING,
                length: lighting.len(),
            });
        }
        if index.len() % AMBIENT_INDEX_SIZE != 0 {
            return Err(Error::InvalidLumpLength {
                lump: LUMP_LEAF_AMBIENT_INDEX,
                length: index.len(),
            });
        }

        let mut samples = Vec::with_capacity(lighting.len() / AMBIENT_SAMPLE_SIZE);
        for record in lighting.chunks_exact(AMBIENT_SAMPLE_SIZE) {
            let mut faces = [[0.0f32; 3]; 6];
            for (face, stored) in faces.iter_mut().zip(record.chunks_exact(4)) {
                *face = decode_light(stored);
            }
            samples.push(AmbientSample {
                cube: LightCube { faces },
                fraction: std::array::from_fn(|axis| f32::from(record[24 + axis]) / 255.0),
            });
        }

        let mut entries = Vec::with_capacity(index.len() / AMBIENT_INDEX_SIZE);
        for record in index.chunks_exact(AMBIENT_INDEX_SIZE) {
            let count = usize::from(u16::from_le_bytes([record[0], record[1]]));
            let first = usize::from(u16::from_le_bytes([record[2], record[3]]));
            // A leaf pointing past the samples would silently light
            // everything in it black, so it is refused rather than
            // clamped.
            if first + count > samples.len() {
                return Err(Error::InvalidAmbientRange {
                    first,
                    count,
                    samples: samples.len(),
                });
            }
            entries.push((first, count));
        }

        Ok(Self {
            samples,
            index: entries,
            hdr,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Whether this came from the high-range lumps.
    pub fn is_hdr(&self) -> bool {
        self.hdr
    }

    pub fn samples_in(&self, leaf: usize) -> &[AmbientSample] {
        match self.index.get(leaf) {
            Some(&(first, count)) => &self.samples[first..first + count],
            None => &[],
        }
    }

    /// The light at a point, weighed across the samples of the leaf it is
    /// in.
    ///
    /// Weighing by distance rather than taking the nearest sample, because
    /// a leaf spanning a doorway is measured on both sides of it and a prop
    /// standing in the doorway should not flip from one to the other as it
    /// is nudged. Returns `None` where the point is in solid or in a leaf
    /// the compiler measured nothing in, which is the caller's cue to fall
    /// back rather than to draw black.
    pub fn at(&self, world: &World, point: [f32; 3]) -> Option<LightCube> {
        let leaf_index = world.point_leaf(point).ok()?;
        let leaf = world.leaves().get(leaf_index)?;
        let samples = self.samples_in(leaf_index);
        match samples {
            [] => None,
            [only] => Some(only.cube),
            _ => {
                let size: [f32; 3] = std::array::from_fn(|axis| {
                    (f32::from(leaf.maxs[axis]) - f32::from(leaf.mins[axis])).max(1.0)
                });
                let mut total = 0.0f32;
                let mut cube = LightCube::default();
                for sample in samples {
                    let mut distance = 0.0f32;
                    for axis in 0..3 {
                        let at = f32::from(leaf.mins[axis]) + sample.fraction[axis] * size[axis];
                        distance += (point[axis] - at) * (point[axis] - at);
                    }
                    // A small floor so a point sitting exactly on a sample
                    // does not divide by zero.
                    let weight = 1.0 / distance.max(1.0);
                    total += weight;
                    for (face, from) in cube.faces.iter_mut().zip(&sample.cube.faces) {
                        for (channel, value) in face.iter_mut().enumerate() {
                            *value += weight * from[channel];
                        }
                    }
                }
                for face in &mut cube.faces {
                    for value in face {
                        *value /= total;
                    }
                }
                Some(cube)
            }
        }
    }
}

/// One stored colour and its shared exponent, as the linear light it means.
///
/// The same three bytes and shared exponent the lightmap samples use, but
/// left linear rather than written into the screen's gamma, because this is
/// shaded against a surface's normal first and only what comes out of that
/// is displayed. See [`encode_for_display`].
fn decode_light(stored: &[u8]) -> [f32; 3] {
    let scale = f32::from(stored[3] as i8).exp2();
    std::array::from_fn(|channel| f32::from(stored[channel]) * scale)
}

/// Linear light as the value that multiplies a material to draw it, which
/// is the same overbright and gamma the lightmap samples are stored
/// against so that a prop and the floor it stands on agree.
pub fn encode_for_display(linear: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|channel| {
        linear[channel]
            .clamp(0.0, LIGHTMAP_OVERBRIGHT)
            .powf(1.0 / LIGHTMAP_GAMMA)
            / LIGHTMAP_OVERBRIGHT
    })
}

const STATIC_PROP_NAME_SIZE: usize = 128;
/// The prop record's stride by lump version. Later versions append fields
/// rather than rearranging them, so the leading ones this reads are the
/// same in all of them, but the stride has to be right or every prop after
/// the first is read from the middle of its neighbour.
const STATIC_PROP_SIZES: [(i32, usize); 6] =
    [(4, 56), (5, 60), (6, 64), (7, 64), (8, 68), (10, 72)];

impl StaticProps {
    pub fn parse(bsp: &Bsp<'_>) -> Result<Self> {
        let lump = bsp.lump(LUMP_GAME_LUMP).unwrap_or(&[]);
        if lump.is_empty() {
            return Ok(Self::default());
        }
        let mut reader = Reader::new(lump);
        let count = reader.read_i32_le()?;
        let count = usize::try_from(count).map_err(|_| Error::SizeOverflow)?;

        let mut found = None;
        for entry in 0..count {
            let at = 4 + entry * GAME_LUMP_ENTRY_SIZE;
            let mut reader = Reader::with_position(lump, at)?;
            let id = reader.read_i32_le()?;
            let _flags = reader.read_u16_le()?;
            let version = reader.read_u16_le()? as i32;
            let offset = reader.read_i32_le()?;
            let length = reader.read_i32_le()?;
            if id == GAME_LUMP_STATIC_PROPS {
                found = Some((version, offset, length));
                break;
            }
        }
        let Some((version, offset, length)) = found else {
            return Ok(Self::default());
        };

        // A game lump's offset is into the whole file rather than into the
        // lump holding it, which is the one thing about this lump that
        // catches every reader out.
        let start = usize::try_from(offset).map_err(|_| Error::SizeOverflow)?;
        let length = usize::try_from(length).map_err(|_| Error::SizeOverflow)?;
        let end = start.checked_add(length).ok_or(Error::SizeOverflow)?;
        let bytes = bsp
            .bytes
            .get(start..end)
            .ok_or(Error::InvalidGameLumpRange {
                offset: start,
                length,
            })?;

        let mut reader = Reader::new(bytes);
        let name_count = usize::try_from(reader.read_i32_le()?).map_err(|_| Error::SizeOverflow)?;
        let mut names = Vec::with_capacity(name_count);
        for _ in 0..name_count {
            let raw = reader.take(STATIC_PROP_NAME_SIZE)?;
            let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
            names.push(
                std::str::from_utf8(&raw[..end])
                    .map_err(|_| Error::InvalidEntityText)?
                    .to_string(),
            );
        }

        let leaf_count = usize::try_from(reader.read_i32_le()?).map_err(|_| Error::SizeOverflow)?;
        let mut leaves = Vec::with_capacity(leaf_count);
        for _ in 0..leaf_count {
            leaves.push(reader.read_u16_le()?);
        }

        let prop_count = usize::try_from(reader.read_i32_le()?).map_err(|_| Error::SizeOverflow)?;
        let remaining = bytes.len() - reader.position();
        let stride = STATIC_PROP_SIZES
            .iter()
            .find(|(known, _)| *known == version)
            .map(|(_, size)| *size)
            // A version this does not know is still readable when its props
            // divide the bytes left evenly, which is what says the guess is
            // a stride rather than a coincidence.
            .or_else(|| (prop_count != 0).then(|| remaining / prop_count))
            .unwrap_or(0);
        if prop_count != 0 && (stride < 56 || stride.checked_mul(prop_count) != Some(remaining)) {
            return Err(Error::InvalidStaticPropStride {
                version,
                stride,
                count: prop_count,
                length: remaining,
            });
        }

        let base = reader.position();
        let mut props = Vec::with_capacity(prop_count);
        for index in 0..prop_count {
            let mut reader = Reader::with_position(bytes, base + index * stride)?;
            let mut origin = [0.0f32; 3];
            for axis in &mut origin {
                *axis = reader.read_f32_le()?;
            }
            let mut angles = [0.0f32; 3];
            for axis in &mut angles {
                *axis = reader.read_f32_le()?;
            }
            let model = usize::from(reader.read_u16_le()?);
            let first_leaf = usize::from(reader.read_u16_le()?);
            let leaf_count = usize::from(reader.read_u16_le()?);
            let _solid = reader.read_u8()?;
            let _flags = reader.read_u8()?;
            let skin = reader.read_i32_le()?;
            if model >= names.len() {
                return Err(Error::InvalidStaticPropModel { index, model });
            }
            if first_leaf + leaf_count > leaves.len() {
                return Err(Error::InvalidStaticPropLeaves {
                    index,
                    first: first_leaf,
                    count: leaf_count,
                });
            }
            props.push(StaticProp {
                model,
                origin,
                angles,
                skin,
                first_leaf,
                leaf_count,
            });
        }

        Ok(Self {
            version,
            names,
            leaves,
            props,
        })
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn props(&self) -> &[StaticProp] {
        &self.props
    }

    pub fn is_empty(&self) -> bool {
        self.props.is_empty()
    }

    /// The model a prop draws.
    pub fn name(&self, prop: &StaticProp) -> Option<&str> {
        self.names.get(prop.model).map(String::as_str)
    }

    /// The leaves a prop stands in, which is what a view culls it by.
    pub fn leaves(&self, prop: &StaticProp) -> &[u16] {
        self.leaves
            .get(prop.first_leaf..prop.first_leaf + prop.leaf_count)
            .unwrap_or(&[])
    }

    /// Where a prop stands, as the same transform a brush model is placed
    /// with.
    pub fn placement(prop: &StaticProp) -> Placement {
        Placement {
            origin: prop.origin,
            angles: prop.angles,
        }
    }
}

fn core_lump<'a>(bsp: &Bsp<'a>, index: usize) -> Result<&'a [u8]> {
    if bsp.header.lumps[index].uncompressed_size != 0 {
        return Err(Error::CompressedCoreLump(index));
    }
    Ok(bsp.lump(index).expect("core lump index is valid"))
}

fn record_reader(bytes: &[u8], lump: usize, size: usize) -> Result<Reader<'_>> {
    if bytes.len().checked_rem(size) != Some(0) {
        return Err(Error::InvalidRecordSize {
            lump,
            length: bytes.len(),
            record_size: size,
        });
    }
    Ok(Reader::new(bytes))
}

fn read_i16_vec3(reader: &mut Reader<'_>) -> Result<[i16; 3]> {
    Ok([
        reader.read_i16_le()?,
        reader.read_i16_le()?,
        reader.read_i16_le()?,
    ])
}

fn parse_vertices(bytes: &[u8]) -> Result<Vec<[f32; 3]>> {
    let mut reader = record_reader(bytes, LUMP_VERTEXES, VERTEX_SIZE)?;
    let mut vertices = Vec::with_capacity(bytes.len() / VERTEX_SIZE);
    while reader.remaining() != 0 {
        vertices.push([
            reader.read_f32_le()?,
            reader.read_f32_le()?,
            reader.read_f32_le()?,
        ]);
    }
    Ok(vertices)
}

fn parse_edges(bytes: &[u8]) -> Result<Vec<[u16; 2]>> {
    let mut reader = record_reader(bytes, LUMP_EDGES, EDGE_SIZE)?;
    let mut edges = Vec::with_capacity(bytes.len() / EDGE_SIZE);
    while reader.remaining() != 0 {
        edges.push([reader.read_u16_le()?, reader.read_u16_le()?]);
    }
    Ok(edges)
}

fn parse_surfedges(bytes: &[u8]) -> Result<Vec<i32>> {
    let mut reader = record_reader(bytes, LUMP_SURFEDGES, SURFEDGE_SIZE)?;
    let mut surfedges = Vec::with_capacity(bytes.len() / SURFEDGE_SIZE);
    while reader.remaining() != 0 {
        surfedges.push(reader.read_i32_le()?);
    }
    Ok(surfedges)
}

fn parse_faces(bytes: &[u8]) -> Result<Vec<Face>> {
    let mut reader = record_reader(bytes, LUMP_FACES, FACE_SIZE)?;
    let mut faces = Vec::with_capacity(bytes.len() / FACE_SIZE);
    while reader.remaining() != 0 {
        let plane = reader.read_u16_le()?;
        let side = reader.read_u8()?;
        // `onNode`, which only matters to the compiler.
        let _on_node = reader.read_u8()?;
        let first_surfedge = reader.read_i32_le()?;
        let surfedge_count = reader.read_u16_le()?;
        let texinfo = reader.read_i16_le()?;
        let displacement = reader.read_i16_le()?;
        // The fog volume a water surface bounds, which every other surface
        // reads from its leaf instead.
        let _surface_fog_volume = reader.read_i16_le()?;
        let mut styles = [0u8; 4];
        for style in &mut styles {
            *style = reader.read_u8()?;
        }
        let light_offset = reader.read_i32_le()?;
        // The surface's area, which only the compiler uses.
        let _area = reader.read_f32_le()?;
        let lightmap_mins = [reader.read_i32_le()?, reader.read_i32_le()?];
        let lightmap_size = [reader.read_i32_le()?, reader.read_i32_le()?];
        // The remainder holds the original face this was split from and the
        // primitive and smoothing-group fields, none of which the geometry
        // or its lighting needs.
        let consumed = 2 + 1 + 1 + 4 + 2 + 2 + 2 + 2 + 4 + 4 + 4 + 8 + 8;
        for _ in 0..FACE_SIZE - consumed {
            reader.read_u8()?;
        }
        faces.push(Face {
            plane,
            side,
            first_surfedge,
            surfedge_count,
            texinfo,
            displacement,
            styles,
            light_offset,
            lightmap_mins,
            lightmap_size,
        });
    }
    Ok(faces)
}

fn parse_texinfos(bytes: &[u8]) -> Result<Vec<TexInfo>> {
    let mut reader = record_reader(bytes, LUMP_TEXINFO, TEXINFO_SIZE)?;
    let mut texinfos = Vec::with_capacity(bytes.len() / TEXINFO_SIZE);
    while reader.remaining() != 0 {
        let mut texture_vectors = [[0.0f32; 4]; 2];
        for axis in &mut texture_vectors {
            for component in axis.iter_mut() {
                *component = reader.read_f32_le()?;
            }
        }
        let mut lightmap_vectors = [[0.0f32; 4]; 2];
        for axis in &mut lightmap_vectors {
            for component in axis.iter_mut() {
                *component = reader.read_f32_le()?;
            }
        }
        let flags = reader.read_i32_le()?;
        let texdata = reader.read_i32_le()?;
        texinfos.push(TexInfo {
            texture_vectors,
            lightmap_vectors,
            flags,
            texdata,
        });
    }
    Ok(texinfos)
}

fn parse_texdatas(bytes: &[u8]) -> Result<Vec<TexData>> {
    let mut reader = record_reader(bytes, LUMP_TEXDATA, TEXDATA_SIZE)?;
    let mut texdatas = Vec::with_capacity(bytes.len() / TEXDATA_SIZE);
    while reader.remaining() != 0 {
        let reflectivity = [
            reader.read_f32_le()?,
            reader.read_f32_le()?,
            reader.read_f32_le()?,
        ];
        let name = reader.read_i32_le()?;
        let name = u32::try_from(name).map_err(|_| Error::InvalidMaterialName(name as u32))?;
        texdatas.push(TexData {
            reflectivity,
            name,
            width: reader.read_i32_le()?,
            height: reader.read_i32_le()?,
            view_width: reader.read_i32_le()?,
            view_height: reader.read_i32_le()?,
        });
    }
    Ok(texdatas)
}

fn parse_texdata_string_table(bytes: &[u8]) -> Result<Vec<u32>> {
    let mut reader = record_reader(bytes, LUMP_TEXDATA_STRING_TABLE, TEXDATA_STRING_TABLE_SIZE)?;
    let mut offsets = Vec::with_capacity(bytes.len() / TEXDATA_STRING_TABLE_SIZE);
    while reader.remaining() != 0 {
        let offset = reader.read_i32_le()?;
        offsets.push(u32::try_from(offset).map_err(|_| Error::InvalidMaterialName(offset as u32))?);
    }
    Ok(offsets)
}

/// One NUL-terminated name out of the run of concatenated material paths.
fn read_material_name(bytes: &[u8], offset: u32) -> Result<String> {
    let start = offset as usize;
    let tail = bytes
        .get(start..)
        .ok_or(Error::InvalidMaterialName(offset))?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::InvalidMaterialName(offset))?;
    // Material paths are ASCII in every shipped map, and anything else is a
    // corrupt lump rather than a name to guess at.
    std::str::from_utf8(&tail[..end])
        .map(str::to_owned)
        .map_err(|_| Error::InvalidMaterialName(offset))
}

fn parse_leaf_faces(bytes: &[u8]) -> Result<Vec<u16>> {
    let mut reader = record_reader(bytes, LUMP_LEAFFACES, LEAFFACE_SIZE)?;
    let mut leaf_faces = Vec::with_capacity(bytes.len() / LEAFFACE_SIZE);
    while reader.remaining() != 0 {
        leaf_faces.push(reader.read_u16_le()?);
    }
    Ok(leaf_faces)
}

fn parse_planes(bytes: &[u8]) -> Result<Vec<Plane>> {
    let mut reader = record_reader(bytes, LUMP_PLANES, PLANE_SIZE)?;
    let mut planes = Vec::with_capacity(bytes.len() / PLANE_SIZE);
    while reader.remaining() != 0 {
        planes.push(Plane {
            normal: [
                reader.read_f32_le()?,
                reader.read_f32_le()?,
                reader.read_f32_le()?,
            ],
            distance: reader.read_f32_le()?,
            kind: reader.read_i32_le()?,
        });
    }
    Ok(planes)
}

fn parse_nodes(bytes: &[u8]) -> Result<Vec<Node>> {
    let mut reader = record_reader(bytes, LUMP_NODES, NODE_SIZE)?;
    let mut nodes = Vec::with_capacity(bytes.len() / NODE_SIZE);
    while reader.remaining() != 0 {
        nodes.push(Node {
            plane: reader.read_i32_le()?,
            children: [reader.read_i32_le()?, reader.read_i32_le()?],
            mins: read_i16_vec3(&mut reader)?,
            maxs: read_i16_vec3(&mut reader)?,
            first_face: reader.read_u16_le()?,
            face_count: reader.read_u16_le()?,
            area: reader.read_i16_le()?,
        });
        reader.skip(2)?;
    }
    Ok(nodes)
}

fn parse_leaves(bytes: &[u8], version: i32) -> Result<Vec<Leaf>> {
    let record_size = match version {
        0 => LEAF_V0_SIZE,
        1 => LEAF_V1_SIZE,
        _ => {
            return Err(Error::UnsupportedLumpVersion {
                lump: LUMP_LEAVES,
                version,
            })
        }
    };
    let mut reader = record_reader(bytes, LUMP_LEAVES, record_size)?;
    let mut leaves = Vec::with_capacity(bytes.len() / record_size);
    while reader.remaining() != 0 {
        let contents = reader.read_i32_le()?;
        let cluster = reader.read_i16_le()?;
        let area_and_flags = reader.read_u16_le()?;
        leaves.push(Leaf {
            contents,
            cluster,
            area: area_and_flags & 0x01ff,
            flags: (area_and_flags >> 9) as u8,
            mins: read_i16_vec3(&mut reader)?,
            maxs: read_i16_vec3(&mut reader)?,
            first_leaf_face: reader.read_u16_le()?,
            leaf_face_count: reader.read_u16_le()?,
            first_leaf_brush: reader.read_u16_le()?,
            leaf_brush_count: reader.read_u16_le()?,
            water_data_id: reader.read_i16_le()?,
        });
        reader.skip(2)?;
        if version == 0 {
            reader.skip(24)?;
        }
    }
    Ok(leaves)
}

fn parse_models(bytes: &[u8]) -> Result<Vec<Model>> {
    let mut reader = record_reader(bytes, LUMP_MODELS, MODEL_SIZE)?;
    let mut models = Vec::with_capacity(bytes.len() / MODEL_SIZE);
    while !reader.is_empty() {
        let mut bounds = [[0.0f32; 3]; 3];
        for corner in &mut bounds {
            for value in corner.iter_mut() {
                *value = reader.read_f32_le()?;
            }
        }
        models.push(Model {
            mins: bounds[0],
            maxs: bounds[1],
            origin: bounds[2],
            head_node: reader.read_i32_le()?,
            first_face: reader.read_i32_le()?,
            face_count: reader.read_i32_le()?,
        });
    }
    Ok(models)
}

fn parse_visibility(bytes: &[u8]) -> Result<Option<Visibility>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let mut reader = Reader::new(bytes);
    let cluster_count = reader.read_i32_le()?;
    let cluster_count =
        usize::try_from(cluster_count).map_err(|_| Error::InvalidVisibilityHeader)?;
    let table_size = cluster_count
        .checked_mul(8)
        .and_then(|size| size.checked_add(4))
        .ok_or(Error::InvalidVisibilityHeader)?;
    if table_size > bytes.len() {
        return Err(Error::InvalidVisibilityHeader);
    }
    let mut offsets = Vec::with_capacity(cluster_count);
    for _ in 0..cluster_count {
        let pair = [reader.read_i32_le()?, reader.read_i32_le()?];
        for offset in pair {
            if offset != -1
                && (offset < 0
                    || (offset as usize) < table_size
                    || (offset as usize) >= bytes.len())
            {
                return Err(Error::InvalidVisibilityOffset(offset));
            }
        }
        offsets.push(pair);
    }
    Ok(Some(Visibility {
        offsets,
        bytes: bytes.to_vec(),
    }))
}

fn decode_visibility(bytes: &[u8], offset: i32, row_bytes: usize) -> Result<Vec<u8>> {
    let mut position =
        usize::try_from(offset).map_err(|_| Error::InvalidVisibilityOffset(offset))?;
    let mut output = Vec::with_capacity(row_bytes);
    while output.len() < row_bytes {
        let byte = *bytes.get(position).ok_or(Error::InvalidVisibilityRun)?;
        position += 1;
        if byte != 0 {
            output.push(byte);
            continue;
        }
        let count = *bytes.get(position).ok_or(Error::InvalidVisibilityRun)? as usize;
        position += 1;
        if count == 0 || count > row_bytes - output.len() {
            return Err(Error::InvalidVisibilityRun);
        }
        output.resize(output.len() + count, 0);
    }
    Ok(output)
}

impl std::error::Error for Error {}

impl From<source_binary::Error> for Error {
    fn from(value: source_binary::Error) -> Self {
        Self::Binary(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LumpHeader {
    pub offset: i32,
    pub length: i32,
    pub version: i32,
    /// Original byte size for a compressed lump, or zero when uncompressed.
    pub uncompressed_size: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: i32,
    pub lumps: [LumpHeader; BSP_LUMP_COUNT],
    pub map_revision: i32,
}

#[derive(Debug)]
pub struct Bsp<'a> {
    header: Header,
    bytes: &'a [u8],
}

impl<'a> Bsp<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        if bytes.len() > limits.max_file_size {
            return Err(Error::FileTooLarge(bytes.len()));
        }
        let mut reader = Reader::new(bytes);
        let ident = reader.read_u32_le()?;
        if ident != BSP_IDENT {
            return Err(Error::InvalidIdent(ident));
        }
        let version = reader.read_i32_le()?;
        if !(19..=21).contains(&version) {
            return Err(Error::UnsupportedVersion(version));
        }
        let mut lumps = [LumpHeader::default(); BSP_LUMP_COUNT];
        for (index, lump) in lumps.iter_mut().enumerate() {
            *lump = LumpHeader {
                offset: reader.read_i32_le()?,
                length: reader.read_i32_le()?,
                version: reader.read_i32_le()?,
                uncompressed_size: reader.read_i32_le()?,
            };
            validate_lump(bytes, limits, index, *lump)?;
        }
        let map_revision = reader.read_i32_le()?;
        Ok(Self {
            header: Header {
                version,
                lumps,
                map_revision,
            },
            bytes,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn lump(&self, index: usize) -> Option<&'a [u8]> {
        let lump = self.header.lumps.get(index)?;
        if lump.length == 0 {
            return Some(&[]);
        }
        let start = lump.offset as usize;
        Some(&self.bytes[start..start + lump.length as usize])
    }

    /// The bytes of the archive the map embeds, empty where it embeds
    /// nothing.
    ///
    /// A map's own materials live only here, so its world cannot be textured
    /// without reading this and searching it ahead of the game's archives.
    pub fn pakfile(&self) -> &'a [u8] {
        self.lump(LUMP_PAKFILE).unwrap_or(&[])
    }

    pub fn to_builder(&self) -> Builder {
        let mut builder = Builder::new(self.header.version, self.header.map_revision);
        for (index, header) in self.header.lumps.iter().copied().enumerate() {
            builder.set_lump(
                index,
                header.version,
                header.uncompressed_size,
                self.lump(index).unwrap().to_vec(),
            );
        }
        builder
    }
}

fn validate_lump(bytes: &[u8], limits: Limits, index: usize, lump: LumpHeader) -> Result<()> {
    if lump.offset < 0 || lump.length < 0 {
        return Err(Error::InvalidLump {
            index,
            offset: lump.offset,
            length: lump.length,
        });
    }
    let offset = lump.offset as usize;
    let length = lump.length as usize;
    if length > limits.max_lump_size {
        return Err(Error::LumpTooLarge { index, length });
    }
    if length == 0 {
        return Ok(());
    }
    let end = offset.checked_add(length).ok_or(Error::InvalidLump {
        index,
        offset: lump.offset,
        length: lump.length,
    })?;
    if offset < BSP_HEADER_SIZE || end > bytes.len() {
        return Err(Error::InvalidLump {
            index,
            offset: lump.offset,
            length: lump.length,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct LumpData {
    version: i32,
    uncompressed_size: i32,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Builder {
    version: i32,
    map_revision: i32,
    lumps: Vec<LumpData>,
}

impl Builder {
    pub fn new(version: i32, map_revision: i32) -> Self {
        Self {
            version,
            map_revision,
            lumps: vec![LumpData::default(); BSP_LUMP_COUNT],
        }
    }

    pub fn set_lump(&mut self, index: usize, version: i32, uncompressed_size: i32, bytes: Vec<u8>) {
        assert!(index < BSP_LUMP_COUNT, "BSP lump index out of range");
        self.lumps[index] = LumpData {
            version,
            uncompressed_size,
            bytes,
        };
    }

    pub fn build(&self) -> Result<Vec<u8>> {
        if !(19..=21).contains(&self.version) {
            return Err(Error::UnsupportedVersion(self.version));
        }
        let mut output = Writer::with_capacity(BSP_HEADER_SIZE);
        output.write_u32_le(BSP_IDENT);
        output.write_i32_le(self.version);
        for lump in &self.lumps {
            output.write_i32_le(0);
            output.write_i32_le(0);
            output.write_i32_le(lump.version);
            output.write_i32_le(lump.uncompressed_size);
        }
        output.write_i32_le(self.map_revision);

        for (index, lump) in self.lumps.iter().enumerate() {
            if lump.bytes.is_empty() {
                continue;
            }
            output.align(4)?;
            let offset = u32::try_from(output.position()).map_err(|_| Error::SizeOverflow)?;
            let length = u32::try_from(lump.bytes.len()).map_err(|_| Error::SizeOverflow)?;
            let header_offset = 8 + index * 16;
            output.patch_u32_le(header_offset, offset)?;
            output.patch_u32_le(header_offset + 4, length)?;
            output.write_bytes(&lump.bytes);
        }
        Ok(output.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One leaf as the format stores it, so a test can state only the
    /// fields it is about.
    #[derive(Debug, Clone, Copy, Default)]
    struct LeafFields {
        cluster: i16,
        mins: [i16; 3],
        maxs: [i16; 3],
        first_leaf_face: u16,
        leaf_face_count: u16,
    }

    fn write_leaf(writer: &mut Writer, leaf: LeafFields) {
        writer.write_i32_le(0);
        writer.write_i16_le(leaf.cluster);
        writer.write_u16_le(0);
        for value in leaf.mins.into_iter().chain(leaf.maxs) {
            writer.write_i16_le(value);
        }
        writer.write_u16_le(leaf.first_leaf_face);
        writer.write_u16_le(leaf.leaf_face_count);
        for _ in 0..2 {
            writer.write_u16_le(0);
        }
        writer.write_i16_le(-1);
        writer.write_u16_le(0);
    }

    fn synthetic_world(visibility_bytes: Option<&[u8]>) -> Vec<u8> {
        let mut planes = Writer::new();
        planes.write_f32_le(1.0);
        planes.write_f32_le(0.0);
        planes.write_f32_le(0.0);
        planes.write_f32_le(0.0);
        planes.write_i32_le(0);

        let mut nodes = Writer::new();
        nodes.write_i32_le(0);
        nodes.write_i32_le(-1);
        nodes.write_i32_le(-2);
        for _ in 0..6 {
            nodes.write_i16_le(0);
        }
        nodes.write_u16_le(0);
        nodes.write_u16_le(0);
        nodes.write_i16_le(0);
        nodes.write_u16_le(0);

        let mut leaves = Writer::new();
        write_leaf(
            &mut leaves,
            LeafFields {
                cluster: 0,
                ..Default::default()
            },
        );
        write_leaf(
            &mut leaves,
            LeafFields {
                cluster: 1,
                ..Default::default()
            },
        );

        let mut builder = Builder::new(20, 1);
        builder.set_lump(LUMP_PLANES, 0, 0, planes.into_inner());
        builder.set_lump(LUMP_NODES, 0, 0, nodes.into_inner());
        builder.set_lump(LUMP_LEAVES, 1, 0, leaves.into_inner());
        if let Some(bytes) = visibility_bytes {
            builder.set_lump(LUMP_VISIBILITY, 0, 0, bytes.to_vec());
        }
        builder.build().unwrap()
    }

    /// Writes one face record, padding out the fields the geometry ignores.
    /// One face as the format stores it, so a test can state only the
    /// fields it is about.
    ///
    /// The defaults are what the compiler writes for an ordinary unlit
    /// surface, which includes a fog volume of negative one: that field sits
    /// immediately after the displacement, and a writer that left it zero
    /// would hide a reader that ran the two together.
    #[derive(Debug, Clone, Copy)]
    struct FaceFields {
        plane: u16,
        first_surfedge: i32,
        surfedge_count: u16,
        texinfo: i16,
        displacement: i16,
        surface_fog_volume: i16,
        styles: [u8; 4],
        light_offset: i32,
        lightmap_mins: [i32; 2],
        lightmap_size: [i32; 2],
    }

    impl Default for FaceFields {
        fn default() -> Self {
            Self {
                plane: 0,
                first_surfedge: 0,
                surfedge_count: 4,
                texinfo: 0,
                displacement: -1,
                surface_fog_volume: -1,
                styles: [LIGHT_STYLE_NONE; 4],
                light_offset: -1,
                lightmap_mins: [0, 0],
                lightmap_size: [0, 0],
            }
        }
    }

    fn write_face_fields(writer: &mut Writer, face: FaceFields) {
        writer.write_u16_le(face.plane);
        // `side` and `onNode`.
        writer.write_u8(0);
        writer.write_u8(0);
        writer.write_i32_le(face.first_surfedge);
        writer.write_u16_le(face.surfedge_count);
        writer.write_i16_le(face.texinfo);
        writer.write_i16_le(face.displacement);
        writer.write_i16_le(face.surface_fog_volume);
        for style in face.styles {
            writer.write_u8(style);
        }
        writer.write_i32_le(face.light_offset);
        // `area`.
        writer.write_f32_le(0.0);
        for axis in face.lightmap_mins {
            writer.write_i32_le(axis);
        }
        for axis in face.lightmap_size {
            writer.write_i32_le(axis);
        }
        // The original face, primitive fields and smoothing groups.
        for _ in 0..FACE_SIZE - 44 {
            writer.write_u8(0);
        }
    }

    fn write_face(
        writer: &mut Writer,
        plane: u16,
        first_surfedge: i32,
        surfedge_count: u16,
        displacement: i16,
    ) {
        write_face_with_texinfo(
            writer,
            plane,
            first_surfedge,
            surfedge_count,
            displacement,
            0,
        );
    }

    /// As above, but naming which texinfo the surface is textured by.
    fn write_face_with_texinfo(
        writer: &mut Writer,
        plane: u16,
        first_surfedge: i32,
        surfedge_count: u16,
        displacement: i16,
        texinfo: i16,
    ) {
        write_face_fields(
            writer,
            FaceFields {
                plane,
                first_surfedge,
                surfedge_count,
                texinfo,
                displacement,
                ..FaceFields::default()
            },
        );
    }

    /// A square built the way the format builds one: four shared corners,
    /// four edges, and a run of surface edges walking them around the loop.
    /// The last surface edge is negative so the reversed-edge path is
    /// exercised rather than only the forward one.
    fn synthetic_surfaces(
        surfedge_overrides: Option<Vec<i32>>,
        faces_bytes: Option<Vec<u8>>,
        leaf_faces: Option<Vec<u16>>,
    ) -> Vec<u8> {
        let mut vertices = Writer::new();
        for corner in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            for value in corner {
                vertices.write_f32_le(value);
            }
        }

        let mut edges = Writer::new();
        for edge in [[0u16, 1u16], [1, 2], [2, 3], [0, 3]] {
            edges.write_u16_le(edge[0]);
            edges.write_u16_le(edge[1]);
        }

        let mut surfedges = Writer::new();
        // Edge three is stored 0->3 but the loop needs 3->0, so it is walked
        // backwards.
        for surfedge in surfedge_overrides.unwrap_or_else(|| vec![0, 1, 2, -3]) {
            surfedges.write_i32_le(surfedge);
        }

        let faces = faces_bytes.unwrap_or_else(|| {
            let mut writer = Writer::new();
            write_face(&mut writer, 0, 0, 4, -1);
            writer.into_inner()
        });

        let mut leaf_face_bytes = Writer::new();
        for leaf_face in leaf_faces.unwrap_or_else(|| vec![0]) {
            leaf_face_bytes.write_u16_le(leaf_face);
        }

        let mut builder = Builder::new(20, 1);
        builder.set_lump(LUMP_VERTEXES, 0, 0, vertices.into_inner());
        builder.set_lump(LUMP_EDGES, 0, 0, edges.into_inner());
        builder.set_lump(LUMP_SURFEDGES, 0, 0, surfedges.into_inner());
        builder.set_lump(LUMP_FACES, 0, 0, faces);
        builder.set_lump(LUMP_LEAFFACES, 0, 0, leaf_face_bytes.into_inner());
        builder.build().unwrap()
    }

    /// One texinfo: a texture laid across the square a texel per unit, with
    /// the given flags and texdata.
    fn write_texinfo(writer: &mut Writer, scale: f32, flags: i32, texdata: i32) {
        write_texinfo_with_lightmap(writer, scale, 0.0, flags, texdata);
    }

    /// As above, with the lightmap laid across the surface at its own scale,
    /// which is coarser than the texture's in every real map.
    fn write_texinfo_with_lightmap(
        writer: &mut Writer,
        scale: f32,
        luxel_scale: f32,
        flags: i32,
        texdata: i32,
    ) {
        for projection in [scale, luxel_scale] {
            for axis in [[projection, 0.0, 0.0, 0.0], [0.0, projection, 0.0, 0.0]] {
                for value in axis {
                    writer.write_f32_le(value);
                }
            }
        }
        writer.write_i32_le(flags);
        writer.write_i32_le(texdata);
    }

    fn write_texdata(writer: &mut Writer, name: i32, width: i32, height: i32) {
        for value in [0.5f32, 0.5, 0.5] {
            writer.write_f32_le(value);
        }
        writer.write_i32_le(name);
        writer.write_i32_le(width);
        writer.write_i32_le(height);
        writer.write_i32_le(width);
        writer.write_i32_le(height);
    }

    /// A map whose surfaces name materials, built from the same square the
    /// geometry tests use.
    fn synthetic_materials(
        faces_bytes: Option<Vec<u8>>,
        texinfos: Option<Vec<u8>>,
        texdatas: Option<Vec<u8>>,
        names: Option<(Vec<i32>, Vec<u8>)>,
    ) -> Vec<u8> {
        let surfaces = synthetic_surfaces(None, faces_bytes, None);
        let bsp = Bsp::parse(&surfaces).unwrap();

        let texinfos = texinfos.unwrap_or_else(|| {
            let mut writer = Writer::new();
            write_texinfo(&mut writer, 1.0, 0, 0);
            writer.into_inner()
        });
        let texdatas = texdatas.unwrap_or_else(|| {
            let mut writer = Writer::new();
            write_texdata(&mut writer, 0, 64, 64);
            writer.into_inner()
        });
        let (offsets, name_bytes) =
            names.unwrap_or_else(|| (vec![0], b"brick/brickwall001a\0".to_vec()));

        let mut table = Writer::new();
        for offset in offsets {
            table.write_i32_le(offset);
        }

        let mut builder = Builder::new(20, 1);
        for lump in [
            LUMP_VERTEXES,
            LUMP_EDGES,
            LUMP_SURFEDGES,
            LUMP_FACES,
            LUMP_LEAFFACES,
        ] {
            builder.set_lump(lump, 0, 0, bsp.lump(lump).unwrap().to_vec());
        }
        builder.set_lump(LUMP_TEXINFO, 0, 0, texinfos);
        builder.set_lump(LUMP_TEXDATA, 0, 0, texdatas);
        builder.set_lump(LUMP_TEXDATA_STRING_TABLE, 0, 0, table.into_inner());
        builder.set_lump(LUMP_TEXDATA_STRING_DATA, 0, 0, name_bytes);
        builder.build().unwrap()
    }

    /// Two rooms either side of the `x` plane, so a test can ask what is
    /// visible from one of them.
    ///
    /// The near room holds two surfaces and the far one holds a third, all
    /// in one material, so the surfaces a frame keeps can be told apart by
    /// which room they are in and the runs of the index buffer they make
    /// can be told apart from the batch they came from. The near room's
    /// visibility reaches only itself, which is the wall between the two
    /// that the whole exercise is about. A fourth surface belongs to a
    /// second brush model, a door, and so sits in no leaf at all.
    fn synthetic_rooms() -> Vec<u8> {
        // Four squares, laid flat, in the order the faces name them.
        let squares = [[0.0f32, 0.0], [0.0, 64.0], [-64.0, 0.0], [-64.0, 64.0]];
        let mut vertices = Writer::new();
        let mut edges = Writer::new();
        let mut surfedges = Writer::new();
        let mut faces = Writer::new();
        for (square, [x, y]) in squares.into_iter().enumerate() {
            for corner in [[x, y], [x + 64.0, y], [x + 64.0, y + 64.0], [x, y + 64.0]] {
                for value in [corner[0], corner[1], 0.0] {
                    vertices.write_f32_le(value);
                }
            }
            let base = square as u16 * 4;
            for edge in [
                [base, base + 1],
                [base + 1, base + 2],
                [base + 2, base + 3],
                [base, base + 3],
            ] {
                edges.write_u16_le(edge[0]);
                edges.write_u16_le(edge[1]);
            }
            let base = square as i32 * 4;
            for surfedge in [base, base + 1, base + 2, -(base + 3)] {
                surfedges.write_i32_le(surfedge);
            }
            write_face_fields(
                &mut faces,
                FaceFields {
                    first_surfedge: base,
                    ..Default::default()
                },
            );
        }

        let mut planes = Writer::new();
        for value in [1.0f32, 0.0, 0.0, 0.0] {
            planes.write_f32_le(value);
        }
        planes.write_i32_le(0);

        let mut nodes = Writer::new();
        nodes.write_i32_le(0);
        nodes.write_i32_le(-1);
        nodes.write_i32_le(-2);
        for _ in 0..6 {
            nodes.write_i16_le(0);
        }
        for _ in 0..2 {
            nodes.write_u16_le(0);
        }
        nodes.write_i16_le(0);
        nodes.write_u16_le(0);

        let mut leaves = Writer::new();
        write_leaf(
            &mut leaves,
            LeafFields {
                cluster: 0,
                mins: [0, 0, -1],
                maxs: [64, 128, 1],
                first_leaf_face: 0,
                leaf_face_count: 2,
            },
        );
        write_leaf(
            &mut leaves,
            LeafFields {
                cluster: 1,
                mins: [-64, 0, -1],
                maxs: [0, 64, 1],
                first_leaf_face: 2,
                leaf_face_count: 1,
            },
        );

        let mut leaf_faces = Writer::new();
        for face in [0u16, 1, 2] {
            leaf_faces.write_u16_le(face);
        }

        // Two clusters, each with a visible and an audible row, then the
        // rows themselves: the near room reaches only itself and the far
        // one reaches both.
        let mut visibility = Writer::new();
        visibility.write_i32_le(2);
        for offset in [20, 21, 22, 23] {
            visibility.write_i32_le(offset);
        }
        visibility.write_bytes(&[0b01, 0b11, 0b11, 0b11]);

        // The world model, then the door, which is what puts the fourth
        // surface outside the tree's reach.
        let mut models = Writer::new();
        for (first_face, face_count) in [(0, 3), (3, 1)] {
            for value in [-64.0f32, 0.0, -1.0, 64.0, 128.0, 1.0, 0.0, 0.0, 0.0] {
                models.write_f32_le(value);
            }
            models.write_i32_le(0);
            models.write_i32_le(first_face);
            models.write_i32_le(face_count);
        }

        let mut texinfos = Writer::new();
        write_texinfo(&mut texinfos, 1.0, 0, 0);
        let mut texdatas = Writer::new();
        write_texdata(&mut texdatas, 0, 64, 64);
        let mut table = Writer::new();
        table.write_i32_le(0);

        let mut builder = Builder::new(20, 1);
        builder.set_lump(LUMP_MODELS, 0, 0, models.into_inner());
        builder.set_lump(LUMP_VERTEXES, 0, 0, vertices.into_inner());
        builder.set_lump(LUMP_EDGES, 0, 0, edges.into_inner());
        builder.set_lump(LUMP_SURFEDGES, 0, 0, surfedges.into_inner());
        builder.set_lump(LUMP_FACES, 0, 0, faces.into_inner());
        builder.set_lump(LUMP_LEAFFACES, 0, 0, leaf_faces.into_inner());
        builder.set_lump(LUMP_PLANES, 0, 0, planes.into_inner());
        builder.set_lump(LUMP_NODES, 0, 0, nodes.into_inner());
        builder.set_lump(LUMP_LEAVES, 1, 0, leaves.into_inner());
        builder.set_lump(LUMP_VISIBILITY, 0, 0, visibility.into_inner());
        builder.set_lump(LUMP_TEXINFO, 0, 0, texinfos.into_inner());
        builder.set_lump(LUMP_TEXDATA, 0, 0, texdatas.into_inner());
        builder.set_lump(LUMP_TEXDATA_STRING_TABLE, 0, 0, table.into_inner());
        builder.set_lump(
            LUMP_TEXDATA_STRING_DATA,
            0,
            0,
            b"brick/brickwall001a\0".to_vec(),
        );
        builder.build().unwrap()
    }

    /// A map whose lit surfaces carry the given samples.
    ///
    /// The square the other tests use, with a lightmap projection coarse
    /// enough that the whole surface fits a few luxels, which is the scale
    /// a real map's lightmaps are at.
    fn synthetic_lit(faces: Vec<FaceFields>, lighting: Vec<u8>, high_range: bool) -> Vec<u8> {
        let mut face_bytes = Writer::new();
        for face in faces {
            write_face_fields(&mut face_bytes, face);
        }
        let mut texinfos = Writer::new();
        write_texinfo_with_lightmap(&mut texinfos, 1.0, 2.0, 0, 0);
        let map = synthetic_materials(
            Some(face_bytes.into_inner()),
            Some(texinfos.into_inner()),
            None,
            None,
        );

        let bsp = Bsp::parse(&map).unwrap();
        let mut builder = Builder::new(20, 1);
        for lump in [
            LUMP_VERTEXES,
            LUMP_EDGES,
            LUMP_SURFEDGES,
            LUMP_FACES,
            LUMP_LEAFFACES,
            LUMP_TEXINFO,
            LUMP_TEXDATA,
            LUMP_TEXDATA_STRING_TABLE,
            LUMP_TEXDATA_STRING_DATA,
        ] {
            if let Some(bytes) = bsp.lump(lump) {
                builder.set_lump(lump, 0, 0, bytes.to_vec());
            }
        }
        let which = if high_range {
            LUMP_LIGHTING_HDR
        } else {
            LUMP_LIGHTING
        };
        builder.set_lump(which, 0, 0, lighting);
        builder.build().unwrap()
    }

    /// One stored sample: colour and the exponent it is scaled by.
    fn sample(colour: [u8; 3], exponent: i8) -> [u8; 4] {
        [colour[0], colour[1], colour[2], exponent as u8]
    }

    fn samples(of: &[[u8; 4]]) -> Vec<u8> {
        of.iter().flatten().copied().collect()
    }

    /// A lit face covering the square, with a lightmap of the given extent
    /// starting at the given offset into the lighting lump.
    fn lit_face(light_offset: i32, size: [i32; 2]) -> FaceFields {
        FaceFields {
            styles: [0, LIGHT_STYLE_NONE, LIGHT_STYLE_NONE, LIGHT_STYLE_NONE],
            light_offset,
            lightmap_size: size,
            ..FaceFields::default()
        }
    }

    #[test]
    fn reads_a_displacement_without_the_field_stored_beside_it() {
        // The displacement is sixteen bits and the fog volume follows it, so
        // a reader taking the pair as one number reports the compiler's
        // ordinary surface as a displacement and its real displacement as
        // none, depending which way round the halves fall.
        let mut faces = Writer::new();
        write_face_fields(
            &mut faces,
            FaceFields {
                displacement: -1,
                surface_fog_volume: 0,
                ..FaceFields::default()
            },
        );
        write_face_fields(
            &mut faces,
            FaceFields {
                displacement: 3,
                surface_fog_volume: -1,
                ..FaceFields::default()
            },
        );
        let map = synthetic_surfaces(None, Some(faces.into_inner()), None);
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();

        assert_eq!(surfaces.faces()[0].displacement, -1);
        assert!(!surfaces.faces()[0].is_displacement());
        assert_eq!(surfaces.faces()[1].displacement, 3);
        assert!(surfaces.faces()[1].is_displacement());
    }

    #[test]
    fn reads_a_faces_lightmap_extent_and_styles() {
        let mut faces = Writer::new();
        write_face_fields(
            &mut faces,
            FaceFields {
                styles: [0, 2, LIGHT_STYLE_NONE, LIGHT_STYLE_NONE],
                light_offset: 64,
                lightmap_mins: [-3, 7],
                lightmap_size: [4, 2],
                ..FaceFields::default()
            },
        );
        let map = synthetic_surfaces(None, Some(faces.into_inner()), None);
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let face = &surfaces.faces()[0];

        assert_eq!(face.light_offset, 64);
        assert_eq!(face.lightmap_mins, [-3, 7]);
        assert_eq!(face.lightmap_size, [4, 2]);
        assert_eq!(face.light_style_count(), 2);
        assert!(face.has_lightmap());
        // One sample wider and taller than the extent, because the samples
        // are at the luxels' corners.
        assert_eq!(face.lightmap_sample_count(), Some(15));
    }

    #[test]
    fn reads_the_lightmap_projection_a_surface_is_lit_through() {
        let mut texinfos = Writer::new();
        write_texinfo_with_lightmap(&mut texinfos, 8.0, 0.25, 0, 0);
        let map = synthetic_materials(None, Some(texinfos.into_inner()), None, None);
        let bsp = Bsp::parse(&map).unwrap();
        let materials = Materials::parse(&bsp).unwrap();
        let texinfo = materials.texinfos()[0];

        // The two projections are separate planes at separate scales, so a
        // reader that took the texture's for both would light the surface at
        // the wrong place on its lightmap.
        assert_eq!(texinfo.texels([1.0, 1.0, 0.0]), [8.0, 8.0]);
        assert_eq!(texinfo.luxels([1.0, 1.0, 0.0]), [0.25, 0.25]);
    }

    #[test]
    fn reads_only_the_static_layer_of_a_face_with_several_light_styles() {
        // Four samples: the static layer, then a layer for a light that
        // switches. Reading both would light the surface with the sum of a
        // lamp's on and off states.
        let lighting = samples(&[
            sample([10, 20, 30], 0),
            sample([40, 50, 60], 0),
            sample([200, 200, 200], 0),
            sample([201, 201, 201], 0),
        ]);
        let mut face = lit_face(0, [1, 0]);
        face.styles = [0, 3, LIGHT_STYLE_NONE, LIGHT_STYLE_NONE];
        let map = synthetic_lit(vec![face], lighting, false);
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let lighting = Lightmaps::parse(&bsp).unwrap();

        let read = lighting
            .face_samples(&surfaces.faces()[0])
            .unwrap()
            .unwrap();
        assert_eq!(
            read,
            &samples(&[sample([10, 20, 30], 0), sample([40, 50, 60], 0)])[..]
        );
    }

    #[test]
    fn takes_the_high_range_lighting_where_a_map_carries_it() {
        let lighting = samples(&[sample([90, 90, 90], 1)]);
        let map = synthetic_lit(vec![lit_face(0, [0, 0])], lighting.clone(), true);
        let bsp = Bsp::parse(&map).unwrap();
        let read = Lightmaps::parse(&bsp).unwrap();

        assert!(read.is_high_range());
        assert_eq!(read.sample_count(), 1);
    }

    #[test]
    fn reports_a_face_with_no_lightmap_rather_than_reading_one() {
        let map = synthetic_lit(
            vec![FaceFields::default()],
            samples(&[sample([1, 2, 3], 0)]),
            false,
        );
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let lighting = Lightmaps::parse(&bsp).unwrap();

        assert_eq!(
            lighting.face_samples(&surfaces.faces()[0]).unwrap(),
            None,
            "an unlit surface has no samples rather than the first in the lump"
        );
    }

    #[test]
    fn refuses_a_face_whose_lightmap_runs_past_the_lighting_lump() {
        let map = synthetic_lit(
            vec![lit_face(0, [8, 8])],
            samples(&[sample([1, 2, 3], 0)]),
            false,
        );
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let lighting = Lightmaps::parse(&bsp).unwrap();

        assert!(
            matches!(
                lighting.face_samples(&surfaces.faces()[0]),
                Err(Error::LightmapOutOfRange {
                    offset: 0,
                    length,
                }) if length == 81 * LIGHTMAP_SAMPLE_SIZE
            ),
            "a face reaching past the lump is refused, got {:?}",
            lighting.face_samples(&surfaces.faces()[0])
        );
    }

    #[test]
    fn refuses_a_lighting_lump_that_is_not_whole_samples() {
        let map = synthetic_lit(vec![lit_face(0, [0, 0])], vec![1, 2, 3], false);
        let bsp = Bsp::parse(&map).unwrap();

        assert!(
            matches!(Lightmaps::parse(&bsp), Err(Error::InvalidLightingLump(3))),
            "a partial sample is refused, got {:?}",
            Lightmaps::parse(&bsp).map(|read| read.sample_count())
        );
    }

    #[test]
    fn packs_each_lit_surface_into_its_own_block() {
        let lighting = samples(&[
            // A 2x1 extent, so four samples.
            sample([255, 255, 255], 0),
            sample([255, 255, 255], 0),
            sample([0, 0, 0], 0),
            sample([0, 0, 0], 0),
            // Then a single sample for the second surface.
            sample([255, 255, 255], 1),
        ]);
        let faces = vec![
            lit_face(0, [1, 1]),
            lit_face(4 * LIGHTMAP_SAMPLE_SIZE as i32, [0, 0]),
            FaceFields::default(),
        ];
        let map = synthetic_lit(faces, lighting, false);
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();
        let lighting = Lightmaps::parse(&bsp).unwrap();
        let atlas = LightmapAtlas::pack(&surfaces, &materials, &lighting, 0..3).unwrap();

        assert_eq!(atlas.len(), 2, "the unlit surface is not packed");
        let first = atlas.placement(0).expect("the lit surface is placed");
        let second = atlas.placement(1).expect("the lit surface is placed");
        assert_eq!(first.extent, [2, 2]);
        assert_eq!(second.extent, [1, 1]);
        assert_eq!(atlas.placement(2), None);
        // Padded apart, so that filtering across one block's edge cannot
        // reach the other surface's lighting.
        assert!(
            second.origin[0] > first.origin[0] + first.extent[0]
                || second.origin[1] > first.origin[1] + first.extent[1],
            "the blocks are separated, got {first:?} and {second:?}"
        );
    }

    #[test]
    fn writes_a_sample_at_the_brightness_the_engine_stores_it() {
        // A sample's value is its byte scaled by two to the exponent and by
        // a further 1/255, which puts white at one. It is stored divided by
        // the overbright and in the screen's gamma, so white lands at half
        // of full and a sample twice as bright still fits.
        let lighting = samples(&[
            sample([255, 255, 255], 0),
            sample([255, 255, 255], 1),
            sample([255, 255, 255], 4),
            sample([0, 0, 0], 0),
        ]);
        let faces = vec![lit_face(0, [1, 1])];
        let map = synthetic_lit(faces, lighting, false);
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();
        let lighting = Lightmaps::parse(&bsp).unwrap();
        let atlas = LightmapAtlas::pack(&surfaces, &materials, &lighting, 0..1).unwrap();

        let placement = atlas.placement(0).unwrap();
        let at = |column: u32, row: u32| {
            let x = placement.origin[0] + column;
            let y = placement.origin[1] + row;
            let index = ((y * atlas.width() + x) * 4) as usize;
            atlas.pixels()[index + 1]
        };
        assert_eq!(at(0, 0), 128, "a fully lit surface is stored at half");
        let overbright =
            (LIGHTMAP_OVERBRIGHT.powf(1.0 / 2.2) / LIGHTMAP_OVERBRIGHT * 255.0 + 0.5) as u8;
        assert_eq!(at(1, 0), overbright, "twice white reaches the overbright");
        assert_eq!(at(0, 1), overbright, "and no further, being clamped there");
        assert_eq!(at(1, 1), 0, "an unlit sample is black");
    }

    #[test]
    fn gives_a_lit_surface_coordinates_inside_its_own_block() {
        let lighting = samples(&[
            sample([255, 255, 255], 0),
            sample([128, 128, 128], 0),
            sample([64, 64, 64], 0),
            sample([32, 32, 32], 0),
        ]);
        let map = synthetic_lit(vec![lit_face(0, [1, 1])], lighting, false);
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();
        let lighting = Lightmaps::parse(&bsp).unwrap();
        let atlas = LightmapAtlas::pack(&surfaces, &materials, &lighting, 0..1).unwrap();
        let geometry = surfaces
            .triangulate_lit(&materials, Some(&atlas), 0..1)
            .unwrap();

        assert_eq!(geometry.lightmap_coords.len(), geometry.positions.len());
        let placement = atlas.placement(0).unwrap();
        for coordinate in &geometry.lightmap_coords {
            let x = coordinate[0] * atlas.width() as f32;
            let y = coordinate[1] * atlas.height() as f32;
            assert!(
                x >= placement.origin[0] as f32
                    && x <= (placement.origin[0] + placement.extent[0]) as f32
                    && y >= placement.origin[1] as f32
                    && y <= (placement.origin[1] + placement.extent[1]) as f32,
                "the coordinate {coordinate:?} falls in {placement:?}"
            );
        }
    }

    #[test]
    fn stops_sharing_a_corner_between_surfaces_once_they_are_lit() {
        // Two surfaces over the same square sharing a texture projection.
        // Without lighting they share their corners; with it they cannot,
        // because a corner's lighting comes from the surface it is part of.
        let lighting = samples(&[sample([200, 200, 200], 0), sample([8, 8, 8], 0)]);
        let faces = vec![
            lit_face(0, [0, 0]),
            lit_face(LIGHTMAP_SAMPLE_SIZE as i32, [0, 0]),
        ];
        let map = synthetic_lit(faces, lighting, false);
        let bsp = Bsp::parse(&map).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();
        let lighting = Lightmaps::parse(&bsp).unwrap();
        let atlas = LightmapAtlas::pack(&surfaces, &materials, &lighting, 0..2).unwrap();

        let shared = surfaces.triangulate_textured(&materials, 0..2).unwrap();
        let lit = surfaces
            .triangulate_lit(&materials, Some(&atlas), 0..2)
            .unwrap();

        assert_eq!(shared.positions.len(), 4, "the corners are shared");
        assert_eq!(lit.positions.len(), 8, "each surface keeps its own");
        assert_eq!(shared.indices.len(), lit.indices.len());
    }

    #[test]
    fn names_the_material_a_surface_is_drawn_with() {
        let bytes = synthetic_materials(None, None, None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        let materials = Materials::parse(&bsp).unwrap();

        assert_eq!(materials.name(0), Some("brick/brickwall001a"));
        assert_eq!(materials.texinfos().len(), 1);
        assert_eq!(materials.texdatas()[0].width, 64);
        assert_eq!(materials.name(1), None);
    }

    #[test]
    fn lays_a_texture_across_a_surface_in_the_units_a_sampler_takes() {
        let bytes = synthetic_materials(None, None, None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();

        let geometry = surfaces.triangulate_textured(&materials, [0]).unwrap();

        // The square runs one unit each way and the texture is laid on at a
        // texel per unit across a 64 texel material, so its far corner is a
        // sixty-fourth of the way across.
        assert_eq!(geometry.positions.len(), 4);
        assert_eq!(geometry.texcoords.len(), 4);
        assert_eq!(geometry.texcoords[0], [0.0, 0.0]);
        assert_eq!(geometry.texcoords[2], [1.0 / 64.0, 1.0 / 64.0]);
        assert_eq!(geometry.triangle_count(), 2);
        assert_eq!(
            geometry.batches,
            vec![Batch {
                texdata: 0,
                first_index: 0,
                index_count: 6,
            }]
        );
    }

    #[test]
    fn groups_surfaces_into_one_run_per_material() {
        // Two squares, the second using a different material.
        let mut faces = Writer::new();
        write_face_with_texinfo(&mut faces, 0, 0, 4, -1, 0);
        write_face_with_texinfo(&mut faces, 0, 0, 4, -1, 1);
        write_face_with_texinfo(&mut faces, 0, 0, 4, -1, 0);

        let mut texinfos = Writer::new();
        write_texinfo(&mut texinfos, 1.0, 0, 0);
        write_texinfo(&mut texinfos, 2.0, 0, 1);

        let mut texdatas = Writer::new();
        write_texdata(&mut texdatas, 0, 64, 64);
        // The second entry of the string table, which is where the second
        // name begins; the table holds the offsets, not the texdata.
        write_texdata(&mut texdatas, 1, 32, 32);

        let bytes = synthetic_materials(
            Some(faces.into_inner()),
            Some(texinfos.into_inner()),
            Some(texdatas.into_inner()),
            Some((
                vec![0, 20],
                b"brick/brickwall001a\0metal/metalwall001a\0".to_vec(),
            )),
        );
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();

        let geometry = surfaces.triangulate_textured(&materials, 0..3).unwrap();

        // The two surfaces sharing a material are drawn in one run even
        // though another surface sits between them in the lump.
        assert_eq!(
            geometry.batches,
            vec![
                Batch {
                    texdata: 0,
                    first_index: 0,
                    index_count: 12,
                },
                Batch {
                    texdata: 1,
                    first_index: 12,
                    index_count: 6,
                },
            ]
        );
        assert_eq!(materials.name(1), Some("metal/metalwall001a"));

        // The two surfaces under one material share their corners, and the
        // third does not share with them because it projects its texture
        // differently.
        assert_eq!(geometry.positions.len(), 8);
        assert_eq!(geometry.texcoords[4], [0.0, 0.0]);
        assert_eq!(geometry.texcoords[6], [2.0 / 32.0, 2.0 / 32.0]);
    }

    #[test]
    fn leaves_out_the_surfaces_a_world_draw_never_shows() {
        let mut faces = Writer::new();
        write_face_with_texinfo(&mut faces, 0, 0, 4, -1, 0);
        write_face_with_texinfo(&mut faces, 0, 0, 4, -1, 1);
        // A face naming no texinfo at all, which the format allows.
        write_face_with_texinfo(&mut faces, 0, 0, 4, -1, -1);

        let mut texinfos = Writer::new();
        write_texinfo(&mut texinfos, 1.0, SURF_SKY, 0);
        write_texinfo(&mut texinfos, 1.0, SURF_NODRAW, 0);

        let bytes = synthetic_materials(
            Some(faces.into_inner()),
            Some(texinfos.into_inner()),
            None,
            None,
        );
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();

        let geometry = surfaces.triangulate_textured(&materials, 0..3).unwrap();

        assert!(
            geometry.is_empty(),
            "the sky and the compiler's own annotations are not world surfaces"
        );
        assert!(geometry.batches.is_empty());
    }

    #[test]
    fn refuses_material_lumps_that_do_not_agree_with_each_other() {
        // A texinfo naming a texdata that is not there.
        let mut texinfos = Writer::new();
        write_texinfo(&mut texinfos, 1.0, 0, 7);
        let bytes = synthetic_materials(None, Some(texinfos.into_inner()), None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Materials::parse(&bsp),
            Err(Error::InvalidTexData(7))
        ));

        // A drawn surface with no material at all.
        let mut texinfos = Writer::new();
        write_texinfo(&mut texinfos, 1.0, 0, -1);
        let bytes = synthetic_materials(None, Some(texinfos.into_inner()), None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Materials::parse(&bsp),
            Err(Error::InvalidTexData(-1))
        ));

        // A texdata naming a string the table does not hold.
        let mut texdatas = Writer::new();
        write_texdata(&mut texdatas, 5, 64, 64);
        let bytes = synthetic_materials(None, None, Some(texdatas.into_inner()), None);
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Materials::parse(&bsp),
            Err(Error::InvalidMaterialName(5))
        ));

        // An offset that runs off the end of the name data, and one whose
        // name is never terminated.
        for names in [
            (vec![64], b"brick/brickwall001a\0".to_vec()),
            (vec![0], b"brick".to_vec()),
        ] {
            let offset = names.0[0] as u32;
            let bytes = synthetic_materials(None, None, None, Some(names));
            let bsp = Bsp::parse(&bytes).unwrap();
            assert!(
                matches!(Materials::parse(&bsp), Err(Error::InvalidMaterialName(seen)) if seen == offset),
                "an unreadable name at {offset} is refused"
            );
        }

        // A material of no size, which no coordinate can be stated against.
        let mut texdatas = Writer::new();
        write_texdata(&mut texdatas, 0, 0, 64);
        let bytes = synthetic_materials(None, None, Some(texdatas.into_inner()), None);
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Materials::parse(&bsp),
            Err(Error::UnusableProjection(0))
        ));

        // A projection that is not a number.
        let mut texinfos = Writer::new();
        write_texinfo(&mut texinfos, f32::NAN, 0, 0);
        let bytes = synthetic_materials(None, Some(texinfos.into_inner()), None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Materials::parse(&bsp),
            Err(Error::UnusableProjection(0))
        ));
    }

    #[test]
    fn refuses_a_surface_naming_a_texinfo_that_is_not_there() {
        let mut faces = Writer::new();
        write_face_with_texinfo(&mut faces, 0, 0, 4, -1, 9);
        let bytes = synthetic_materials(Some(faces.into_inner()), None, None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();

        assert!(matches!(
            surfaces.triangulate_textured(&materials, [0]),
            Err(Error::InvalidTexInfo(9))
        ));
    }

    #[test]
    fn triangulates_a_world_surface_into_shared_indexed_corners() {
        let bytes = synthetic_surfaces(None, None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();

        assert_eq!(surfaces.faces().len(), 1);
        assert_eq!(surfaces.leaf_faces(), &[0]);
        assert!(!surfaces.faces()[0].is_displacement());

        // Walking the loop has to recover the square in order, including the
        // corner reached through the reversed edge.
        assert_eq!(surfaces.face_loop(0).unwrap(), vec![0, 1, 2, 3]);

        let geometry = surfaces.triangulate([0]).unwrap();
        // Four corners, not six: the two triangles of the fan share the
        // diagonal, which is the whole point of indexing them.
        assert_eq!(
            geometry.positions,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ]
        );
        assert_eq!(geometry.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(geometry.triangle_count(), 2);
        assert!(!geometry.is_empty());
    }

    #[test]
    fn shares_positions_between_faces_that_meet() {
        // Two faces over the same four corners: the second must reuse the
        // positions the first emitted rather than appending its own copies.
        let mut faces = Writer::new();
        write_face(&mut faces, 0, 0, 4, -1);
        write_face(&mut faces, 0, 0, 4, -1);
        let bytes = synthetic_surfaces(None, Some(faces.into_inner()), Some(vec![0, 1]));
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();

        let geometry = surfaces.triangulate([0, 1]).unwrap();
        assert_eq!(geometry.positions.len(), 4);
        assert_eq!(geometry.indices, vec![0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn skips_displacement_faces_whose_geometry_lives_elsewhere() {
        let mut faces = Writer::new();
        write_face(&mut faces, 0, 0, 4, 7);
        let bytes = synthetic_surfaces(None, Some(faces.into_inner()), Some(vec![0]));
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();

        assert!(surfaces.faces()[0].is_displacement());
        let geometry = surfaces.triangulate([0]).unwrap();
        assert!(geometry.is_empty());
        assert!(geometry.positions.is_empty());
    }

    #[test]
    fn refuses_surface_lumps_that_point_outside_themselves() {
        // A surface edge naming an edge the lump does not hold.
        let bytes = synthetic_surfaces(Some(vec![0, 1, 2, 9]), None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Surfaces::parse(&bsp),
            Err(Error::InvalidSurfedge {
                surfedge: 3,
                edge: 9
            })
        ));

        // A face whose run of surface edges runs past the end of the lump.
        let mut faces = Writer::new();
        write_face(&mut faces, 0, 2, 4, -1);
        let bytes = synthetic_surfaces(None, Some(faces.into_inner()), None);
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Surfaces::parse(&bsp),
            Err(Error::InvalidFaceRange {
                face: 0,
                first: 2,
                count: 4
            })
        ));

        // A leaf naming a face that does not exist.
        let bytes = synthetic_surfaces(None, None, Some(vec![5]));
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            Surfaces::parse(&bsp),
            Err(Error::InvalidFaceIndex(5))
        ));

        // Two edges cannot enclose a surface.
        let mut faces = Writer::new();
        write_face(&mut faces, 0, 0, 2, -1);
        let bytes = synthetic_surfaces(None, Some(faces.into_inner()), None);
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        assert!(matches!(
            surfaces.triangulate([0]),
            Err(Error::DegenerateFace { face: 0, edges: 2 })
        ));
        assert!(matches!(
            surfaces.triangulate([1]),
            Err(Error::InvalidFaceIndex(1))
        ));
    }

    #[test]
    fn deterministic_lump_round_trip() {
        let mut builder = Builder::new(20, 7);
        builder.set_lump(0, 1, 0, b"{\"classname\" \"worldspawn\"}\0".to_vec());
        builder.set_lump(40, 3, 1234, vec![1, 2, 3, 4, 5]);
        let first = builder.build().unwrap();
        let second = builder.build().unwrap();
        assert_eq!(first, second);

        let bsp = Bsp::parse(&first).unwrap();
        assert_eq!(bsp.header().version, 20);
        assert_eq!(bsp.header().map_revision, 7);
        assert_eq!(bsp.lump(0).unwrap(), b"{\"classname\" \"worldspawn\"}\0");
        assert_eq!(bsp.lump(40).unwrap(), &[1, 2, 3, 4, 5]);

        let repacked = bsp.to_builder().build().unwrap();
        assert_eq!(first, repacked);
    }

    #[test]
    fn golden_empty_header() {
        let bytes = Builder::new(19, 42).build().unwrap();
        assert_eq!(bytes.len(), BSP_HEADER_SIZE);
        assert_eq!(&bytes[0..8], &[b'V', b'B', b'S', b'P', 19, 0, 0, 0]);
        assert_eq!(&bytes[BSP_HEADER_SIZE - 4..], &42i32.to_le_bytes());
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(bsp.header().lumps.iter().all(|lump| lump.length == 0));
    }

    #[test]
    fn rejects_bad_ranges_and_versions() {
        let mut bytes = Builder::new(20, 0).build().unwrap();
        bytes[4..8].copy_from_slice(&22i32.to_le_bytes());
        assert!(matches!(
            Bsp::parse(&bytes),
            Err(Error::UnsupportedVersion(22))
        ));

        bytes[4..8].copy_from_slice(&20i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        bytes[12..16].copy_from_slice(&4i32.to_le_bytes());
        assert!(matches!(
            Bsp::parse(&bytes),
            Err(Error::InvalidLump { index: 0, .. })
        ));
    }

    fn entities_of(text: &str) -> Entities {
        let mut builder = Builder::new(20, 1);
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        builder.set_lump(LUMP_ENTITIES, 0, 0, bytes);
        let bytes = builder.build().unwrap();
        let bsp = Bsp::parse(&bytes).unwrap();
        Entities::parse(&bsp).unwrap()
    }

    #[test]
    fn reads_what_the_map_places_in_itself() {
        let entities = entities_of(
            "{\n\"classname\" \"worldspawn\"\n\"skyname\" \"sky_day01_01\"\n}\n\
             {\n\"classname\" \"info_player_start\"\n\"origin\" \"-4690 -1186 -27\"\n\
             \"angles\" \"0 90 0\"\n}\n\
             {\n\"classname\" \"func_door\"\n\"model\" \"*12\"\n\"origin\" \"8 0 0\"\n}\n",
        );

        assert_eq!(entities.len(), 3);
        assert_eq!(
            entities.worldspawn().and_then(|spawn| spawn.get("skyname")),
            Some("sky_day01_01")
        );

        let start = entities
            .by_classname("info_player_start")
            .next()
            .expect("the map starts the player somewhere");
        assert_eq!(start.origin(), Some([-4690.0, -1186.0, -27.0]));
        assert_eq!(start.vector("angles"), Some([0.0, 90.0, 0.0]));
        assert_eq!(start.brush_model(), None, "a start is a point, not a brush");

        let door = entities.iter().last().expect("three entities were read");
        assert_eq!(
            door.brush_model(),
            Some(12),
            "a door names one of the map's own brush models"
        );
    }

    #[test]
    fn keeps_the_keys_a_map_states_more_than_once() {
        // An entity fires several outputs on one event by repeating the
        // key, so folding the pairs into a map would lose all but one.
        let entities = entities_of(
            "{\n\"classname\" \"trigger_multiple\"\n\
             \"OnStartTouch\" \"door,Open,,0,-1\"\n\
             \"OnStartTouch\" \"light,TurnOn,,0,-1\"\n}\n",
        );

        let trigger = entities.iter().next().unwrap();
        assert_eq!(
            trigger.all("OnStartTouch").collect::<Vec<_>>(),
            ["door,Open,,0,-1", "light,TurnOn,,0,-1"]
        );
        assert_eq!(trigger.get("OnStartTouch"), Some("door,Open,,0,-1"));
        assert_eq!(trigger.get("OnEndTouch"), None);
    }

    #[test]
    fn reads_a_value_holding_a_content_path() {
        // The format has no escape, so a backslash in a value is a
        // Windows content path rather than the start of one.
        let entities = entities_of(
            "{\n\"classname\" \"prop_static\"\n\
             \"model\" \"models\\props_c17\\door01a.mdl\"\n}\n",
        );

        let prop = entities.iter().next().unwrap();
        assert_eq!(prop.get("model"), Some("models\\props_c17\\door01a.mdl"));
        assert_eq!(prop.brush_model(), None, "a model file is not a brush");
    }

    #[test]
    fn ignores_what_the_lump_is_padded_with() {
        // The lump is terminated by a nul and padded to a boundary past
        // it, which is not entity text and is not the map's to answer for.
        let mut builder = Builder::new(20, 1);
        let mut bytes = b"{\n\"classname\" \"worldspawn\"\n}\n\0".to_vec();
        bytes.extend_from_slice(b"\xff\xff\xff\xff");
        builder.set_lump(LUMP_ENTITIES, 0, 0, bytes);
        let bytes = builder.build().unwrap();
        let bsp = Bsp::parse(&bytes).unwrap();

        let entities = Entities::parse(&bsp).unwrap();

        assert_eq!(entities.len(), 1);
        assert!(entities.worldspawn().is_some());
    }

    #[test]
    fn rejects_entity_text_the_format_does_not_allow() {
        for broken in [
            "{\n\"classname\" \"worldspawn\"\n",
            "\"classname\" \"worldspawn\"\n",
            "{\n{\n}\n",
            "{\n\"classname\"\n}\n",
            "}\n",
        ] {
            let mut builder = Builder::new(20, 1);
            let mut bytes = broken.as_bytes().to_vec();
            bytes.push(0);
            builder.set_lump(LUMP_ENTITIES, 0, 0, bytes);
            let bytes = builder.build().unwrap();
            let bsp = Bsp::parse(&bytes).unwrap();
            assert!(
                matches!(Entities::parse(&bsp), Err(Error::InvalidEntityText)),
                "{broken:?} is not entity text"
            );
        }
    }

    #[test]
    fn draws_only_what_the_visibility_set_reaches() {
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let world = World::parse(&bsp).unwrap();

        // The near room's visibility reaches only itself, so standing in it
        // leaves the far room's surface out even though the map holds it.
        let near = VisibleFaces::select(&world, &surfaces, [32.0, 32.0, 0.0]).unwrap();
        assert_eq!(near.faces().collect::<Vec<_>>(), [0, 1]);
        assert_eq!(near.total(), 4);
        assert_eq!(near.visible_leaves(), 1);

        // The far room reaches both, so it draws the whole map.
        let far = VisibleFaces::select(&world, &surfaces, [-32.0, 32.0, 0.0]).unwrap();
        assert_eq!(far.faces().collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(far.visible_leaves(), 2);
    }

    #[test]
    fn leaves_out_the_surfaces_the_world_tree_does_not_speak_for() {
        // A door is a brush model an entity places, so it appears in no
        // leaf, its geometry is stored about its own origin rather than in
        // the world, and the visibility set says nothing about it. Drawing
        // it where it is stored would pile it on the map's origin, so it
        // waits for the entity that places it.
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let world = World::parse(&bsp).unwrap();

        assert_eq!(world.models().len(), 2);
        assert_eq!(world.world_faces(), 0..3);
        assert_eq!(world.models()[1].faces(), 3..4);

        for standing in [[32.0, 32.0, 0.0], [-32.0, 32.0, 0.0]] {
            let visible = VisibleFaces::select(&world, &surfaces, standing).unwrap();
            assert!(
                !visible.contains(3),
                "the door is not the world tree's to place, standing at {standing:?}"
            );
        }
    }

    #[test]
    fn draws_the_whole_map_from_a_point_with_no_visibility_row() {
        // A map compiled without visibility, which is what a map still
        // being built looks like. Culling it to nothing would leave a
        // level designer staring at an empty screen, so it draws all of it.
        let bytes = synthetic_surfaces(None, None, None);
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();

        let visible = VisibleFaces::everything(&surfaces);

        assert_eq!(visible.len(), surfaces.faces().len());
        assert!(visible.contains(0));
    }

    /// Bounding planes that keep everything on the positive side of the
    /// one given, with the other five far enough out to hold the fixture.
    fn bounded_by(plane: [f32; 4]) -> Frustum {
        let mut planes = [[0.0, 0.0, 0.0, 4000.0]; 6];
        planes[0] = plane;
        for (slot, normal) in planes.iter_mut().skip(1).zip([
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
        ]) {
            slot[..3].copy_from_slice(&normal);
        }
        Frustum::new(planes)
    }

    /// The fixture's geometry, with every surface in it visible.
    fn rooms_geometry(bsp: &Bsp<'_>) -> (Surfaces, TexturedGeometry) {
        let surfaces = Surfaces::parse(bsp).unwrap();
        let materials = Materials::parse(bsp).unwrap();
        let geometry = surfaces
            .triangulate_textured(&materials, 0..surfaces.faces().len())
            .unwrap();
        (surfaces, geometry)
    }

    #[test]
    fn drops_the_surfaces_the_view_cannot_hold() {
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let (surfaces, geometry) = rooms_geometry(&bsp);
        let everything = VisibleFaces::everything(&surfaces);

        // Looking away from the whole map.
        let away = bounded_by([-1.0, 0.0, 0.0, -1000.0]);
        assert!(
            geometry
                .visible_batches(&everything, Some(&away))
                .is_empty(),
            "nothing is in front of the view"
        );

        // Turned around, the two surfaces at positive `x` are in front of
        // the plane and the two behind it are not.
        let near = bounded_by([1.0, 0.0, 0.0, -1.0]);
        let runs = geometry.visible_batches(&everything, Some(&near));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].first_index, 0);
        assert_eq!(runs[0].index_count, 12);
    }

    #[test]
    fn keeps_a_surface_the_view_only_clips_part_of() {
        // Culling by a box has to be generous. A surface the view crosses
        // is on screen however little of it is, and dropping it leaves a
        // hole no later pass can fill in.
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let (surfaces, geometry) = rooms_geometry(&bsp);
        let everything = VisibleFaces::everything(&surfaces);

        // A plane crossing the two surfaces that span `y` sixty-four to a
        // hundred and twenty-eight, leaving most of each behind it.
        let across = bounded_by([0.0, 1.0, 0.0, -100.0]);
        let runs = geometry.visible_batches(&everything, Some(&across));

        assert_eq!(runs.len(), 2, "the two surfaces the plane crosses");
        assert_eq!(runs[0].first_index, 6);
        assert_eq!(runs[1].first_index, 18);
    }

    #[test]
    fn culls_by_a_surfaces_own_reach_rather_than_the_leaf_holding_it() {
        // A surface is listed in the leaves it starts in and can reach
        // well past them, by as much as a thousand units on a shipped map.
        // Culling by the leaf's box would drop such a surface while part
        // of it was still on screen.
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let (_, geometry) = rooms_geometry(&bsp);
        let world = World::parse(&bsp).unwrap();

        // The far room's leaf ends at `y` sixty-four, but the surface the
        // near room lists reaches to a hundred and twenty-eight.
        let leaf = world.point_leaf([-32.0, 32.0, 0.0]).unwrap();
        assert_eq!(world.leaves()[leaf].maxs[1], 64);
        let reaching = geometry.runs.iter().find(|run| run.face == 1).unwrap();
        assert_eq!(reaching.maxs[1], 128.0);

        let beyond = bounded_by([0.0, 1.0, 0.0, -100.0]);
        assert!(
            !beyond.excludes(reaching.mins, reaching.maxs),
            "the surface reaches past the plane even though its leaf does not"
        );
    }

    #[test]
    fn narrows_the_visibility_set_by_the_view() {
        // The two do different work: the visibility set drops what a wall
        // hides, and the view drops what the screen does not hold. A frame
        // wants both.
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let (surfaces, geometry) = rooms_geometry(&bsp);
        let world = World::parse(&bsp).unwrap();

        // Standing in the far room, which reaches everything.
        let visible = VisibleFaces::select(&world, &surfaces, [-32.0, 32.0, 0.0]).unwrap();
        assert_eq!(visible.len(), 3);

        // A view holding only what the visibility set already reached
        // leaves the selection alone.
        let wide = bounded_by([0.0, 0.0, 1.0, 4000.0]);
        assert_eq!(
            geometry.visible_batches(&visible, Some(&wide)),
            geometry.visible_batches(&visible, None)
        );

        // A view holding only the far half draws less than the visibility
        // set reached, and less than the view alone would have: the door
        // is within the view but not the world tree's to place.
        let half = bounded_by([-1.0, 0.0, 0.0, -1.0]);
        let both = geometry.visible_batches(&visible, Some(&half));
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].first_index, 12, "the far room's surface alone");
        assert_eq!(both[0].index_count, 6);
        assert_eq!(
            geometry
                .visible_batches(&VisibleFaces::everything(&surfaces), Some(&half))
                .iter()
                .map(|run| run.index_count)
                .sum::<usize>(),
            12,
            "the view alone would have drawn the door with it"
        );
    }

    #[test]
    fn rejects_a_leaf_naming_surfaces_the_map_does_not_hold() {
        let bytes = synthetic_rooms();
        let mut bsp_bytes = bytes.clone();
        let bsp = Bsp::parse(&bytes).unwrap();
        let leaves = bsp.header.lumps[LUMP_LEAVES];
        // Point the near room's run past the end of the leaf-face lump.
        let count = leaves.offset as usize + 20;
        bsp_bytes[count..count + 2].copy_from_slice(&99u16.to_le_bytes());

        let bsp = Bsp::parse(&bsp_bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let world = World::parse(&bsp).unwrap();

        assert!(matches!(
            VisibleFaces::select(&world, &surfaces, [32.0, 32.0, 0.0]),
            Err(Error::InvalidLeafFaceRun { .. })
        ));
    }

    #[test]
    fn draws_the_same_triangles_in_the_same_order_when_nothing_is_culled() {
        // The runs a frame selects are the geometry's own batches when
        // every surface is visible. If they were not, culling would change
        // what a frame draws rather than only how much of it.
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();
        let geometry = surfaces
            .triangulate_textured(&materials, 0..surfaces.faces().len())
            .unwrap();

        let runs = geometry.visible_batches(&VisibleFaces::everything(&surfaces), None);

        assert_eq!(runs, geometry.batches);
        assert_eq!(geometry.runs.len(), 4, "one run per surface");
        assert_eq!(
            runs.iter().map(|run| run.index_count).sum::<usize>(),
            geometry.indices.len()
        );
    }

    #[test]
    fn joins_neighbouring_surfaces_and_splits_where_one_is_dropped() {
        let bytes = synthetic_rooms();
        let bsp = Bsp::parse(&bytes).unwrap();
        let surfaces = Surfaces::parse(&bsp).unwrap();
        let materials = Materials::parse(&bsp).unwrap();
        let world = World::parse(&bsp).unwrap();
        let geometry = surfaces
            .triangulate_textured(&materials, 0..surfaces.faces().len())
            .unwrap();

        // The near room's two surfaces sit next to each other in one
        // material, so they draw as one run rather than two.
        let near = VisibleFaces::select(&world, &surfaces, [32.0, 32.0, 0.0]).unwrap();
        let joined = geometry.visible_batches(&near, None);
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].first_index, 0);
        assert_eq!(joined[0].index_count, 12, "two squares, two triangles each");

        // Dropping the surface between two visible ones splits the run,
        // because the indices left out sit between them in the buffer.
        let mut split_apart = VisibleFaces::everything(&surfaces);
        split_apart.seen[1] = false;
        split_apart.faces -= 1;
        let split = geometry.visible_batches(&split_apart, None);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].index_count, 6);
        assert_eq!(split[1].first_index, 12);
        assert_eq!(
            split[1].index_count, 12,
            "the two surfaces after the gap are still joined to each other"
        );
        assert!(
            split.iter().all(|run| run.texdata == joined[0].texdata),
            "every run still draws with the material its surfaces name"
        );
    }

    #[test]
    fn owns_world_tree_and_visibility_queries() {
        let mut visibility = Writer::new();
        visibility.write_i32_le(2);
        visibility.write_i32_le(20);
        visibility.write_i32_le(21);
        visibility.write_i32_le(22);
        visibility.write_i32_le(23);
        visibility.write_bytes(&[0b01, 0b11, 0b10, 0b11]);
        let bytes = synthetic_world(Some(&visibility.into_inner()));
        let bsp = Bsp::parse(&bytes).unwrap();
        let world = World::parse(&bsp).unwrap();

        assert_eq!(world.planes().len(), 1);
        assert_eq!(world.nodes().len(), 1);
        assert_eq!(world.leaves().len(), 2);
        assert_eq!(world.cluster_count(), 2);
        assert_eq!(world.point_leaf([1.0, 0.0, 0.0]).unwrap(), 0);
        assert_eq!(world.point_leaf([-1.0, 0.0, 0.0]).unwrap(), 1);
        assert_eq!(
            world
                .visibility(0, VisibilityKind::PotentiallyVisible)
                .unwrap(),
            [0b01]
        );
        assert!(world.cluster_visible(0, 0).unwrap());
        assert!(!world.cluster_visible(0, 1).unwrap());
        assert!(world.cluster_visible(1, 1).unwrap());
    }

    #[test]
    fn no_visibility_means_all_clusters_visible() {
        let bytes = synthetic_world(None);
        let bsp = Bsp::parse(&bytes).unwrap();
        let world = World::parse(&bsp).unwrap();
        assert_eq!(world.cluster_count(), 2);
        assert_eq!(
            world
                .visibility(0, VisibilityKind::PotentiallyVisible)
                .unwrap(),
            [0xff]
        );
        assert!(world.cluster_visible(0, 1).unwrap());
    }

    #[test]
    fn rejects_invalid_visibility_runs() {
        let mut visibility = Writer::new();
        visibility.write_i32_le(2);
        for _ in 0..4 {
            visibility.write_i32_le(20);
        }
        visibility.write_bytes(&[0, 0]);
        let bytes = synthetic_world(Some(&visibility.into_inner()));
        let bsp = Bsp::parse(&bytes).unwrap();
        assert!(matches!(
            World::parse(&bsp),
            Err(Error::InvalidVisibilityRun)
        ));
    }
}
