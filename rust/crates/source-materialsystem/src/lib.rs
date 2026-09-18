//! Turns the material names a map stores into textures on the GPU.
//!
//! A world surface names a material, a material names a texture, and the
//! texture is a VTF somewhere across the game's archives and the map's own.
//! This walks that chain once per material and keeps the result, so a map
//! whose four hundred batches share two hundred materials uploads each
//! texture once.
//!
//! What it deliberately does not do is decode anything. Half-Life 2's
//! textures are overwhelmingly block-compressed, Apple silicon samples those
//! formats natively, and so the bytes found on disk are the bytes handed to
//! Metal. Only the formats Metal has no equivalent for are converted, and
//! each of those conversions is named below.

use source_filesystem::SearchPaths;
use source_material::Material;
use source_render::{Device, Texture, TextureFormat};
use source_vtf::ImageFormat;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

/// A material resolved as far as it can be, with its texture on the GPU.
pub struct Binding {
    pub material: Material,
    /// The base texture, or `None` where the material names one the engine
    /// creates rather than one that is stored. See [`Unbound`].
    pub texture: Option<Texture>,
    pub width: u32,
    pub height: u32,
    /// The size of the texture data handed to the device, across every mip
    /// level.
    pub bytes: usize,
    /// Why there is no texture, where there is none.
    pub unbound: Option<Unbound>,
}

impl Binding {
    pub fn is_bound(&self) -> bool {
        self.texture.is_some()
    }
}

/// Why a material that parsed has no texture bound.
///
/// These are the cases a map legitimately contains, as distinct from the
/// errors in [`Error`]: a material can resolve perfectly and still name
/// nothing that can be uploaded from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unbound {
    /// The material sets no `$basetexture`, which the shaders that draw from
    /// a colour or an environment map do.
    NoBaseTexture,
    /// The texture is one the engine renders into rather than loads, named
    /// with the `_rt_` prefix, such as the camera feeds on the monitors in
    /// Half-Life 2.
    RenderTarget(String),
    /// A cubemap built from the map rather than stored as a file.
    MapCubemap(String),
    /// The texture is stored in a format this does not upload.
    UnsupportedFormat { path: String, format: ImageFormat },
    /// The base level is smaller than one compression block, which Metal
    /// cannot be given a region of.
    BelowOneBlock {
        path: String,
        width: u16,
        height: u16,
    },
}

impl fmt::Display for Unbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBaseTexture => write!(f, "material names no base texture"),
            Self::RenderTarget(path) => write!(f, "{path} is rendered by the engine"),
            Self::MapCubemap(path) => write!(f, "{path} is built from the map"),
            Self::UnsupportedFormat { path, format } => {
                write!(f, "{path} is stored in image format {}", format.0)
            }
            Self::BelowOneBlock {
                path,
                width,
                height,
            } => write!(f, "{path} is {width}x{height}, below one block"),
        }
    }
}

/// Material and texture loading, with everything it has loaded kept.
#[derive(Default)]
pub struct MaterialSystem {
    bindings: HashMap<String, Binding>,
    uploaded_bytes: usize,
}

impl MaterialSystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many materials have been loaded.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// The total size of the texture data uploaded so far, counting every
    /// mip level as it was handed to the device.
    pub fn uploaded_bytes(&self) -> usize {
        self.uploaded_bytes
    }

    pub fn get(&self, name: &str) -> Option<&Binding> {
        self.bindings.get(&source_material::material_path(name))
    }

    /// Loads a material by the name a map's surface gives it, uploading its
    /// base texture, and returns what was already loaded on a second ask.
    pub fn bind(&mut self, device: &Device, paths: &SearchPaths, name: &str) -> Result<&Binding> {
        let path = source_material::material_path(name);
        if !self.bindings.contains_key(&path) {
            let binding = self.load(device, paths, &path)?;
            self.uploaded_bytes += binding.bytes;
            self.bindings.insert(path.clone(), binding);
        }
        Ok(&self.bindings[&path])
    }

    fn load(&self, device: &Device, paths: &SearchPaths, path: &str) -> Result<Binding> {
        let text = read_text(paths, path)?;
        let material = source_material::resolve(&text, |include| read_text(paths, include).ok())
            .map_err(|error| Error::Material {
                path: path.to_owned(),
                error,
            })?;

        let Some(texture_path) = material.base_texture() else {
            return Ok(Binding {
                material,
                texture: None,
                width: 0,
                height: 0,
                bytes: 0,
                unbound: Some(Unbound::NoBaseTexture),
            });
        };

        if let Some(reason) = engine_owned(&texture_path) {
            return Ok(Binding {
                material,
                texture: None,
                width: 0,
                height: 0,
                bytes: 0,
                unbound: Some(reason),
            });
        }

        let bytes = paths
            .read(&texture_path, None)
            .map_err(|error| Error::Read {
                path: texture_path.clone(),
                message: error.to_string(),
            })?;
        let vtf = source_vtf::Texture::parse(&bytes).map_err(|error| Error::Vtf {
            path: texture_path.clone(),
            message: error.to_string(),
        })?;
        let header = vtf.header();

        let Some(format) = upload_format(header.image_format) else {
            return Ok(Binding {
                material,
                texture: None,
                width: header.width.into(),
                height: header.height.into(),
                bytes: 0,
                unbound: Some(Unbound::UnsupportedFormat {
                    path: texture_path,
                    format: header.image_format,
                }),
            });
        };

        // Metal is given whole blocks, so a base level narrower than one has
        // no region to replace. Such textures exist but never cover a
        // surface a player sees.
        let block = u16::try_from(format.block_extent()).unwrap_or(1);
        if header.width % block != 0 || header.height % block != 0 {
            return Ok(Binding {
                material,
                texture: None,
                width: header.width.into(),
                height: header.height.into(),
                bytes: 0,
                unbound: Some(Unbound::BelowOneBlock {
                    path: texture_path,
                    width: header.width,
                    height: header.height,
                }),
            });
        }

        // The first frame and face is the one a world surface samples; the
        // rest belong to animated materials and cube maps.
        let stored = vtf.mip_chain(0, 0).map_err(|error| Error::Vtf {
            path: texture_path.clone(),
            message: error.to_string(),
        })?;
        let levels = convert(header.image_format, &stored);
        let borrowed: Vec<&[u8]> = levels.iter().map(|level| level.as_ref()).collect();

        let texture = device
            .create_mipped_texture(header.width.into(), header.height.into(), format, &borrowed)
            .map_err(|error| Error::Upload {
                path: texture_path,
                message: error.to_string(),
            })?;

        Ok(Binding {
            material,
            texture: Some(texture),
            width: header.width.into(),
            height: header.height.into(),
            bytes: borrowed.iter().map(|level| level.len()).sum(),
            unbound: None,
        })
    }
}

fn read_text(paths: &SearchPaths, path: &str) -> Result<String> {
    let bytes = paths.read(path, None).map_err(|error| Error::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    // Materials are written by hand and a few carry stray bytes, which the
    // legacy reader passes through rather than refusing the file over.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Whether a named texture is one the engine produces rather than loads.
fn engine_owned(path: &str) -> Option<Unbound> {
    if path.contains("_rt_") {
        return Some(Unbound::RenderTarget(path.to_owned()));
    }
    if path.contains("env_cubemap") {
        return Some(Unbound::MapCubemap(path.to_owned()));
    }
    None
}

/// The GPU format a stored VTF format uploads as, or `None` for one that is
/// not uploaded.
///
/// The block-compressed formats map straight across because Apple silicon
/// samples them as they are stored. The rest are widened to eight bits per
/// channel with blue first, which is the one uncompressed format the
/// renderer takes.
fn upload_format(format: ImageFormat) -> Option<TextureFormat> {
    Some(match format {
        ImageFormat::DXT1 | ImageFormat::DXT1_ONEBITALPHA => TextureFormat::Bc1Rgba,
        ImageFormat::DXT3 => TextureFormat::Bc2Rgba,
        ImageFormat::DXT5 => TextureFormat::Bc3Rgba,
        ImageFormat::BGRA8888 | ImageFormat::BGRX8888 => TextureFormat::Bgra8Unorm,
        ImageFormat::RGBA8888
        | ImageFormat::ABGR8888
        | ImageFormat::ARGB8888
        | ImageFormat::RGB888
        | ImageFormat::BGR888
        | ImageFormat::I8
        | ImageFormat::IA88
        | ImageFormat::A8 => TextureFormat::Bgra8Unorm,
        _ => return None,
    })
}

/// Rewrites a texture's levels into the layout the GPU format expects.
///
/// The compressed and already-blue-first formats are handed over untouched,
/// which is the case for nearly every texture in the game. The others are
/// expanded a pixel at a time, which is why only the formats shipped content
/// actually uses are listed.
fn convert<'a>(format: ImageFormat, levels: &[&'a [u8]]) -> Vec<Cow<'a, [u8]>> {
    let widen: fn(&[u8]) -> Vec<u8> = match format {
        ImageFormat::RGBA8888 => |bytes| swizzle(bytes, 4, [2, 1, 0], Some(3)),
        ImageFormat::ABGR8888 => |bytes| swizzle(bytes, 4, [1, 2, 3], Some(0)),
        ImageFormat::ARGB8888 => |bytes| swizzle(bytes, 4, [3, 2, 1], Some(0)),
        ImageFormat::RGB888 => |bytes| swizzle(bytes, 3, [2, 1, 0], None),
        ImageFormat::BGR888 => |bytes| swizzle(bytes, 3, [0, 1, 2], None),
        // A single channel becomes grey, and an alpha-only texture becomes
        // white at that alpha, which is how the engine's own loader widens
        // them.
        ImageFormat::I8 => |bytes| swizzle(bytes, 1, [0, 0, 0], None),
        ImageFormat::IA88 => |bytes| swizzle(bytes, 2, [0, 0, 0], Some(1)),
        ImageFormat::A8 => |bytes| {
            let mut out = Vec::with_capacity(bytes.len() * 4);
            for alpha in bytes {
                out.extend_from_slice(&[0xff, 0xff, 0xff, *alpha]);
            }
            out
        },
        _ => return levels.iter().map(|level| Cow::Borrowed(*level)).collect(),
    };
    levels
        .iter()
        .map(|level| Cow::Owned(widen(level)))
        .collect()
}

/// Reorders a texture's channels into blue-green-red-alpha.
///
/// `take` names the source byte for each of blue, green and red within one
/// pixel, and `alpha` the source byte for alpha where the format stores one.
fn swizzle(bytes: &[u8], stride: usize, take: [usize; 3], alpha: Option<usize>) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() / stride * 4);
    for pixel in bytes.chunks_exact(stride) {
        for source in take {
            out.push(pixel[source]);
        }
        out.push(alpha.map_or(0xff, |source| pixel[source]));
    }
    out
}

#[derive(Debug)]
pub enum Error {
    Read {
        path: String,
        message: String,
    },
    Material {
        path: String,
        error: source_material::Error,
    },
    Vtf {
        path: String,
        message: String,
    },
    Upload {
        path: String,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } => write!(f, "reading {path}: {message}"),
            Self::Material { path, error } => write!(f, "material {path}: {error}"),
            Self::Vtf { path, message } => write!(f, "texture {path}: {message}"),
            Self::Upload { path, message } => write!(f, "uploading {path}: {message}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_the_compressed_formats_to_the_gpu_as_they_are_stored() {
        // Nearly every texture in the game is one of these, and decoding
        // them on the way through would cost memory and fidelity for
        // nothing: the GPU samples the stored form.
        for (stored, expected) in [
            (ImageFormat::DXT1, TextureFormat::Bc1Rgba),
            (ImageFormat::DXT1_ONEBITALPHA, TextureFormat::Bc1Rgba),
            (ImageFormat::DXT3, TextureFormat::Bc2Rgba),
            (ImageFormat::DXT5, TextureFormat::Bc3Rgba),
            (ImageFormat::BGRA8888, TextureFormat::Bgra8Unorm),
        ] {
            assert_eq!(upload_format(stored), Some(expected));
            let level: &[u8] = &[1, 2, 3, 4];
            let converted = convert(stored, &[level]);
            assert!(
                matches!(converted[0], Cow::Borrowed(_)),
                "image format {} is uploaded without a copy",
                stored.0
            );
            assert_eq!(converted[0].as_ref(), level);
        }
    }

    #[test]
    fn widens_the_formats_metal_has_no_equivalent_of() {
        // One opaque pixel, written the way each format stores it, all of
        // which have to arrive as the same blue-green-red-alpha bytes.
        let blue = 0x10;
        let green = 0x20;
        let red = 0x30;
        let expected = vec![blue, green, red, 0xff];

        for (stored, pixel) in [
            (ImageFormat::RGBA8888, vec![red, green, blue, 0xff]),
            (ImageFormat::ABGR8888, vec![0xff, blue, green, red]),
            (ImageFormat::ARGB8888, vec![0xff, red, green, blue]),
            (ImageFormat::RGB888, vec![red, green, blue]),
            (ImageFormat::BGR888, vec![blue, green, red]),
        ] {
            assert_eq!(upload_format(stored), Some(TextureFormat::Bgra8Unorm));
            let converted = convert(stored, &[pixel.as_slice()]);
            assert_eq!(
                converted[0].as_ref(),
                expected.as_slice(),
                "image format {} widens to blue-first",
                stored.0
            );
        }
    }

    #[test]
    fn widens_the_single_channel_formats_the_way_the_engine_does() {
        // A luminance texture is grey at full alpha, and an alpha-only one
        // is white at the stored alpha.
        assert_eq!(
            convert(ImageFormat::I8, &[[0x40, 0x80].as_slice()])[0].as_ref(),
            &[0x40, 0x40, 0x40, 0xff, 0x80, 0x80, 0x80, 0xff]
        );
        assert_eq!(
            convert(ImageFormat::IA88, &[[0x40, 0x11].as_slice()])[0].as_ref(),
            &[0x40, 0x40, 0x40, 0x11]
        );
        assert_eq!(
            convert(ImageFormat::A8, &[[0x11, 0x22].as_slice()])[0].as_ref(),
            &[0xff, 0xff, 0xff, 0x11, 0xff, 0xff, 0xff, 0x22]
        );
    }

    #[test]
    fn widens_every_level_of_a_chain() {
        let base: &[u8] = &[1, 2, 3, 4, 5, 6];
        let small: &[u8] = &[7, 8, 9];
        let converted = convert(ImageFormat::RGB888, &[base, small]);

        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].len(), 8, "two pixels widen to four bytes each");
        assert_eq!(converted[1].len(), 4);
    }

    #[test]
    fn declines_a_format_it_would_have_to_guess_at() {
        // Format 24 is a four-channel signed format used for normal maps,
        // and 41 is past the enum entirely. Uploading either as though it
        // were colour would texture a surface with noise.
        assert_eq!(upload_format(ImageFormat(24)), None);
        assert_eq!(upload_format(ImageFormat(41)), None);
        assert_eq!(upload_format(ImageFormat::UNKNOWN), None);
    }

    #[test]
    fn names_the_textures_the_engine_produces_rather_than_loads() {
        assert_eq!(
            engine_owned("materials/_rt_camera.vtf"),
            Some(Unbound::RenderTarget("materials/_rt_camera.vtf".to_owned()))
        );
        assert_eq!(
            engine_owned("materials/engine/env_cubemap.vtf"),
            Some(Unbound::MapCubemap(
                "materials/engine/env_cubemap.vtf".to_owned()
            ))
        );
        assert_eq!(engine_owned("materials/brick/wall.vtf"), None);
    }

    #[test]
    fn describes_why_a_material_went_unbound() {
        for reason in [
            Unbound::NoBaseTexture,
            Unbound::RenderTarget("a".to_owned()),
            Unbound::MapCubemap("a".to_owned()),
            Unbound::UnsupportedFormat {
                path: "a".to_owned(),
                format: ImageFormat(24),
            },
            Unbound::BelowOneBlock {
                path: "a".to_owned(),
                width: 2,
                height: 2,
            },
        ] {
            assert!(!reason.to_string().is_empty());
        }
        for error in [
            Error::Read {
                path: "a".to_owned(),
                message: "b".to_owned(),
            },
            Error::Material {
                path: "a".to_owned(),
                error: source_material::Error::Empty,
            },
            Error::Vtf {
                path: "a".to_owned(),
                message: "b".to_owned(),
            },
            Error::Upload {
                path: "a".to_owned(),
                message: "b".to_owned(),
            },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}
