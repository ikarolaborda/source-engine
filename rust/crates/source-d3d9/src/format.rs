//! Direct3D 9 surface formats and what each becomes in Metal.

/// `D3DFORMAT` values the device knows.
///
/// These are not Direct3D's own numbers. On POSIX the engine declares the
/// enumeration itself, in `public/bitmap/imageformat.h`, and numbers it from
/// zero in the order it lists the formats; that is what crosses the ABI.
pub mod d3dfmt {
    pub const D16: u32 = 1;
    pub const D24S8: u32 = 2;
    pub const A8R8G8B8: u32 = 3;
    pub const X8R8G8B8: u32 = 5;
    pub const L8: u32 = 9;
    pub const A8L8: u32 = 10;
    pub const DXT1: u32 = 12;
    pub const DXT3: u32 = 13;
    pub const DXT5: u32 = 14;
    pub const V8U8: u32 = 15;
    pub const Q8W8V8U8: u32 = 16;
    pub const A16B16G16R16F: u32 = 18;
    pub const A16B16G16R16: u32 = 19;
    pub const R32F: u32 = 20;
    pub const A32B32G32R32F: u32 = 21;
    pub const R8G8B8: u32 = 22;
    pub const A8: u32 = 24;
    pub const D24X8: u32 = 27;
    /// The vendor formats are four-character codes, as in Direct3D.
    pub const NV_INTZ: u32 = u32::from_le_bytes(*b"INTZ");
}

/// `MTLPixelFormat` values the device creates.
pub mod mtlfmt {
    pub const INVALID: u64 = 0;
    pub const A8_UNORM: u64 = 1;
    pub const RG8_SNORM: u64 = 32;
    pub const R32_FLOAT: u64 = 55;
    pub const RGBA8_SNORM: u64 = 72;
    pub const BGRA8_UNORM: u64 = 80;
    pub const BGRA8_UNORM_SRGB: u64 = 81;
    pub const RGBA16_UNORM: u64 = 110;
    pub const RGBA16_FLOAT: u64 = 115;
    pub const RGBA32_FLOAT: u64 = 125;
    pub const BC1_RGBA: u64 = 130;
    pub const BC1_RGBA_SRGB: u64 = 131;
    pub const BC2_RGBA: u64 = 132;
    pub const BC2_RGBA_SRGB: u64 = 133;
    pub const BC3_RGBA: u64 = 134;
    pub const BC3_RGBA_SRGB: u64 = 135;
    pub const DEPTH16_UNORM: u64 = 250;
    pub const DEPTH32_FLOAT_STENCIL8: u64 = 260;
}

/// A format Metal has no equivalent of, and which is widened to BGRA as it is
/// uploaded. The engine creates a handful of small textures in these without
/// asking whether it may.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expand {
    None,
    /// Three bytes a pixel, blue first.
    Bgr24,
    /// One byte of luminance, read as grey.
    L8,
    /// Luminance then alpha.
    A8L8,
}

impl Expand {
    /// Widens `source` to four bytes a pixel.
    pub fn apply(self, source: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(source.len() * 4);
        match self {
            Expand::None => out.extend_from_slice(source),
            Expand::Bgr24 => {
                for p in source.chunks_exact(3) {
                    out.extend_from_slice(&[p[0], p[1], p[2], 0xFF]);
                }
            }
            Expand::L8 => {
                for l in source {
                    out.extend_from_slice(&[*l, *l, *l, 0xFF]);
                }
            }
            Expand::A8L8 => {
                for p in source.chunks_exact(2) {
                    out.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
                }
            }
        }
        out
    }
}

/// How one Direct3D format is stored and viewed in Metal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatInfo {
    /// The format the texture is created in.
    pub metal: u64,
    /// The view sampled or rendered through when sRGB conversion is asked
    /// for, or `INVALID` when the format has no sRGB form.
    pub metal_srgb: u64,
    /// Edge length of a block: 4 for the block-compressed formats, else 1.
    pub block: u32,
    /// Bytes in one block, which for an uncompressed format is one pixel.
    pub block_bytes: u32,
    pub depth: bool,
    pub stencil: bool,
    /// The format has an unused channel where alpha would be, which Direct3D
    /// reads as one. Metal has no such format, so uploads set it.
    pub opaque_alpha: bool,
    /// `block_bytes` describes the caller's pixels. When this is set the
    /// texture itself is BGRA and an upload is widened on the way in.
    pub expand: Expand,
}

impl FormatInfo {
    const fn color(metal: u64, metal_srgb: u64, bytes: u32) -> Self {
        Self {
            metal,
            metal_srgb,
            block: 1,
            block_bytes: bytes,
            depth: false,
            stencil: false,
            opaque_alpha: false,
            expand: Expand::None,
        }
    }

    const fn blocks(metal: u64, metal_srgb: u64, bytes: u32) -> Self {
        Self {
            metal,
            metal_srgb,
            block: 4,
            block_bytes: bytes,
            depth: false,
            stencil: false,
            opaque_alpha: false,
            expand: Expand::None,
        }
    }

    /// Bytes in one row of a level `width` pixels wide.
    pub fn row_bytes(&self, width: u32) -> usize {
        (width.div_ceil(self.block) * self.block_bytes) as usize
    }

    /// Rows of storage in a level `height` pixels tall, which for a block
    /// format is rows of blocks.
    pub fn rows(&self, height: u32) -> usize {
        height.div_ceil(self.block) as usize
    }
}

/// Looks a `D3DFORMAT` up, or `None` for one the device does not create. The
/// set matches what `IDirect3D9::CheckDeviceFormat` in `tometal/` admits to.
pub fn lookup(format: u32) -> Option<FormatInfo> {
    use mtlfmt::*;
    Some(match format {
        d3dfmt::A8R8G8B8 => FormatInfo::color(BGRA8_UNORM, BGRA8_UNORM_SRGB, 4),
        d3dfmt::X8R8G8B8 => FormatInfo {
            opaque_alpha: true,
            ..FormatInfo::color(BGRA8_UNORM, BGRA8_UNORM_SRGB, 4)
        },
        d3dfmt::R8G8B8 => FormatInfo {
            expand: Expand::Bgr24,
            ..FormatInfo::color(BGRA8_UNORM, BGRA8_UNORM_SRGB, 3)
        },
        d3dfmt::L8 => FormatInfo {
            expand: Expand::L8,
            ..FormatInfo::color(BGRA8_UNORM, BGRA8_UNORM_SRGB, 1)
        },
        d3dfmt::A8L8 => FormatInfo {
            expand: Expand::A8L8,
            ..FormatInfo::color(BGRA8_UNORM, BGRA8_UNORM_SRGB, 2)
        },
        d3dfmt::A8 => FormatInfo::color(A8_UNORM, INVALID, 1),
        d3dfmt::V8U8 => FormatInfo::color(RG8_SNORM, INVALID, 2),
        d3dfmt::Q8W8V8U8 => FormatInfo::color(RGBA8_SNORM, INVALID, 4),
        d3dfmt::A16B16G16R16 => FormatInfo::color(RGBA16_UNORM, INVALID, 8),
        d3dfmt::A16B16G16R16F => FormatInfo::color(RGBA16_FLOAT, INVALID, 8),
        d3dfmt::A32B32G32R32F => FormatInfo::color(RGBA32_FLOAT, INVALID, 16),
        d3dfmt::R32F => FormatInfo::color(R32_FLOAT, INVALID, 4),
        d3dfmt::DXT1 => FormatInfo::blocks(BC1_RGBA, BC1_RGBA_SRGB, 8),
        d3dfmt::DXT3 => FormatInfo::blocks(BC2_RGBA, BC2_RGBA_SRGB, 16),
        d3dfmt::DXT5 => FormatInfo::blocks(BC3_RGBA, BC3_RGBA_SRGB, 16),
        d3dfmt::D16 => FormatInfo {
            depth: true,
            ..FormatInfo::color(DEPTH16_UNORM, INVALID, 2)
        },
        // Apple's GPUs have no 24-bit depth, so both of Direct3D's become a
        // float depth with a stencil plane, which is at least as precise.
        d3dfmt::D24S8 | d3dfmt::D24X8 => FormatInfo {
            depth: true,
            stencil: true,
            ..FormatInfo::color(DEPTH32_FLOAT_STENCIL8, INVALID, 4)
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_formats_round_up_to_whole_blocks() {
        let dxt1 = lookup(d3dfmt::DXT1).unwrap();
        assert_eq!(dxt1.row_bytes(1), 8);
        assert_eq!(dxt1.row_bytes(4), 8);
        assert_eq!(dxt1.row_bytes(5), 16);
        assert_eq!(dxt1.rows(6), 2);
        let dxt5 = lookup(d3dfmt::DXT5).unwrap();
        assert_eq!(dxt5.row_bytes(256), 64 * 16);
    }

    #[test]
    fn uncompressed_formats_are_a_pixel_per_block() {
        let bgra = lookup(d3dfmt::A8R8G8B8).unwrap();
        assert_eq!(bgra.row_bytes(17), 68);
        assert_eq!(bgra.rows(9), 9);
        assert!(!bgra.opaque_alpha);
        assert!(lookup(d3dfmt::X8R8G8B8).unwrap().opaque_alpha);
    }

    #[test]
    fn narrow_formats_widen_to_bgra() {
        assert_eq!(
            Expand::Bgr24.apply(&[1, 2, 3, 4, 5, 6]),
            [1, 2, 3, 255, 4, 5, 6, 255]
        );
        assert_eq!(Expand::L8.apply(&[7, 9]), [7, 7, 7, 255, 9, 9, 9, 255]);
        assert_eq!(Expand::A8L8.apply(&[7, 9]), [7, 7, 7, 9]);
        let rgb = lookup(d3dfmt::R8G8B8).unwrap();
        assert_eq!(rgb.row_bytes(5), 15);
        assert_eq!(rgb.metal, mtlfmt::BGRA8_UNORM);
    }

    #[test]
    fn unknown_formats_are_refused() {
        assert!(lookup(0).is_none());
        assert!(lookup(d3dfmt::NV_INTZ).is_none());
    }
}
