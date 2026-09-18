//! Structural validation for Source studio-model sidecar formats.
//!
//! This crate validates the pointer-free disk relationships in MDL, VVD, VTX,
//! and PHY files. It intentionally does not deserialize native C++ structs.

use source_binary::Reader;
use source_keyvalues::{parse_bytes as parse_keyvalues, Document, ParseOptions};
use std::fmt;
use std::ops::Range;

pub const STUDIO_MAGIC: u32 = u32::from_le_bytes(*b"IDST");
pub const STUDIO_VERSION: i32 = 49;
pub const MIN_STUDIO_VERSION: i32 = 44;
pub const VVD_MAGIC: u32 = u32::from_le_bytes(*b"IDSV");
pub const VVD_VERSION: i32 = 4;
pub const VTX_VERSION: i32 = 7;
const MAX_LODS: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_items: usize,
    pub max_vertices: usize,
    pub max_solids: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 1024 * 1024 * 1024,
            max_items: 1_000_000,
            max_vertices: 10_000_000,
            max_solids: 4096,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    KeyValues(source_keyvalues::Error),
    FileTooLarge {
        size: usize,
        limit: usize,
    },
    InvalidMagic {
        format: &'static str,
        value: u32,
    },
    /// A bone names a parent that does not come before it, so the skeleton
    /// cannot be composed from the root down.
    BoneOutOfOrder {
        bone: usize,
        parent: i32,
    },
    UnsupportedVersion {
        format: &'static str,
        value: i32,
    },
    InvalidLength {
        format: &'static str,
        value: i32,
    },
    InvalidUtf8 {
        field: &'static str,
    },
    UnterminatedString {
        field: &'static str,
        offset: usize,
    },
    InvalidCount {
        field: &'static str,
        value: i32,
    },
    CountLimitExceeded {
        field: &'static str,
        value: usize,
        limit: usize,
    },
    InvalidOffset {
        field: &'static str,
        value: i32,
    },
    InvalidRange {
        field: &'static str,
        offset: usize,
        size: usize,
    },
    InvalidLodVertexCounts,
    InvalidVertexIndex {
        value: u16,
        vertex_count: usize,
    },
    SizeOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::KeyValues(error) => error.fmt(f),
            Self::FileTooLarge { size, limit } => {
                write!(f, "file size {size} exceeds limit {limit}")
            }
            Self::InvalidMagic { format, value } => {
                write!(f, "invalid {format} magic 0x{value:08x}")
            }
            Self::BoneOutOfOrder { bone, parent } => {
                write!(
                    f,
                    "bone {bone} names parent {parent}, which is not before it"
                )
            }
            Self::UnsupportedVersion { format, value } => {
                write!(f, "unsupported {format} version {value}")
            }
            Self::InvalidLength { format, value } => {
                write!(f, "invalid {format} declared length {value}")
            }
            Self::InvalidUtf8 { field } => write!(f, "{field} is not valid UTF-8"),
            Self::UnterminatedString { field, offset } => {
                write!(f, "unterminated {field} string at offset {offset}")
            }
            Self::InvalidCount { field, value } => {
                write!(f, "invalid {field} count {value}")
            }
            Self::CountLimitExceeded {
                field,
                value,
                limit,
            } => write!(f, "{field} count {value} exceeds limit {limit}"),
            Self::InvalidOffset { field, value } => {
                write!(f, "invalid {field} offset {value}")
            }
            Self::InvalidRange {
                field,
                offset,
                size,
            } => write!(f, "{field} range {offset}+{size} is outside the file"),
            Self::InvalidLodVertexCounts => write!(f, "invalid VVD LOD vertex counts"),
            Self::InvalidVertexIndex {
                value,
                vertex_count,
            } => write!(
                f,
                "VTX index {value} is outside strip-group vertex count {vertex_count}"
            ),
            Self::SizeOverflow => write!(f, "model format size arithmetic overflow"),
        }
    }
}

impl std::error::Error for Error {}

impl From<source_binary::Error> for Error {
    fn from(value: source_binary::Error) -> Self {
        Self::Binary(value)
    }
}

impl From<source_keyvalues::Error> for Error {
    fn from(value: source_keyvalues::Error) -> Self {
        Self::KeyValues(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Mdl<'a> {
    pub version: i32,
    pub checksum: i32,
    pub name: &'a str,
    pub declared_length: usize,
    /// The box the compiler measured the model's own geometry into, which
    /// is what a reader can hold decoded vertices against.
    pub hull_min: [f32; 3],
    pub hull_max: [f32; 3],
    /// The header's own flags, of which the only one read here says the
    /// model is a static prop.
    pub flags: i32,
    /// Where the bone table starts and how long it is, so the bind pose can
    /// be read without re-walking the header.
    bones_at: Range<usize>,
    pub keyvalues: Option<&'a [u8]>,
}

/// Bytes one bone takes in the table, over versions 44 through 49.
const BONE_SIZE: usize = 216;

/// The header flag marking a model the world places and never animates.
pub const STUDIOHDR_FLAGS_STATIC_PROP: i32 = 1 << 4;

const BODY_PART_SIZE: usize = 16;
const TEXTURE_SIZE: usize = 64;
const MODEL_SIZE: usize = 148;
const MESH_SIZE: usize = 116;

/// One of a model's meshes: the unit that carries a material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mesh {
    pub body_part: usize,
    pub model: usize,
    pub mesh: usize,
    /// Which of the model's materials this mesh draws with, before any skin
    /// table is applied.
    pub material: usize,
    /// Where this mesh's vertices start in the model's own vertex run,
    /// which is what a triangle index is relative to.
    pub vertex_base: usize,
    pub vertex_count: usize,
}

/// A rotation and a translation, three rows of four, which is how the model
/// format states every bone transform.
///
/// Rows of four rather than a four-by-four because the bottom row of a rigid
/// transform is always `0 0 0 1`, and the file leaves it out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform(pub [[f32; 4]; 3]);

impl Transform {
    pub const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]);

    /// The transform a quaternion and an offset describe, in the `x, y, z, w`
    /// order the file stores a quaternion in.
    pub fn from_rotation_and_offset(rotation: [f32; 4], offset: [f32; 3]) -> Self {
        let [x, y, z, w] = rotation;
        Self([
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
                offset[0],
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
                offset[1],
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
                offset[2],
            ],
        ])
    }

    /// This transform followed by nothing, applied to `other` first: the
    /// composition that puts a child bone's transform into its parent's
    /// space.
    #[must_use]
    pub fn times(&self, other: &Self) -> Self {
        let mut out = [[0.0f32; 4]; 3];
        for (row, values) in out.iter_mut().enumerate() {
            for (column, value) in values.iter_mut().enumerate().take(3) {
                *value = (0..3).map(|k| self.0[row][k] * other.0[k][column]).sum();
            }
            values[3] =
                (0..3).map(|k| self.0[row][k] * other.0[k][3]).sum::<f32>() + self.0[row][3];
        }
        Self(out)
    }

    pub fn apply(&self, point: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|row| {
            (0..3).map(|k| self.0[row][k] * point[k]).sum::<f32>() + self.0[row][3]
        })
    }

    /// How far this is from leaving everything where it was, as the largest
    /// difference in any of its twelve values.
    pub fn distance_from_identity(&self) -> f32 {
        let mut worst = 0.0f32;
        for (row, values) in self.0.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                let identity = if row == column { 1.0 } else { 0.0 };
                worst = worst.max((value - identity).abs());
            }
        }
        worst
    }
}

/// One of a model's bones, as far as placing its vertices needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bone {
    pub parent: i32,
    pub position: [f32; 3],
    /// The bone's own rotation, as the quaternion the file stores in x, y,
    /// z, w order.
    pub rotation: [f32; 4],
    /// The transform from model space into this bone's own space.
    ///
    /// A pose places a vertex by composing the bone's model-space transform
    /// with this, so in the rest pose the two cancel and the vertex stays
    /// where it was stored. See [`Mdl::bind_pose`].
    pub pose_to_bone: Transform,
}

impl Bone {
    /// Where this bone sits relative to its parent.
    pub fn local(&self) -> Transform {
        Transform::from_rotation_and_offset(self.rotation, self.position)
    }
}

/// The matrices that place a vertex, for a pose stated as each bone's
/// transform in model space.
///
/// Source skins a vertex by the bone's model-space transform composed with
/// the transform out of model space into that bone, so that the bone moving
/// moves the vertex and the bone at rest leaves it alone.
pub fn skinning(pose: &[Transform], bones: &[Bone]) -> Vec<Transform> {
    pose.iter()
        .zip(bones)
        .map(|(bone_to_model, bone)| bone_to_model.times(&bone.pose_to_bone))
        .collect()
}

/// Where a vertex lands, weighted across the bones it is bound to.
///
/// Returns the stored position unchanged where the vertex names a bone the
/// model does not have, which is what a file that disagrees with itself
/// would otherwise skin into nowhere.
pub fn skin(matrices: &[Transform], vertex: &Vertex) -> [f32; 3] {
    let mut placed = [0.0f32; 3];
    for index in 0..usize::from(vertex.bone_count) {
        let Some(matrix) = matrices.get(usize::from(vertex.bones[index])) else {
            return vertex.position;
        };
        let moved = matrix.apply(vertex.position);
        for (axis, value) in placed.iter_mut().enumerate() {
            *value += vertex.weights[index] * moved[axis];
        }
    }
    placed
}

impl<'a> Mdl<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        check_file_size(bytes, limits)?;
        let mut reader = Reader::new(bytes);
        let magic = reader.read_u32_le()?;
        if magic != STUDIO_MAGIC {
            return Err(Error::InvalidMagic {
                format: "MDL",
                value: magic,
            });
        }
        let version = reader.read_i32_le()?;
        if !(MIN_STUDIO_VERSION..=STUDIO_VERSION).contains(&version) {
            return Err(Error::UnsupportedVersion {
                format: "MDL",
                value: version,
            });
        }
        let checksum = reader.read_i32_le()?;
        let name_bytes = reader.take(64)?;
        let name_end = name_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name_bytes.len());
        let name = std::str::from_utf8(&name_bytes[..name_end])
            .map_err(|_| Error::InvalidUtf8 { field: "MDL name" })?;
        let length = reader.read_i32_le()?;
        let declared_length = positive_usize("MDL length", length)?;
        if declared_length < 344 || declared_length > bytes.len() {
            return Err(Error::InvalidLength {
                format: "MDL",
                value: length,
            });
        }
        let contents = &bytes[..declared_length];

        // Validate all of the root-level count/relative-offset pairs common to
        // versions 44 through 49. Detailed element decoding can be layered on
        // top without ever trusting a native pointer layout.
        for &(field, count_at, offset_at) in &[
            ("bones", 156, 160),
            ("bone controllers", 164, 168),
            ("hitbox sets", 172, 176),
            ("local animations", 180, 184),
            ("local sequences", 188, 192),
            ("textures", 204, 208),
            ("CD textures", 212, 216),
            ("body parts", 232, 236),
            ("local attachments", 240, 244),
            ("local nodes", 248, 252),
            ("flex descriptions", 260, 264),
            ("flex controllers", 268, 272),
            ("flex rules", 276, 280),
            ("IK chains", 284, 288),
            ("mouths", 292, 296),
            ("pose parameters", 300, 304),
            ("IK autoplay locks", 320, 324),
            ("included models", 336, 340),
        ] {
            validate_count_offset(contents, field, count_at, offset_at, limits.max_items)?;
        }

        let vector_at = |offset: usize| -> Result<[f32; 3]> {
            let mut reader = Reader::new(contents);
            reader.skip(offset)?;
            let mut out = [0.0f32; 3];
            for axis in &mut out {
                *axis = reader.read_f32_le()?;
            }
            Ok(out)
        };
        let hull_min = vector_at(104)?;
        let hull_max = vector_at(116)?;
        let flags = read_i32_at(contents, 152)?;
        let bone_count = checked_count("bones", read_i32_at(contents, 156)?, limits.max_items)?;
        let bones_at = if bone_count == 0 {
            0..0
        } else {
            relative_range(
                contents,
                0,
                read_i32_at(contents, 160)?,
                bone_count,
                BONE_SIZE,
                "bones",
            )?
        };
        if !hull_min
            .iter()
            .chain(hull_max.iter())
            .all(|value| value.is_finite())
        {
            return Err(Error::InvalidLength {
                format: "MDL hull",
                value: 0,
            });
        }

        let skin_refs = read_i32_at(contents, 220)?;
        let skin_families = read_i32_at(contents, 224)?;
        let skin_count = checked_count("skin references", skin_refs, limits.max_items)?
            .checked_mul(checked_count(
                "skin families",
                skin_families,
                limits.max_items,
            )?)
            .ok_or(Error::SizeOverflow)?;
        let skin_offset = read_i32_at(contents, 228)?;
        if skin_count != 0 {
            relative_range(contents, 0, skin_offset, skin_count, 2, "skin table")?;
        }

        let surface_property_offset = read_i32_at(contents, 308)?;
        if surface_property_offset != 0 {
            c_string(
                contents,
                positive_usize("surface property", surface_property_offset)?,
                "surface property",
            )?;
        }
        let keyvalue_offset = read_i32_at(contents, 312)?;
        let keyvalue_size = read_i32_at(contents, 316)?;
        let keyvalues = if keyvalue_size == 0 {
            None
        } else {
            let size = positive_usize("embedded KeyValues size", keyvalue_size)?;
            let offset = positive_usize("embedded KeyValues", keyvalue_offset)?;
            Some(
                contents
                    .get(offset..offset.checked_add(size).ok_or(Error::SizeOverflow)?)
                    .ok_or(Error::InvalidRange {
                        field: "embedded KeyValues",
                        offset,
                        size,
                    })?,
            )
        };

        Ok(Self {
            version,
            checksum,
            name,
            declared_length,
            hull_min,
            hull_max,
            flags,
            bones_at,
            keyvalues,
        })
    }

    /// Whether the model is a static prop: one the world places and never
    /// animates.
    ///
    /// This decides what the stored vertices mean. A static prop's are in
    /// the model's own space and can be drawn as they are read. A jointed
    /// model's are relative to the bones that move them, so drawing one
    /// means composing the bind pose first.
    pub fn is_static_prop(&self) -> bool {
        self.flags & STUDIOHDR_FLAGS_STATIC_PROP != 0
    }

    /// The materials the model's meshes name, and the directories to look
    /// for them in.
    ///
    /// A model names a material by its bare name and separately lists the
    /// directories it may be under, so resolving one means joining the two
    /// and taking the first that exists. Models share directories, which is
    /// why the file stores them apart from the names.
    pub fn materials(&self, bytes: &'a [u8]) -> Result<(Vec<String>, Vec<String>)> {
        let contents = &bytes[..self.declared_length];
        let read_name = |at: usize, base: usize| -> Result<String> {
            let offset = read_i32_at(contents, at)?;
            let start = relative_offset(contents, base, offset, "MDL material name")?;
            let text = c_string(contents, start, "MDL material name")?;
            // The compiler writes these with the separators of the machine
            // it ran on, so a model built on Windows names its directories
            // with backslashes and nothing that reads content can use them
            // as they stand.
            Ok(text.replace('\\', "/"))
        };

        let texture_count = checked_count("MDL textures", read_i32_at(contents, 204)?, usize::MAX)?;
        let mut names = Vec::with_capacity(texture_count);
        if texture_count != 0 {
            let table = relative_range(
                contents,
                0,
                read_i32_at(contents, 208)?,
                texture_count,
                TEXTURE_SIZE,
                "MDL textures",
            )?;
            for index in 0..texture_count {
                let at = table.start + index * TEXTURE_SIZE;
                names.push(read_name(at, at)?);
            }
        }

        let directory_count = checked_count(
            "MDL material directories",
            read_i32_at(contents, 212)?,
            usize::MAX,
        )?;
        let mut directories = Vec::with_capacity(directory_count);
        if directory_count != 0 {
            let table = relative_range(
                contents,
                0,
                read_i32_at(contents, 216)?,
                directory_count,
                4,
                "MDL material directories",
            )?;
            for index in 0..directory_count {
                // These offsets are from the start of the file rather than
                // from the entry, unlike every other name in the header.
                directories.push(read_name(table.start + index * 4, 0)?);
            }
        }

        Ok((names, directories))
    }

    /// Every mesh the model draws, in the order the triangle file lists
    /// them.
    ///
    /// A mesh is the unit that carries a material, and its triangles index
    /// vertices from `vertex_base` onwards, which is what joins the two
    /// files: the triangle file numbers a mesh's vertices from zero and the
    /// vertex file holds them all in one run.
    pub fn meshes(&self, bytes: &'a [u8]) -> Result<Vec<Mesh>> {
        let contents = &bytes[..self.declared_length];
        let part_count = checked_count("body parts", read_i32_at(contents, 232)?, usize::MAX)?;
        if part_count == 0 {
            return Ok(Vec::new());
        }
        let parts = relative_range(
            contents,
            0,
            read_i32_at(contents, 236)?,
            part_count,
            BODY_PART_SIZE,
            "body parts",
        )?;

        let mut meshes = Vec::new();
        for part in 0..part_count {
            let part_at = parts.start + part * BODY_PART_SIZE;
            let model_count = checked_count(
                "body part models",
                read_i32_at(contents, part_at + 4)?,
                usize::MAX,
            )?;
            if model_count == 0 {
                continue;
            }
            let models = relative_range(
                contents,
                part_at,
                read_i32_at(contents, part_at + 12)?,
                model_count,
                MODEL_SIZE,
                "body part models",
            )?;
            for model in 0..model_count {
                let model_at = models.start + model * MODEL_SIZE;
                let mesh_count = checked_count(
                    "model meshes",
                    read_i32_at(contents, model_at + 72)?,
                    usize::MAX,
                )?;
                // Where this model's vertices begin, stated in bytes by the
                // file and in vertices by everything that indexes them.
                let vertex_index = read_i32_at(contents, model_at + 84)?;
                let vertex_base =
                    positive_usize("model vertex offset", vertex_index.max(0))? / VVD_VERTEX_SIZE;
                if mesh_count == 0 {
                    continue;
                }
                let entries = relative_range(
                    contents,
                    model_at,
                    read_i32_at(contents, model_at + 76)?,
                    mesh_count,
                    MESH_SIZE,
                    "model meshes",
                )?;
                for mesh in 0..mesh_count {
                    let mesh_at = entries.start + mesh * MESH_SIZE;
                    let material = read_i32_at(contents, mesh_at)?;
                    let vertex_offset = read_i32_at(contents, mesh_at + 12)?;
                    let vertex_count = read_i32_at(contents, mesh_at + 8)?;
                    meshes.push(Mesh {
                        body_part: part,
                        model,
                        mesh,
                        material: checked_count("mesh material", material, usize::MAX)?,
                        vertex_base: vertex_base
                            + checked_count("mesh vertex offset", vertex_offset, usize::MAX)?,
                        vertex_count: checked_count("mesh vertices", vertex_count, usize::MAX)?,
                    });
                }
            }
        }
        Ok(meshes)
    }

    /// The model's bind pose, in the order the vertices name it.
    pub fn bones(&self, bytes: &'a [u8]) -> Result<Vec<Bone>> {
        let count = self.bones_at.len() / BONE_SIZE;
        let mut bones = Vec::with_capacity(count);
        for index in 0..count {
            let at = self.bones_at.start + index * BONE_SIZE;
            let mut reader = Reader::with_position(bytes, at)?;
            reader.skip(4)?; // the bone's name, which placing it does not need
            let parent = reader.read_i32_le()?;
            reader.skip(24)?; // six bone controllers
            let mut position = [0.0f32; 3];
            for axis in &mut position {
                *axis = reader.read_f32_le()?;
            }
            let mut rotation = [0.0f32; 4];
            for part in &mut rotation {
                *part = reader.read_f32_le()?;
            }
            // The Euler angles and the two scales the file keeps beside the
            // quaternion are for animation decompression, not for placing a
            // vertex in the rest pose.
            reader.skip(36)?;
            let mut pose_to_bone = Transform::IDENTITY;
            for row in &mut pose_to_bone.0 {
                for value in row {
                    *value = reader.read_f32_le()?;
                }
            }
            bones.push(Bone {
                parent,
                position,
                rotation,
                pose_to_bone,
            });
        }
        Ok(bones)
    }

    /// Each bone's transform in model space at rest, composed down the
    /// skeleton from the root.
    ///
    /// This is the pose the vertices were stored against, so skinning
    /// through it leaves every vertex where the file put it. That is not a
    /// reason to skip it: it is what an animated pose is stated relative
    /// to, and it is the check that the skeleton was read correctly, since
    /// a bone composed onto the wrong parent or a quaternion read in the
    /// wrong order will not cancel [`Bone::pose_to_bone`].
    ///
    /// A bone naming a parent that does not come before it is an error,
    /// because a skeleton is stored parents-first and composing out of
    /// order would silently place a limb with a stale parent.
    pub fn bind_pose(&self, bytes: &'a [u8]) -> Result<Vec<Transform>> {
        let bones = self.bones(bytes)?;
        let mut pose: Vec<Transform> = Vec::with_capacity(bones.len());
        for (index, bone) in bones.iter().enumerate() {
            let local = bone.local();
            let placed = if bone.parent < 0 {
                local
            } else {
                let parent = usize::try_from(bone.parent).map_err(|_| Error::BoneOutOfOrder {
                    bone: index,
                    parent: bone.parent,
                })?;
                let Some(parent) = pose.get(parent) else {
                    return Err(Error::BoneOutOfOrder {
                        bone: index,
                        parent: bone.parent,
                    });
                };
                parent.times(&local)
            };
            pose.push(placed);
        }
        Ok(pose)
    }
}

#[derive(Debug, Clone)]
pub struct Vvd {
    pub checksum: i32,
    pub lod_vertex_counts: Vec<usize>,
    pub fixup_count: usize,
    pub fixup_data: Option<Range<usize>>,
    pub vertex_data: Range<usize>,
    pub tangent_data: Option<Range<usize>>,
}

/// One of a model's vertices, as the file stores it.
///
/// The bones and weights are what a pose moves the vertex by, and are read
/// here rather than left in the bytes because a vertex without them can only
/// be drawn in the model's rest pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub texcoord: [f32; 2],
    pub bones: [u8; 3],
    pub weights: [f32; 3],
    pub bone_count: u8,
}

/// Bytes one stored vertex takes: its weights, position, normal and
/// texture coordinate.
const VVD_VERTEX_SIZE: usize = 48;
const VVD_FIXUP_SIZE: usize = 12;

impl Vvd {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Self> {
        check_file_size(bytes, limits)?;
        let mut reader = Reader::new(bytes);
        let magic = reader.read_u32_le()?;
        if magic != VVD_MAGIC {
            return Err(Error::InvalidMagic {
                format: "VVD",
                value: magic,
            });
        }
        let version = reader.read_i32_le()?;
        if version != VVD_VERSION {
            return Err(Error::UnsupportedVersion {
                format: "VVD",
                value: version,
            });
        }
        let checksum = reader.read_i32_le()?;
        let lod_count = checked_count("VVD LOD", reader.read_i32_le()?, MAX_LODS)?;
        if lod_count == 0 {
            return Err(Error::InvalidCount {
                field: "VVD LOD",
                value: 0,
            });
        }
        let mut all_lod_counts = [0usize; MAX_LODS];
        for count in &mut all_lod_counts {
            *count = checked_count("VVD vertices", reader.read_i32_le()?, limits.max_vertices)?;
        }
        let lod_vertex_counts = all_lod_counts[..lod_count].to_vec();
        if lod_vertex_counts.windows(2).any(|pair| pair[1] > pair[0]) || lod_vertex_counts[0] == 0 {
            return Err(Error::InvalidLodVertexCounts);
        }
        let fixup_count = checked_count("VVD fixups", reader.read_i32_le()?, limits.max_items)?;
        let fixup_offset = reader.read_i32_le()?;
        let vertex_offset = reader.read_i32_le()?;
        let tangent_offset = reader.read_i32_le()?;
        let fixup_data = if fixup_count == 0 {
            None
        } else {
            Some(relative_range(
                bytes,
                0,
                fixup_offset,
                fixup_count,
                VVD_FIXUP_SIZE,
                "VVD fixups",
            )?)
        };
        let vertex_count = lod_vertex_counts[0];
        let vertex_data = relative_range(
            bytes,
            0,
            vertex_offset,
            vertex_count,
            VVD_VERTEX_SIZE,
            "VVD vertices",
        )?;
        let tangent_data = if tangent_offset == 0 {
            None
        } else {
            Some(relative_range(
                bytes,
                0,
                tangent_offset,
                vertex_count,
                16,
                "VVD tangents",
            )?)
        };
        Ok(Self {
            checksum,
            lod_vertex_counts,
            fixup_count,
            fixup_data,
            vertex_data,
            tangent_data,
        })
    }

    /// The model's vertices at one level of detail, in the order the meshes
    /// index them.
    ///
    /// A file does not store one run of vertices per level. It stores one
    /// run holding all of them and a table saying which stretches of it each
    /// level keeps, because the levels share most of their vertices and the
    /// coarser ones are built by leaving stretches out. Reading the run
    /// directly gives the right count and the wrong vertices for every level
    /// but the finest, and even the finest only by coincidence of the table
    /// usually listing it in order.
    pub fn vertices(&self, bytes: &[u8], lod: usize) -> Result<Vec<Vertex>> {
        let count = *self.lod_vertex_counts.get(lod).ok_or(Error::InvalidCount {
            field: "VVD LOD",
            value: lod as i32,
        })?;

        let stored = self
            .vertex_data
            .clone()
            .len()
            .checked_div(VVD_VERTEX_SIZE)
            .unwrap_or(0);
        let read = |index: usize| -> Result<Vertex> {
            if index >= stored {
                return Err(Error::InvalidCount {
                    field: "VVD vertex index",
                    value: index as i32,
                });
            }
            let at = self.vertex_data.start + index * VVD_VERTEX_SIZE;
            let mut reader = Reader::new(&bytes[at..at + VVD_VERTEX_SIZE]);
            let mut weights = [0.0f32; 3];
            for weight in &mut weights {
                *weight = reader.read_f32_le()?;
            }
            let mut bones = [0u8; 3];
            for bone in &mut bones {
                *bone = reader.read_u8()?;
            }
            let bone_count = reader.read_u8()?;
            let mut position = [0.0f32; 3];
            for axis in &mut position {
                *axis = reader.read_f32_le()?;
            }
            let mut normal = [0.0f32; 3];
            for axis in &mut normal {
                *axis = reader.read_f32_le()?;
            }
            let texcoord = [reader.read_f32_le()?, reader.read_f32_le()?];
            Ok(Vertex {
                position,
                normal,
                texcoord,
                bones,
                weights,
                bone_count,
            })
        };

        let Some(fixups) = &self.fixup_data else {
            // Nothing was left out, so the level's vertices are the first of
            // them.
            return (0..count).map(read).collect();
        };

        let mut vertices = Vec::with_capacity(count);
        for index in 0..self.fixup_count {
            let at = fixups.start + index * VVD_FIXUP_SIZE;
            let entry_lod = read_i32_at(bytes, at)?;
            let source = read_i32_at(bytes, at + 4)?;
            let run = read_i32_at(bytes, at + 8)?;
            // A stretch is kept by every level as coarse as the one it names,
            // which is why the comparison runs the way it looks backwards:
            // level zero is the finest.
            if entry_lod < lod as i32 {
                continue;
            }
            let source = checked_count("VVD fixup source", source, stored)?;
            let run = checked_count("VVD fixup run", run, stored)?;
            let end = source.checked_add(run).ok_or(Error::InvalidCount {
                field: "VVD fixup run",
                value: run as i32,
            })?;
            if end > stored {
                return Err(Error::InvalidCount {
                    field: "VVD fixup run",
                    value: end as i32,
                });
            }
            for vertex in source..end {
                vertices.push(read(vertex)?);
            }
        }

        if vertices.len() != count {
            return Err(Error::InvalidLodVertexCounts);
        }
        Ok(vertices)
    }
}

#[derive(Debug, Clone)]
pub struct Vtx {
    pub checksum: i32,
    pub lod_count: usize,
    pub body_part_count: usize,
    /// Where the body-part table sits, kept so the triangles can be walked
    /// again without re-reading the header.
    body_parts: Range<usize>,
    pub mesh_count: usize,
    pub strip_group_count: usize,
    pub strip_count: usize,
}

impl Vtx {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Self> {
        check_file_size(bytes, limits)?;
        let version = read_i32_at(bytes, 0)?;
        if version != VTX_VERSION {
            return Err(Error::UnsupportedVersion {
                format: "VTX",
                value: version,
            });
        }
        let checksum = read_i32_at(bytes, 16)?;
        let lod_count = checked_count("VTX LOD", read_i32_at(bytes, 20)?, MAX_LODS)?;
        if lod_count == 0 {
            return Err(Error::InvalidCount {
                field: "VTX LOD",
                value: 0,
            });
        }
        let replacement_lists = relative_range(
            bytes,
            0,
            read_i32_at(bytes, 24)?,
            lod_count,
            8,
            "VTX material replacement lists",
        )?;
        for list in 0..lod_count {
            let base = replacement_lists.start + list * 8;
            let count = checked_count(
                "VTX material replacements",
                read_i32_at(bytes, base)?,
                limits.max_items,
            )?;
            if count == 0 {
                continue;
            }
            let entries = relative_range(
                bytes,
                base,
                read_i32_at(bytes, base + 4)?,
                count,
                6,
                "VTX material replacements",
            )?;
            for entry in 0..count {
                let entry_base = entries.start + entry * 6;
                let name_offset = read_i32_at(bytes, entry_base + 2)?;
                let name = relative_offset(bytes, entry_base, name_offset, "VTX replacement name")?;
                c_string(bytes, name, "VTX replacement name")?;
            }
        }

        let body_part_count =
            checked_count("VTX body parts", read_i32_at(bytes, 28)?, limits.max_items)?;
        let body_parts = relative_range(
            bytes,
            0,
            read_i32_at(bytes, 32)?,
            body_part_count,
            8,
            "VTX body parts",
        )?;
        let mut totals = VtxTotals::default();
        for body_part in 0..body_part_count {
            let body_base = body_parts.start + body_part * 8;
            let model_count = checked_count(
                "VTX models",
                read_i32_at(bytes, body_base)?,
                limits.max_items,
            )?;
            let models = relative_range(
                bytes,
                body_base,
                read_i32_at(bytes, body_base + 4)?,
                model_count,
                8,
                "VTX models",
            )?;
            for model in 0..model_count {
                parse_vtx_model(
                    bytes,
                    models.start + model * 8,
                    lod_count,
                    limits,
                    &mut totals,
                )?;
            }
        }
        Ok(Self {
            checksum,
            lod_count,
            body_part_count,
            body_parts,
            mesh_count: totals.meshes,
            strip_group_count: totals.strip_groups,
            strip_count: totals.strips,
        })
    }
}

/// One mesh's triangles, as indices into the model's vertices for one
/// level of detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Triangles {
    pub body_part: usize,
    pub model: usize,
    pub mesh: usize,
    pub material: usize,
    pub indices: Vec<u32>,
}

/// A strip holding a plain list of triangles rather than a strip of them.
const STRIP_IS_TRILIST: u8 = 1;

/// The triangles a model draws at one level of detail.
///
/// This takes both files because neither holds the answer. The triangle
/// file numbers each mesh's vertices from zero and says which of them each
/// triangle uses; the model file says where that mesh's vertices sit in the
/// one run the vertex file stores, and which material the mesh draws with.
/// Joining them wrongly is the classic way to get a model that renders as a
/// cloud of triangles pulled from the wrong meshes.
pub fn triangles(
    mdl: &Mdl<'_>,
    mdl_bytes: &[u8],
    vtx: &Vtx,
    vtx_bytes: &[u8],
    lod: usize,
) -> Result<Vec<Triangles>> {
    let meshes = mdl.meshes(mdl_bytes)?;
    let mut out = Vec::new();
    let mut walked = 0usize;

    let body_parts = vtx.body_parts.clone();
    for part in 0..vtx.body_part_count {
        let part_at = body_parts.start + part * 8;
        let model_count =
            checked_count("VTX models", read_i32_at(vtx_bytes, part_at)?, usize::MAX)?;
        let models = relative_range(
            vtx_bytes,
            part_at,
            read_i32_at(vtx_bytes, part_at + 4)?,
            model_count,
            8,
            "VTX models",
        )?;
        for model in 0..model_count {
            let model_at = models.start + model * 8;
            let lod_count =
                checked_count("VTX model LOD", read_i32_at(vtx_bytes, model_at)?, MAX_LODS)?;
            if lod >= lod_count {
                return Err(Error::InvalidCount {
                    field: "VTX model LOD",
                    value: lod as i32,
                });
            }
            let lods = relative_range(
                vtx_bytes,
                model_at,
                read_i32_at(vtx_bytes, model_at + 4)?,
                lod_count,
                12,
                "VTX model LODs",
            )?;
            let lod_at = lods.start + lod * 12;
            let mesh_count =
                checked_count("VTX meshes", read_i32_at(vtx_bytes, lod_at)?, usize::MAX)?;
            let entries = relative_range(
                vtx_bytes,
                lod_at,
                read_i32_at(vtx_bytes, lod_at + 4)?,
                mesh_count,
                9,
                "VTX meshes",
            )?;
            for mesh in 0..mesh_count {
                // The two files walk their meshes in the same order, which
                // is what lets one supply what the other leaves out. A file
                // pair that disagrees on how many there are cannot be
                // joined at all, so it is refused rather than guessed at.
                let placement = meshes.get(walked).ok_or(Error::InvalidCount {
                    field: "VTX meshes beyond the model's own",
                    value: walked as i32,
                })?;
                walked += 1;
                let indices = mesh_triangles(vtx_bytes, entries.start + mesh * 9, placement)?;
                if indices.is_empty() {
                    continue;
                }
                out.push(Triangles {
                    body_part: placement.body_part,
                    model: placement.model,
                    mesh: placement.mesh,
                    material: placement.material,
                    indices,
                });
            }
        }
    }

    if walked != meshes.len() {
        return Err(Error::InvalidCount {
            field: "VTX meshes against the model's own",
            value: walked as i32,
        });
    }
    Ok(out)
}

fn mesh_triangles(bytes: &[u8], base: usize, placement: &Mesh) -> Result<Vec<u32>> {
    let group_count = checked_count("VTX strip groups", read_i32_at(bytes, base)?, usize::MAX)?;
    let mdl49 = *bytes.get(base + 8).ok_or(Error::InvalidRange {
        field: "VTX mesh flags",
        offset: base + 8,
        size: 1,
    })? & 0x80
        != 0;
    let stride = if mdl49 { 33 } else { 25 };
    let groups = relative_range(
        bytes,
        base,
        read_i32_at(bytes, base + 4)?,
        group_count,
        stride,
        "VTX strip groups",
    )?;

    let mut out = Vec::new();
    for group in 0..group_count {
        let group_at = groups.start + group * stride;
        let vertex_count = checked_count(
            "VTX strip-group vertices",
            read_i32_at(bytes, group_at)?,
            usize::MAX,
        )?;
        let vertices = relative_range(
            bytes,
            group_at,
            read_i32_at(bytes, group_at + 4)?,
            vertex_count,
            9,
            "VTX strip-group vertices",
        )?;
        let index_count = checked_count(
            "VTX strip-group indices",
            read_i32_at(bytes, group_at + 8)?,
            usize::MAX,
        )?;
        let indices = relative_range(
            bytes,
            group_at,
            read_i32_at(bytes, group_at + 12)?,
            index_count,
            2,
            "VTX strip-group indices",
        )?;

        // A strip-group index names one of the group's own vertices, and
        // that vertex names one of the mesh's. Both hops are needed: the
        // group's list is what the strips were built over and the mesh's
        // numbering is what the vertex file uses.
        let resolve = |index: usize| -> Result<u32> {
            let at = vertices.start + index * 9;
            let original = u16::from_le_bytes([
                *bytes.get(at + 4).ok_or(Error::InvalidRange {
                    field: "VTX strip-group vertex",
                    offset: at + 4,
                    size: 2,
                })?,
                *bytes.get(at + 5).ok_or(Error::InvalidRange {
                    field: "VTX strip-group vertex",
                    offset: at + 4,
                    size: 2,
                })?,
            ]);
            if usize::from(original) >= placement.vertex_count {
                return Err(Error::InvalidVertexIndex {
                    value: original,
                    vertex_count: placement.vertex_count,
                });
            }
            u32::try_from(placement.vertex_base + usize::from(original))
                .map_err(|_| Error::SizeOverflow)
        };
        let at = |slot: usize| -> Result<usize> {
            let offset = indices.start + slot * 2;
            let value = u16::from_le_bytes([
                *bytes.get(offset).ok_or(Error::InvalidRange {
                    field: "VTX strip-group index",
                    offset,
                    size: 2,
                })?,
                *bytes.get(offset + 1).ok_or(Error::InvalidRange {
                    field: "VTX strip-group index",
                    offset,
                    size: 2,
                })?,
            ]);
            if usize::from(value) >= vertex_count {
                return Err(Error::InvalidVertexIndex {
                    value,
                    vertex_count,
                });
            }
            Ok(usize::from(value))
        };

        let group_flags = *bytes.get(group_at + 24).ok_or(Error::InvalidRange {
            field: "VTX strip-group flags",
            offset: group_at + 24,
            size: 1,
        })?;
        let strip49 = mdl49 || group_flags & 0x80 != 0;
        let strip_stride = if strip49 { 35 } else { 27 };
        let strip_count =
            checked_count("VTX strips", read_i32_at(bytes, group_at + 16)?, usize::MAX)?;
        let strips = relative_range(
            bytes,
            group_at,
            read_i32_at(bytes, group_at + 20)?,
            strip_count,
            strip_stride,
            "VTX strips",
        )?;
        for strip in 0..strip_count {
            let strip_at = strips.start + strip * strip_stride;
            let count = checked_count(
                "VTX strip indices",
                read_i32_at(bytes, strip_at)?,
                index_count,
            )?;
            let first = checked_count(
                "VTX strip index offset",
                read_i32_at(bytes, strip_at + 4)?,
                index_count,
            )?;
            if first.checked_add(count).is_none_or(|end| end > index_count) {
                return Err(Error::InvalidRange {
                    field: "VTX strip indices",
                    offset: first,
                    size: count,
                });
            }
            let flags = *bytes.get(strip_at + 18).ok_or(Error::InvalidRange {
                field: "VTX strip flags",
                offset: strip_at + 18,
                size: 1,
            })?;
            if flags & STRIP_IS_TRILIST != 0 {
                for triangle in 0..count / 3 {
                    for corner in 0..3 {
                        out.push(resolve(at(first + triangle * 3 + corner)?)?);
                    }
                }
                continue;
            }
            // A strip shares two corners with the triangle before it and
            // alternates winding, so every other triangle is emitted the
            // other way round to keep them all facing the same side.
            for triangle in 0..count.saturating_sub(2) {
                let corners = if triangle % 2 == 0 {
                    [0usize, 1, 2]
                } else {
                    [1usize, 0, 2]
                };
                let resolved: [usize; 3] = [
                    at(first + triangle)?,
                    at(first + triangle + 1)?,
                    at(first + triangle + 2)?,
                ];
                if resolved[0] == resolved[1]
                    || resolved[1] == resolved[2]
                    || resolved[0] == resolved[2]
                {
                    // Strips join runs with degenerate triangles, which
                    // draw nothing and are dropped rather than passed on.
                    continue;
                }
                for corner in corners {
                    out.push(resolve(resolved[corner])?);
                }
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct Phy<'a> {
    pub id: i32,
    pub checksum: i32,
    pub solid_ranges: Vec<Range<usize>>,
    pub properties_text: &'a [u8],
    pub properties: Option<Document>,
}

impl<'a> Phy<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        check_file_size(bytes, limits)?;
        let mut reader = Reader::new(bytes);
        let header_size = reader.read_i32_le()?;
        if header_size != 16 {
            return Err(Error::InvalidLength {
                format: "PHY header",
                value: header_size,
            });
        }
        let id = reader.read_i32_le()?;
        let solid_count = checked_count("PHY solids", reader.read_i32_le()?, limits.max_solids)?;
        let checksum = reader.read_i32_le()?;
        let mut solid_ranges = Vec::with_capacity(solid_count);
        for _ in 0..solid_count {
            let size = positive_usize("PHY solid size", reader.read_i32_le()?)?;
            let start = reader.position();
            reader.skip(size)?;
            solid_ranges.push(start..reader.position());
        }
        let properties_text = trim_property_text(&bytes[reader.position()..]);
        let properties = if properties_text.is_empty() {
            None
        } else {
            Some(parse_keyvalues(properties_text, ParseOptions::default())?)
        };
        Ok(Self {
            id,
            checksum,
            solid_ranges,
            properties_text,
            properties,
        })
    }
}

#[derive(Default)]
struct VtxTotals {
    meshes: usize,
    strip_groups: usize,
    strips: usize,
}

fn parse_vtx_model(
    bytes: &[u8],
    base: usize,
    file_lod_count: usize,
    limits: Limits,
    totals: &mut VtxTotals,
) -> Result<()> {
    let lod_count = checked_count("VTX model LOD", read_i32_at(bytes, base)?, MAX_LODS)?;
    if lod_count == 0 || lod_count > file_lod_count {
        return Err(Error::InvalidCount {
            field: "VTX model LOD",
            value: i32::try_from(lod_count).unwrap_or(i32::MAX),
        });
    }
    let lods = relative_range(
        bytes,
        base,
        read_i32_at(bytes, base + 4)?,
        lod_count,
        12,
        "VTX model LODs",
    )?;
    for lod in 0..lod_count {
        let lod_base = lods.start + lod * 12;
        let mesh_count = checked_count(
            "VTX meshes",
            read_i32_at(bytes, lod_base)?,
            limits.max_items,
        )?;
        totals.meshes = checked_total(totals.meshes, mesh_count, limits.max_items, "VTX meshes")?;
        let meshes = relative_range(
            bytes,
            lod_base,
            read_i32_at(bytes, lod_base + 4)?,
            mesh_count,
            9,
            "VTX meshes",
        )?;
        for mesh in 0..mesh_count {
            parse_vtx_mesh(bytes, meshes.start + mesh * 9, limits, totals)?;
        }
    }
    Ok(())
}

fn parse_vtx_mesh(bytes: &[u8], base: usize, limits: Limits, totals: &mut VtxTotals) -> Result<()> {
    let strip_group_count = checked_count(
        "VTX strip groups",
        read_i32_at(bytes, base)?,
        limits.max_items,
    )?;
    totals.strip_groups = checked_total(
        totals.strip_groups,
        strip_group_count,
        limits.max_items,
        "VTX strip groups",
    )?;
    let mdl49 = *bytes.get(base + 8).ok_or(Error::InvalidRange {
        field: "VTX mesh flags",
        offset: base + 8,
        size: 1,
    })? & 0x80
        != 0;
    let stride = if mdl49 { 33 } else { 25 };
    let groups = relative_range(
        bytes,
        base,
        read_i32_at(bytes, base + 4)?,
        strip_group_count,
        stride,
        "VTX strip groups",
    )?;
    for group in 0..strip_group_count {
        parse_vtx_strip_group(bytes, groups.start + group * stride, mdl49, limits, totals)?;
    }
    Ok(())
}

fn parse_vtx_strip_group(
    bytes: &[u8],
    base: usize,
    mdl49: bool,
    limits: Limits,
    totals: &mut VtxTotals,
) -> Result<()> {
    let vertex_count = checked_count(
        "VTX strip-group vertices",
        read_i32_at(bytes, base)?,
        limits.max_vertices,
    )?;
    relative_range(
        bytes,
        base,
        read_i32_at(bytes, base + 4)?,
        vertex_count,
        9,
        "VTX strip-group vertices",
    )?;
    let index_count = checked_count(
        "VTX strip-group indices",
        read_i32_at(bytes, base + 8)?,
        limits.max_items,
    )?;
    let indices = relative_range(
        bytes,
        base,
        read_i32_at(bytes, base + 12)?,
        index_count,
        2,
        "VTX strip-group indices",
    )?;
    for offset in (indices.start..indices.end).step_by(2) {
        let value = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        if usize::from(value) >= vertex_count && vertex_count != 0 {
            return Err(Error::InvalidVertexIndex {
                value,
                vertex_count,
            });
        }
    }
    let strip_count = checked_count(
        "VTX strips",
        read_i32_at(bytes, base + 16)?,
        limits.max_items,
    )?;
    totals.strips = checked_total(totals.strips, strip_count, limits.max_items, "VTX strips")?;
    let group_flags = *bytes.get(base + 24).ok_or(Error::InvalidRange {
        field: "VTX strip-group flags",
        offset: base + 24,
        size: 1,
    })?;
    let strip49 = mdl49 || group_flags & 0x80 != 0;
    let strip_stride = if strip49 { 35 } else { 27 };
    let strips = relative_range(
        bytes,
        base,
        read_i32_at(bytes, base + 20)?,
        strip_count,
        strip_stride,
        "VTX strips",
    )?;
    for strip in 0..strip_count {
        let strip_base = strips.start + strip * strip_stride;
        let strip_indices = checked_count(
            "VTX strip indices",
            read_i32_at(bytes, strip_base)?,
            index_count,
        )?;
        let index_offset = checked_count(
            "VTX strip index offset",
            read_i32_at(bytes, strip_base + 4)?,
            index_count,
        )?;
        if index_offset
            .checked_add(strip_indices)
            .is_none_or(|end| end > index_count)
        {
            return Err(Error::InvalidRange {
                field: "VTX strip indices",
                offset: index_offset,
                size: strip_indices,
            });
        }
        let strip_vertices = checked_count(
            "VTX strip vertices",
            read_i32_at(bytes, strip_base + 8)?,
            vertex_count,
        )?;
        let vertex_offset = checked_count(
            "VTX strip vertex offset",
            read_i32_at(bytes, strip_base + 12)?,
            vertex_count,
        )?;
        if vertex_offset
            .checked_add(strip_vertices)
            .is_none_or(|end| end > vertex_count)
        {
            return Err(Error::InvalidRange {
                field: "VTX strip vertices",
                offset: vertex_offset,
                size: strip_vertices,
            });
        }
        let changes = checked_count(
            "VTX bone state changes",
            read_i32_at(bytes, strip_base + 19)?,
            limits.max_items,
        )?;
        if changes != 0 {
            relative_range(
                bytes,
                strip_base,
                read_i32_at(bytes, strip_base + 23)?,
                changes,
                8,
                "VTX bone state changes",
            )?;
        }
    }
    Ok(())
}

fn check_file_size(bytes: &[u8], limits: Limits) -> Result<()> {
    if bytes.len() > limits.max_file_size {
        Err(Error::FileTooLarge {
            size: bytes.len(),
            limit: limits.max_file_size,
        })
    } else {
        Ok(())
    }
}

fn validate_count_offset(
    bytes: &[u8],
    field: &'static str,
    count_at: usize,
    offset_at: usize,
    limit: usize,
) -> Result<()> {
    let count = checked_count(field, read_i32_at(bytes, count_at)?, limit)?;
    let offset = read_i32_at(bytes, offset_at)?;
    if count != 0 {
        relative_range(bytes, 0, offset, count, 1, field)?;
    }
    Ok(())
}

fn read_i32_at(bytes: &[u8], offset: usize) -> Result<i32> {
    let mut reader = Reader::with_position(bytes, offset)?;
    Ok(reader.read_i32_le()?)
}

fn positive_usize(field: &'static str, value: i32) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::InvalidOffset { field, value })
}

fn checked_count(field: &'static str, value: i32, limit: usize) -> Result<usize> {
    let count = usize::try_from(value).map_err(|_| Error::InvalidCount { field, value })?;
    if count > limit {
        Err(Error::CountLimitExceeded {
            field,
            value: count,
            limit,
        })
    } else {
        Ok(count)
    }
}

fn checked_total(
    current: usize,
    additional: usize,
    limit: usize,
    field: &'static str,
) -> Result<usize> {
    let value = current.checked_add(additional).ok_or(Error::SizeOverflow)?;
    if value > limit {
        Err(Error::CountLimitExceeded {
            field,
            value,
            limit,
        })
    } else {
        Ok(value)
    }
}

fn relative_offset(bytes: &[u8], base: usize, relative: i32, field: &'static str) -> Result<usize> {
    // VTX commonly places shared tables before the structure that references
    // them, so its relative offsets are signed in practice as well as in C++.
    let offset = if relative >= 0 {
        base.checked_add(relative as usize)
    } else {
        base.checked_sub(relative.unsigned_abs() as usize)
    }
    .ok_or(Error::InvalidOffset {
        field,
        value: relative,
    })?;
    if offset > bytes.len() {
        Err(Error::InvalidOffset {
            field,
            value: relative,
        })
    } else {
        Ok(offset)
    }
}

fn relative_range(
    bytes: &[u8],
    base: usize,
    relative: i32,
    count: usize,
    stride: usize,
    field: &'static str,
) -> Result<Range<usize>> {
    let start = relative_offset(bytes, base, relative, field)?;
    let size = count.checked_mul(stride).ok_or(Error::SizeOverflow)?;
    let end = start.checked_add(size).ok_or(Error::SizeOverflow)?;
    if end > bytes.len() {
        Err(Error::InvalidRange {
            field,
            offset: start,
            size,
        })
    } else {
        Ok(start..end)
    }
}

fn c_string<'a>(bytes: &'a [u8], offset: usize, field: &'static str) -> Result<&'a str> {
    let tail = bytes.get(offset..).ok_or(Error::InvalidRange {
        field,
        offset,
        size: 1,
    })?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::UnterminatedString { field, offset })?;
    std::str::from_utf8(&tail[..end]).map_err(|_| Error::InvalidUtf8 { field })
}

fn trim_property_text(mut bytes: &[u8]) -> &[u8] {
    while bytes
        .last()
        .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == 0)
    {
        bytes = &bytes[..bytes.len() - 1];
    }
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn parses_minimal_mdl_and_bounds_embedded_keyvalues() {
        let mut bytes = vec![0; 400];
        bytes[0..4].copy_from_slice(b"IDST");
        put_i32(&mut bytes, 4, 49);
        put_i32(&mut bytes, 8, 1234);
        bytes[12..17].copy_from_slice(b"test\0");
        put_i32(&mut bytes, 76, 400);
        bytes[350..368].copy_from_slice(b"root { key value }");
        put_i32(&mut bytes, 312, 350);
        put_i32(&mut bytes, 316, 18);
        let mdl = Mdl::parse(&bytes).unwrap();
        assert_eq!(mdl.name, "test");
        assert_eq!(mdl.checksum, 1234);
        assert_eq!(mdl.keyvalues.unwrap(), b"root { key value }");
    }

    #[test]
    fn parses_vvd_and_rejects_bad_lod_order() {
        let mut bytes = vec![0; 64 + 48 * 3 + 16 * 3];
        bytes[0..4].copy_from_slice(b"IDSV");
        put_i32(&mut bytes, 4, 4);
        put_i32(&mut bytes, 8, 42);
        put_i32(&mut bytes, 12, 2);
        put_i32(&mut bytes, 16, 3);
        put_i32(&mut bytes, 20, 2);
        put_i32(&mut bytes, 52, 64);
        put_i32(&mut bytes, 56, 64);
        put_i32(&mut bytes, 60, 64 + 48 * 3);
        let vvd = Vvd::parse(&bytes).unwrap();
        assert_eq!(vvd.lod_vertex_counts, [3, 2]);

        put_i32(&mut bytes, 20, 4);
        assert!(matches!(
            Vvd::parse(&bytes),
            Err(Error::InvalidLodVertexCounts)
        ));
    }

    #[test]
    fn parses_empty_vtx_hierarchy() {
        let mut bytes = vec![0; 44];
        put_i32(&mut bytes, 0, 7);
        put_i32(&mut bytes, 16, 42);
        put_i32(&mut bytes, 20, 1);
        put_i32(&mut bytes, 24, 36);
        put_i32(&mut bytes, 28, 0);
        put_i32(&mut bytes, 32, 44);
        let vtx = Vtx::parse(&bytes).unwrap();
        assert_eq!(vtx.lod_count, 1);
        assert_eq!(vtx.mesh_count, 0);
    }

    #[test]
    fn parses_phy_solids_and_properties() {
        let mut bytes = vec![0; 16];
        put_i32(&mut bytes, 0, 16);
        put_i32(&mut bytes, 8, 1);
        put_i32(&mut bytes, 12, 55);
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.extend_from_slice(b"IVPS");
        bytes.extend_from_slice(b"solid { index 0 }\0");
        let phy = Phy::parse(&bytes).unwrap();
        assert_eq!(phy.solid_ranges.len(), 1);
        assert_eq!(phy.properties.unwrap().roots().count(), 1);
    }

    #[test]
    fn rejects_truncated_ranges() {
        let mut bytes = vec![0; 64];
        bytes[0..4].copy_from_slice(b"IDSV");
        put_i32(&mut bytes, 4, 4);
        put_i32(&mut bytes, 12, 1);
        put_i32(&mut bytes, 16, 10);
        put_i32(&mut bytes, 56, 64);
        assert!(matches!(
            Vvd::parse(&bytes),
            Err(Error::InvalidRange { .. })
        ));
    }
}
