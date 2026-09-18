//! Strict structural parser for Source 1 VTF 7.0 through 7.5 textures.

use source_binary::Reader;
use std::fmt;
use std::ops::Range;

pub const VTF_SIGNATURE: &[u8; 4] = b"VTF\0";
pub const VTF_MAJOR_VERSION: u32 = 7;
pub const VTF_MAX_MINOR_VERSION: u32 = 5;
pub const TEXTUREFLAGS_ENVMAP: u32 = 0x0000_4000;
pub const RSRCF_HAS_NO_DATA_CHUNK: u8 = 0x02;
pub const LEGACY_LOW_RES_IMAGE: [u8; 3] = [0x01, 0, 0];
pub const LEGACY_IMAGE: [u8; 3] = [0x30, 0, 0];

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_dimension: u16,
    pub max_frames: u16,
    pub max_resources: u32,
    pub max_decoded_image_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 1024 * 1024 * 1024,
            max_dimension: 16_384,
            max_frames: 4096,
            max_resources: 32,
            max_decoded_image_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Header {
    pub version_major: u32,
    pub version_minor: u32,
    pub header_size: u32,
    pub width: u16,
    pub height: u16,
    pub flags: u32,
    pub frame_count: u16,
    pub first_frame: u16,
    pub reflectivity: [f32; 3],
    pub bump_scale: f32,
    pub image_format: ImageFormat,
    pub mip_count: u8,
    pub low_res_image_format: ImageFormat,
    pub low_res_width: u8,
    pub low_res_height: u8,
    pub depth: u16,
    pub resource_count: u32,
}

impl Header {
    pub fn is_cube_map(&self) -> bool {
        self.flags & TEXTUREFLAGS_ENVMAP != 0
    }

    pub fn face_count(&self) -> usize {
        if self.is_cube_map() {
            // VTF 7.1 through 7.4 stored the seventh spheremap fallback.
            // Versions 7.0 and 7.5 store only the six cube faces; this is the
            // same compatibility rule used by CVTFTexture::ImageFileInfo.
            if (1..=4).contains(&self.version_minor) {
                7
            } else {
                6
            }
        } else {
            1
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageFormat(pub i32);

impl ImageFormat {
    pub const UNKNOWN: Self = Self(-1);
    pub const COUNT: i32 = 41;

    // The formats HL2's shipped materials are actually stored in, named so a
    // consumer deciding how to upload one does not match on magic numbers.
    // The values are `IMAGE_FORMAT_*` in `public/bitmap/imageformat.h`.
    pub const RGBA8888: Self = Self(0);
    pub const ABGR8888: Self = Self(1);
    pub const RGB888: Self = Self(2);
    pub const BGR888: Self = Self(3);
    pub const I8: Self = Self(5);
    pub const IA88: Self = Self(6);
    pub const A8: Self = Self(8);
    pub const ARGB8888: Self = Self(11);
    pub const BGRA8888: Self = Self(12);
    pub const DXT1: Self = Self(13);
    pub const DXT3: Self = Self(14);
    pub const DXT5: Self = Self(15);
    pub const BGRX8888: Self = Self(16);
    pub const DXT1_ONEBITALPHA: Self = Self(20);
    pub const UV88: Self = Self(22);

    /// The block edge a format encodes in, or one for the uncompressed
    /// formats. A block-compressed level is always whole blocks, which is why
    /// a 2x2 mip of a DXT texture still occupies a full 4x4 block.
    pub fn block_extent(self) -> usize {
        match self {
            Self::DXT1 | Self::DXT3 | Self::DXT5 | Self::DXT1_ONEBITALPHA => 4,
            _ => match self.0 {
                // The remaining block-compressed formats: ATI1N/ATI2N and
                // the linear DXT aliases.
                37..=40 => 4,
                _ => 1,
            },
        }
    }

    pub fn is_block_compressed(self) -> bool {
        self.block_extent() != 1
    }

    pub fn is_known(self) -> bool {
        (0..Self::COUNT).contains(&self.0)
    }

    pub fn encoded_len(self, width: usize, height: usize, depth: usize) -> Option<usize> {
        let pixels = width.checked_mul(height)?.checked_mul(depth)?;
        let bytes_per_pixel = match self.0 {
            0 | 1 | 11 | 12 | 16 | 23 | 26 | 27 | 31..=33 | 35 | 36 => 4,
            2 | 3 | 9 | 10 => 3,
            4 | 6 | 17..=19 | 21 | 22 | 30 | 34 => 2,
            5 | 7 | 8 => 1,
            24 | 25 => 8,
            28 => 12,
            29 => 16,
            13 | 20 | 38 | 39 => {
                return block_encoded_len(width, height, depth, 8);
            }
            14 | 15 | 37 | 40 => {
                return block_encoded_len(width, height, depth, 16);
            }
            _ => return None,
        };
        pixels.checked_mul(bytes_per_pixel)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub kind: [u8; 3],
    pub flags: u8,
    /// Inline value for `RSRCF_HAS_NO_DATA_CHUNK`, otherwise a file offset.
    pub data: u32,
    /// Payload bytes. Extended resources exclude their four-byte length prefix.
    pub payload: Option<Range<usize>>,
}

#[derive(Debug, Clone)]
pub struct Texture<'a> {
    header: Header,
    resources: Vec<Resource>,
    bytes: &'a [u8],
    high_res_image_size: usize,
    low_res_image_size: usize,
}

impl<'a> Texture<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::default())
    }

    pub fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        if bytes.len() > limits.max_file_size {
            return Err(Error::FileTooLarge {
                size: bytes.len(),
                limit: limits.max_file_size,
            });
        }
        let header = parse_header(bytes)?;
        validate_header(&header, bytes.len(), limits)?;
        let high_res_image_size = high_res_image_size(&header)?;
        if high_res_image_size > limits.max_decoded_image_bytes {
            return Err(Error::DecodedImageTooLarge {
                size: high_res_image_size,
                limit: limits.max_decoded_image_bytes,
            });
        }
        let low_res_image_size = low_res_image_size(&header)?;
        let resources = if header.version_minor >= 3 {
            parse_resources(bytes, &header, high_res_image_size, low_res_image_size)?
        } else {
            legacy_resources(bytes, &header, high_res_image_size, low_res_image_size)?
        };
        Ok(Self {
            header,
            resources,
            bytes,
            high_res_image_size,
            low_res_image_size,
        })
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    pub fn high_res_image_size(&self) -> usize {
        self.high_res_image_size
    }

    pub fn low_res_image_size(&self) -> usize {
        self.low_res_image_size
    }

    pub fn resource_payload(&self, kind: [u8; 3]) -> Option<&'a [u8]> {
        self.resources
            .iter()
            .find(|resource| resource.kind == kind)
            .and_then(|resource| resource.payload.clone())
            .and_then(|range| self.bytes.get(range))
    }

    /// The pixel dimensions of one mip level.
    ///
    /// Each level is half the one before it, stopping at one rather than
    /// reaching zero, which is what makes the smallest level a single pixel
    /// on its longest edge.
    pub fn level_extent(&self, mip: usize) -> Result<(usize, usize, usize)> {
        if mip >= usize::from(self.header.mip_count) {
            return Err(Error::InvalidMipLevel {
                mip,
                count: self.header.mip_count,
            });
        }
        let shrink = |value: u16| (usize::from(value) >> mip).max(1);
        Ok((
            shrink(self.header.width),
            shrink(self.header.height),
            shrink(self.header.depth),
        ))
    }

    /// The encoded size of one mip level, for one face of one frame.
    pub fn level_len(&self, mip: usize) -> Result<usize> {
        let (width, height, depth) = self.level_extent(mip)?;
        self.header
            .image_format
            .encoded_len(width, height, depth)
            .ok_or(Error::InvalidImageFormat(self.header.image_format.0))
    }

    /// The encoded bytes of one mip level of one face of one frame, exactly
    /// as the file stores them.
    ///
    /// The file's order is not the order the engine keeps in memory. On disk
    /// the levels run smallest first, with every frame and face of a level
    /// together before the next larger level begins, which is what let the
    /// original loader read a reduced texture by stopping early. A GPU upload
    /// wants the opposite order, so callers take the levels through
    /// [`Self::mip_chain`] rather than reversing this themselves.
    pub fn level(&self, mip: usize, frame: usize, face: usize) -> Result<&'a [u8]> {
        let faces = self.header.face_count();
        let frames = usize::from(self.header.frame_count);
        if frame >= frames {
            return Err(Error::InvalidFrame {
                frame,
                count: self.header.frame_count,
            });
        }
        if face >= faces {
            return Err(Error::InvalidFace { face, count: faces });
        }

        let image = self
            .resource_payload(LEGACY_IMAGE)
            .ok_or(Error::MissingImageResource)?;

        // Skip every level smaller than this one, each of which holds one
        // run per frame and face.
        let mut offset = 0usize;
        for smaller in (mip + 1..usize::from(self.header.mip_count)).rev() {
            let size = self.level_len(smaller)?;
            offset = offset
                .checked_add(
                    size.checked_mul(frames)
                        .and_then(|size| size.checked_mul(faces))
                        .ok_or(Error::SizeOverflow)?,
                )
                .ok_or(Error::SizeOverflow)?;
        }

        let size = self.level_len(mip)?;
        let within = frame
            .checked_mul(faces)
            .and_then(|index| index.checked_add(face))
            .and_then(|index| index.checked_mul(size))
            .ok_or(Error::SizeOverflow)?;
        let start = offset.checked_add(within).ok_or(Error::SizeOverflow)?;
        let end = start.checked_add(size).ok_or(Error::SizeOverflow)?;

        image.get(start..end).ok_or(Error::ImageDataTruncated {
            expected: end,
            available: image.len(),
        })
    }

    /// Every mip level of one face of one frame, largest first, which is the
    /// order a GPU texture's levels are numbered in.
    pub fn mip_chain(&self, frame: usize, face: usize) -> Result<Vec<&'a [u8]>> {
        (0..usize::from(self.header.mip_count))
            .map(|mip| self.level(mip, frame, face))
            .collect()
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    FileTooLarge {
        size: usize,
        limit: usize,
    },
    InvalidSignature([u8; 4]),
    UnsupportedVersion {
        major: u32,
        minor: u32,
    },
    InvalidHeaderSize {
        size: u32,
        minimum: usize,
    },
    InvalidDimensions {
        width: u16,
        height: u16,
        depth: u16,
    },
    DimensionLimitExceeded {
        value: u16,
        limit: u16,
    },
    InvalidFrameCount(u16),
    FrameLimitExceeded {
        value: u16,
        limit: u16,
    },
    InvalidMipCount {
        value: u8,
        maximum: u8,
    },
    InvalidImageFormat(i32),
    InvalidLowResImageFormat(i32),
    InvalidCubeMap,
    ResourceLimitExceeded {
        value: u32,
        limit: u32,
    },
    ResourceTableOutOfRange,
    DuplicateResource([u8; 3]),
    InvalidResourceOffset {
        kind: [u8; 3],
        offset: u32,
    },
    InvalidResourceLength {
        kind: [u8; 3],
        offset: u32,
        size: u32,
    },
    MissingImageResource,
    ImageDataTruncated {
        expected: usize,
        available: usize,
    },
    LowResDataTruncated {
        expected: usize,
        available: usize,
    },
    DecodedImageTooLarge {
        size: usize,
        limit: usize,
    },
    InvalidMipLevel {
        mip: usize,
        count: u8,
    },
    InvalidFrame {
        frame: usize,
        count: u16,
    },
    InvalidFace {
        face: usize,
        count: usize,
    },
    SizeOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::FileTooLarge { size, limit } => {
                write!(f, "VTF file size {size} exceeds limit {limit}")
            }
            Self::InvalidSignature(value) => write!(f, "invalid VTF signature {value:?}"),
            Self::UnsupportedVersion { major, minor } => {
                write!(f, "unsupported VTF version {major}.{minor}")
            }
            Self::InvalidHeaderSize { size, minimum } => {
                write!(f, "VTF header size {size} is smaller than {minimum}")
            }
            Self::InvalidDimensions {
                width,
                height,
                depth,
            } => {
                write!(f, "invalid VTF dimensions {width}x{height}x{depth}")
            }
            Self::DimensionLimitExceeded { value, limit } => {
                write!(f, "VTF dimension {value} exceeds limit {limit}")
            }
            Self::InvalidFrameCount(value) => write!(f, "invalid VTF frame count {value}"),
            Self::FrameLimitExceeded { value, limit } => {
                write!(f, "VTF frame count {value} exceeds limit {limit}")
            }
            Self::InvalidMipCount { value, maximum } => {
                write!(f, "invalid VTF mip count {value}; maximum is {maximum}")
            }
            Self::InvalidImageFormat(value) => write!(f, "invalid VTF image format {value}"),
            Self::InvalidLowResImageFormat(value) => {
                write!(f, "invalid VTF low-resolution image format {value}")
            }
            Self::InvalidCubeMap => write!(f, "VTF cube map must be square and non-volume"),
            Self::ResourceLimitExceeded { value, limit } => {
                write!(f, "VTF resource count {value} exceeds limit {limit}")
            }
            Self::ResourceTableOutOfRange => write!(f, "VTF resource table is out of range"),
            Self::DuplicateResource(kind) => write!(f, "duplicate VTF resource {kind:?}"),
            Self::InvalidResourceOffset { kind, offset } => {
                write!(f, "VTF resource {kind:?} has invalid offset {offset}")
            }
            Self::InvalidResourceLength { kind, offset, size } => write!(
                f,
                "VTF resource {kind:?} has invalid payload range {offset}+{size}"
            ),
            Self::MissingImageResource => write!(f, "VTF has no high-resolution image resource"),
            Self::ImageDataTruncated {
                expected,
                available,
            } => write!(
                f,
                "VTF image data is truncated: expected {expected} bytes, found {available}"
            ),
            Self::LowResDataTruncated {
                expected,
                available,
            } => write!(
                f,
                "VTF thumbnail is truncated: expected {expected} bytes, found {available}"
            ),
            Self::DecodedImageTooLarge { size, limit } => {
                write!(f, "VTF decoded image size {size} exceeds limit {limit}")
            }
            Self::InvalidMipLevel { mip, count } => {
                write!(f, "VTF mip level {mip} is outside the {count} it has")
            }
            Self::InvalidFrame { frame, count } => {
                write!(f, "VTF frame {frame} is outside the {count} it has")
            }
            Self::InvalidFace { face, count } => {
                write!(f, "VTF face {face} is outside the {count} it has")
            }
            Self::SizeOverflow => write!(f, "VTF size arithmetic overflow"),
        }
    }
}

impl std::error::Error for Error {}

impl From<source_binary::Error> for Error {
    fn from(value: source_binary::Error) -> Self {
        Self::Binary(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn parse_header(bytes: &[u8]) -> Result<Header> {
    let mut reader = Reader::new(bytes);
    let signature: [u8; 4] = reader.take(4)?.try_into().expect("four-byte slice");
    if &signature != VTF_SIGNATURE {
        return Err(Error::InvalidSignature(signature));
    }
    let version_major = reader.read_u32_le()?;
    let version_minor = reader.read_u32_le()?;
    if version_major != VTF_MAJOR_VERSION || version_minor > VTF_MAX_MINOR_VERSION {
        return Err(Error::UnsupportedVersion {
            major: version_major,
            minor: version_minor,
        });
    }
    let header_size = reader.read_u32_le()?;
    let width = reader.read_u16_le()?;
    let height = reader.read_u16_le()?;
    let flags = reader.read_u32_le()?;
    let frame_count = reader.read_u16_le()?;
    let first_frame = reader.read_u16_le()?;
    reader.skip(4)?;
    let reflectivity = [
        reader.read_f32_le()?,
        reader.read_f32_le()?,
        reader.read_f32_le()?,
    ];
    reader.skip(4)?;
    let bump_scale = reader.read_f32_le()?;
    let image_format = ImageFormat(reader.read_i32_le()?);
    let mip_count = reader.read_u8()?;
    let low_res_image_format = ImageFormat(reader.read_i32_le()?);
    let low_res_width = reader.read_u8()?;
    let low_res_height = reader.read_u8()?;
    let depth = if version_minor >= 2 {
        reader.read_u16_le()?
    } else {
        1
    };
    let resource_count = if version_minor >= 3 {
        reader.skip(3)?;
        reader.read_u32_le()?
    } else {
        0
    };
    Ok(Header {
        version_major,
        version_minor,
        header_size,
        width,
        height,
        flags,
        frame_count,
        first_frame,
        reflectivity,
        bump_scale,
        image_format,
        mip_count,
        low_res_image_format,
        low_res_width,
        low_res_height,
        depth,
        resource_count,
    })
}

fn validate_header(header: &Header, file_size: usize, limits: Limits) -> Result<()> {
    let minimum = if header.version_minor <= 1 { 64 } else { 80 };
    let header_size = header.header_size as usize;
    if header_size < minimum || header_size > file_size {
        return Err(Error::InvalidHeaderSize {
            size: header.header_size,
            minimum,
        });
    }
    if header.width == 0 || header.height == 0 || header.depth == 0 {
        return Err(Error::InvalidDimensions {
            width: header.width,
            height: header.height,
            depth: header.depth,
        });
    }
    for value in [header.width, header.height, header.depth] {
        if value > limits.max_dimension {
            return Err(Error::DimensionLimitExceeded {
                value,
                limit: limits.max_dimension,
            });
        }
    }
    if header.frame_count == 0 {
        return Err(Error::InvalidFrameCount(header.frame_count));
    }
    if header.frame_count > limits.max_frames {
        return Err(Error::FrameLimitExceeded {
            value: header.frame_count,
            limit: limits.max_frames,
        });
    }
    if !header.image_format.is_known() {
        return Err(Error::InvalidImageFormat(header.image_format.0));
    }
    if header.low_res_image_format != ImageFormat::UNKNOWN
        && !header.low_res_image_format.is_known()
    {
        return Err(Error::InvalidLowResImageFormat(
            header.low_res_image_format.0,
        ));
    }
    if (header.low_res_width == 0) != (header.low_res_height == 0) {
        return Err(Error::InvalidDimensions {
            width: u16::from(header.low_res_width),
            height: u16::from(header.low_res_height),
            depth: 1,
        });
    }
    if header.low_res_width != 0 && header.low_res_image_format == ImageFormat::UNKNOWN {
        return Err(Error::InvalidLowResImageFormat(
            header.low_res_image_format.0,
        ));
    }
    if header.is_cube_map() && (header.width != header.height || header.depth != 1) {
        return Err(Error::InvalidCubeMap);
    }
    let maximum_mips = maximum_mip_count(header.width, header.height, header.depth);
    if header.mip_count == 0 || header.mip_count > maximum_mips {
        return Err(Error::InvalidMipCount {
            value: header.mip_count,
            maximum: maximum_mips,
        });
    }
    if header.resource_count > limits.max_resources {
        return Err(Error::ResourceLimitExceeded {
            value: header.resource_count,
            limit: limits.max_resources,
        });
    }
    Ok(())
}

fn parse_resources(
    bytes: &[u8],
    header: &Header,
    high_res_size: usize,
    low_res_size: usize,
) -> Result<Vec<Resource>> {
    let count = header.resource_count as usize;
    let table_start = 80usize;
    let table_size = count.checked_mul(8).ok_or(Error::SizeOverflow)?;
    let table_end = table_start
        .checked_add(table_size)
        .ok_or(Error::SizeOverflow)?;
    if table_end > header.header_size as usize || table_end > bytes.len() {
        return Err(Error::ResourceTableOutOfRange);
    }
    let mut reader = Reader::with_position(bytes, table_start)?;
    let mut resources = Vec::with_capacity(count);
    for _ in 0..count {
        let type_bytes = reader.take(4)?;
        let kind = [type_bytes[0], type_bytes[1], type_bytes[2]];
        let flags = type_bytes[3];
        let data = reader.read_u32_le()?;
        if resources
            .iter()
            .any(|resource: &Resource| resource.kind == kind)
        {
            return Err(Error::DuplicateResource(kind));
        }
        resources.push(Resource {
            kind,
            flags,
            data,
            payload: None,
        });
    }

    let mut chunk_offsets: Vec<(usize, usize)> = resources
        .iter()
        .enumerate()
        .filter(|(_, resource)| resource.flags & RSRCF_HAS_NO_DATA_CHUNK == 0)
        .map(|(index, resource)| (resource.data as usize, index))
        .collect();
    chunk_offsets.sort_unstable();
    for (position, &(offset, index)) in chunk_offsets.iter().enumerate() {
        let resource = &resources[index];
        if offset < header.header_size as usize || offset > bytes.len() {
            return Err(Error::InvalidResourceOffset {
                kind: resource.kind,
                offset: resource.data,
            });
        }
        let next = chunk_offsets
            .get(position + 1)
            .map_or(bytes.len(), |entry| entry.0);
        if next < offset || next > bytes.len() {
            return Err(Error::InvalidResourceOffset {
                kind: resource.kind,
                offset: resource.data,
            });
        }
        let payload = if is_legacy_resource(resource.kind) {
            offset..next
        } else {
            let length_bytes = bytes.get(offset..offset.saturating_add(4)).ok_or(
                Error::InvalidResourceOffset {
                    kind: resource.kind,
                    offset: resource.data,
                },
            )?;
            let size = u32::from_le_bytes(length_bytes.try_into().expect("four-byte slice"));
            let start = offset.checked_add(4).ok_or(Error::SizeOverflow)?;
            let end = start
                .checked_add(size as usize)
                .ok_or(Error::SizeOverflow)?;
            if end > next || end > bytes.len() {
                return Err(Error::InvalidResourceLength {
                    kind: resource.kind,
                    offset: resource.data,
                    size,
                });
            }
            start..end
        };
        resources[index].payload = Some(payload);
    }

    let image = resources
        .iter()
        .find(|resource| resource.kind == LEGACY_IMAGE)
        .and_then(|resource| resource.payload.as_ref())
        .ok_or(Error::MissingImageResource)?;
    require_payload_size(image, high_res_size, true)?;
    if low_res_size != 0 {
        let low_res = resources
            .iter()
            .find(|resource| resource.kind == LEGACY_LOW_RES_IMAGE)
            .and_then(|resource| resource.payload.as_ref())
            .ok_or(Error::LowResDataTruncated {
                expected: low_res_size,
                available: 0,
            })?;
        require_payload_size(low_res, low_res_size, false)?;
    }
    Ok(resources)
}

fn legacy_resources(
    bytes: &[u8],
    header: &Header,
    high_res_size: usize,
    low_res_size: usize,
) -> Result<Vec<Resource>> {
    let low_start = header.header_size as usize;
    let image_start = low_start
        .checked_add(low_res_size)
        .ok_or(Error::SizeOverflow)?;
    let image_end = image_start
        .checked_add(high_res_size)
        .ok_or(Error::SizeOverflow)?;
    if image_start > bytes.len() {
        return Err(Error::LowResDataTruncated {
            expected: low_res_size,
            available: bytes.len().saturating_sub(low_start),
        });
    }
    if image_end > bytes.len() {
        return Err(Error::ImageDataTruncated {
            expected: high_res_size,
            available: bytes.len().saturating_sub(image_start),
        });
    }
    let mut resources = Vec::with_capacity(2);
    if low_res_size != 0 {
        resources.push(Resource {
            kind: LEGACY_LOW_RES_IMAGE,
            flags: 0,
            data: u32::try_from(low_start).map_err(|_| Error::SizeOverflow)?,
            payload: Some(low_start..image_start),
        });
    }
    resources.push(Resource {
        kind: LEGACY_IMAGE,
        flags: 0,
        data: u32::try_from(image_start).map_err(|_| Error::SizeOverflow)?,
        payload: Some(image_start..image_end),
    });
    Ok(resources)
}

fn require_payload_size(range: &Range<usize>, expected: usize, image: bool) -> Result<()> {
    let available = range.end - range.start;
    if available < expected {
        if image {
            Err(Error::ImageDataTruncated {
                expected,
                available,
            })
        } else {
            Err(Error::LowResDataTruncated {
                expected,
                available,
            })
        }
    } else {
        Ok(())
    }
}

fn is_legacy_resource(kind: [u8; 3]) -> bool {
    kind == LEGACY_LOW_RES_IMAGE || kind == LEGACY_IMAGE
}

fn high_res_image_size(header: &Header) -> Result<usize> {
    let mut width = header.width as usize;
    let mut height = header.height as usize;
    let mut depth = header.depth as usize;
    let mut per_face = 0usize;
    for _ in 0..header.mip_count {
        let level = header
            .image_format
            .encoded_len(width, height, depth)
            .ok_or(Error::InvalidImageFormat(header.image_format.0))?;
        per_face = per_face.checked_add(level).ok_or(Error::SizeOverflow)?;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
        depth = (depth / 2).max(1);
    }
    per_face
        .checked_mul(header.frame_count as usize)
        .and_then(|size| size.checked_mul(header.face_count()))
        .ok_or(Error::SizeOverflow)
}

fn low_res_image_size(header: &Header) -> Result<usize> {
    if header.low_res_width == 0 || header.low_res_height == 0 {
        return Ok(0);
    }
    header
        .low_res_image_format
        .encoded_len(
            header.low_res_width as usize,
            header.low_res_height as usize,
            1,
        )
        .ok_or(Error::InvalidLowResImageFormat(
            header.low_res_image_format.0,
        ))
}

fn maximum_mip_count(width: u16, height: u16, depth: u16) -> u8 {
    let largest = width.max(height).max(depth);
    (u16::BITS - largest.leading_zeros()) as u8
}

fn block_encoded_len(
    width: usize,
    height: usize,
    depth: usize,
    block_bytes: usize,
) -> Option<usize> {
    let blocks_wide = width.max(4).div_ceil(4);
    let blocks_high = height.max(4).div_ceil(4);
    blocks_wide
        .checked_mul(blocks_high)?
        .checked_mul(depth)?
        .checked_mul(block_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(version_minor: u32, resource_count: u32, header_size: u32) -> Vec<u8> {
        let mut bytes = vec![0; header_size as usize];
        bytes[0..4].copy_from_slice(VTF_SIGNATURE);
        bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&version_minor.to_le_bytes());
        bytes[12..16].copy_from_slice(&header_size.to_le_bytes());
        bytes[16..18].copy_from_slice(&4u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&4u16.to_le_bytes());
        bytes[24..26].copy_from_slice(&1u16.to_le_bytes());
        bytes[48..52].copy_from_slice(&1f32.to_le_bytes());
        bytes[52..56].copy_from_slice(&0i32.to_le_bytes());
        bytes[56] = 3;
        bytes[57..61].copy_from_slice(&(-1i32).to_le_bytes());
        if version_minor >= 2 {
            bytes[63..65].copy_from_slice(&1u16.to_le_bytes());
        }
        if version_minor >= 3 {
            bytes[68..72].copy_from_slice(&resource_count.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn parses_legacy_texture_and_computes_mips() {
        let mut bytes = header(2, 0, 80);
        bytes.resize(80 + 84, 0xaa);
        let texture = Texture::parse(&bytes).unwrap();
        assert_eq!(texture.header().version_minor, 2);
        assert_eq!(texture.high_res_image_size(), 84);
        assert_eq!(texture.resource_payload(LEGACY_IMAGE).unwrap().len(), 84);
    }

    #[test]
    fn parses_resource_dictionary_and_inline_resource() {
        let mut bytes = header(5, 2, 96);
        bytes[80..84].copy_from_slice(&[0x30, 0, 0, 0]);
        bytes[84..88].copy_from_slice(&96u32.to_le_bytes());
        bytes[88..92].copy_from_slice(b"CRC\x02");
        bytes[92..96].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes.resize(96 + 84, 0xbb);
        let texture = Texture::parse(&bytes).unwrap();
        assert_eq!(texture.resources().len(), 2);
        assert_eq!(texture.resources()[1].payload, None);
        assert_eq!(texture.resource_payload(LEGACY_IMAGE).unwrap().len(), 84);
    }

    #[test]
    fn rejects_truncated_malformed_and_excessive_textures() {
        let mut bytes = header(2, 0, 80);
        bytes.resize(100, 0);
        assert!(matches!(
            Texture::parse(&bytes),
            Err(Error::ImageDataTruncated { .. })
        ));

        let mut bytes = header(5, 1, 88);
        bytes[80..84].copy_from_slice(&[0x30, 0, 0, 0]);
        bytes[84..88].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            Texture::parse(&bytes),
            Err(Error::InvalidResourceOffset { .. })
        ));

        let mut bytes = header(2, 0, 80);
        bytes[16..18].copy_from_slice(&32_768u16.to_le_bytes());
        assert!(matches!(
            Texture::parse(&bytes),
            Err(Error::DimensionLimitExceeded { .. })
        ));
    }

    /// A 4x4 RGBA texture with three mip levels, where every level of every
    /// frame and face is filled with its own byte so a mis-ordered read is
    /// visible rather than merely the wrong size.
    ///
    /// On disk the levels run smallest first: the 1x1 level of every frame
    /// and face, then every 2x2, then every 4x4.
    fn levelled_texture(frames: u16, fill: impl Fn(usize, usize, usize) -> u8) -> Vec<u8> {
        let mut bytes = header(2, 0, 80);
        bytes[24..26].copy_from_slice(&frames.to_le_bytes());

        // 1x1 is four bytes, 2x2 is sixteen, 4x4 is sixty-four.
        for (mip, size) in [(2usize, 4usize), (1, 16), (0, 64)] {
            for frame in 0..usize::from(frames) {
                bytes.extend(std::iter::repeat_n(fill(mip, frame, 0), size));
            }
        }
        bytes
    }

    #[test]
    fn reads_each_mip_level_out_of_the_order_the_file_stores_them_in() {
        // The engine writes the smallest level first and reverses the order
        // when it loads, so a reader that assumes largest-first returns the
        // 1x1 level as though it were the 4x4 one.
        let bytes = levelled_texture(1, |mip, _, _| 0xa0 + mip as u8);
        let texture = Texture::parse(&bytes).unwrap();

        assert_eq!(texture.level_extent(0).unwrap(), (4, 4, 1));
        assert_eq!(texture.level_extent(1).unwrap(), (2, 2, 1));
        assert_eq!(texture.level_extent(2).unwrap(), (1, 1, 1));
        assert_eq!(texture.level_len(0).unwrap(), 64);
        assert_eq!(texture.level_len(2).unwrap(), 4);

        for (mip, size) in [(0usize, 64usize), (1, 16), (2, 4)] {
            let level = texture.level(mip, 0, 0).unwrap();
            assert_eq!(level.len(), size, "mip {mip} is its own size");
            assert!(
                level.iter().all(|byte| *byte == 0xa0 + mip as u8),
                "mip {mip} holds its own bytes rather than another level's"
            );
        }

        // The chain a GPU upload takes runs the other way round, largest
        // first, because that is how a texture numbers its levels.
        let chain = texture.mip_chain(0, 0).unwrap();
        assert_eq!(
            chain.iter().map(|level| level.len()).collect::<Vec<_>>(),
            vec![64, 16, 4]
        );
        assert_eq!(chain[0][0], 0xa0);
        assert_eq!(chain[2][0], 0xa2);
    }

    #[test]
    fn keeps_each_frame_of_an_animated_texture_separate() {
        // Every frame of a level sits together before the next larger level,
        // so a reader that walks frames at the wrong stride returns one
        // frame's pixels for another.
        let bytes = levelled_texture(2, |mip, frame, _| ((mip as u8) << 4) | frame as u8);
        let texture = Texture::parse(&bytes).unwrap();

        assert_eq!(texture.header().frame_count, 2);
        for mip in 0..3 {
            for frame in 0..2 {
                let level = texture.level(mip, frame, 0).unwrap();
                assert!(
                    level
                        .iter()
                        .all(|byte| *byte == ((mip as u8) << 4) | frame as u8),
                    "mip {mip} of frame {frame} holds its own bytes"
                );
            }
        }
    }

    #[test]
    fn refuses_levels_frames_and_faces_a_texture_does_not_have() {
        let bytes = levelled_texture(1, |_, _, _| 0);
        let texture = Texture::parse(&bytes).unwrap();

        assert!(matches!(
            texture.level_extent(3),
            Err(Error::InvalidMipLevel { mip: 3, count: 3 })
        ));
        assert!(matches!(
            texture.level(0, 1, 0),
            Err(Error::InvalidFrame { frame: 1, count: 1 })
        ));
        assert!(matches!(
            texture.level(0, 0, 1),
            Err(Error::InvalidFace { face: 1, count: 1 })
        ));
    }

    #[test]
    fn names_the_formats_shipped_materials_use() {
        // The values are the engine's own enum, and getting one wrong would
        // upload a texture as the wrong format rather than fail.
        assert_eq!(ImageFormat::RGBA8888.0, 0);
        assert_eq!(ImageFormat::BGRA8888.0, 12);
        assert_eq!(ImageFormat::DXT1.0, 13);
        assert_eq!(ImageFormat::DXT5.0, 15);

        assert_eq!(ImageFormat::DXT1.block_extent(), 4);
        assert_eq!(ImageFormat::DXT5.block_extent(), 4);
        assert_eq!(ImageFormat::BGRA8888.block_extent(), 1);
        assert!(ImageFormat::DXT1.is_block_compressed());
        assert!(!ImageFormat::RGB888.is_block_compressed());
    }

    #[test]
    fn computes_block_compressed_sizes() {
        assert_eq!(ImageFormat(13).encoded_len(1, 1, 1), Some(8));
        assert_eq!(ImageFormat(15).encoded_len(5, 5, 1), Some(64));
    }

    #[test]
    fn versioned_cube_map_face_counts_match_legacy_loader() {
        for (minor, faces) in [(0, 6), (1, 7), (4, 7), (5, 6)] {
            let mut bytes = header(minor, 0, if minor <= 1 { 64 } else { 80 });
            bytes[20..24].copy_from_slice(&TEXTUREFLAGS_ENVMAP.to_le_bytes());
            bytes[16..18].copy_from_slice(&4u16.to_le_bytes());
            bytes[18..20].copy_from_slice(&4u16.to_le_bytes());
            let header = parse_header(&bytes).unwrap();
            assert_eq!(header.face_count(), faces, "VTF 7.{minor}");
        }
    }
}
