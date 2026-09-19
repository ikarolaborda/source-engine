//! Metal's enumerations and by-value structures, as the framework headers
//! number and lay them out, and the conversions from Direct3D's.

use crate::objc::NSUInteger;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MTLViewport {
    pub origin_x: f64,
    pub origin_y: f64,
    pub width: f64,
    pub height: f64,
    pub znear: f64,
    pub zfar: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MTLScissorRect {
    pub x: NSUInteger,
    pub y: NSUInteger,
    pub width: NSUInteger,
    pub height: NSUInteger,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MTLOrigin {
    pub x: NSUInteger,
    pub y: NSUInteger,
    pub z: NSUInteger,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MTLSize {
    pub width: NSUInteger,
    pub height: NSUInteger,
    pub depth: NSUInteger,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MTLRegion {
    pub origin: MTLOrigin,
    pub size: MTLSize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MTLClearColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSEdgeInsets {
    pub top: f64,
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
}

pub const TEXTURE_TYPE_2D: NSUInteger = 2;
pub const TEXTURE_TYPE_CUBE: NSUInteger = 5;
pub const TEXTURE_TYPE_3D: NSUInteger = 7;

pub const TEXTURE_USAGE_SHADER_READ: NSUInteger = 1;
pub const TEXTURE_USAGE_RENDER_TARGET: NSUInteger = 4;
pub const TEXTURE_USAGE_PIXEL_FORMAT_VIEW: NSUInteger = 0x10;

pub const STORAGE_MODE_SHARED: NSUInteger = 0;
pub const STORAGE_MODE_PRIVATE: NSUInteger = 2;
/// `MTLResourceStorageModeShared`, the storage mode shifted into an options
/// mask, with the default cache mode.
pub const RESOURCE_STORAGE_SHARED: NSUInteger = STORAGE_MODE_SHARED << 4;

pub const LOAD_ACTION_DONT_CARE: NSUInteger = 0;
pub const LOAD_ACTION_LOAD: NSUInteger = 1;
pub const LOAD_ACTION_CLEAR: NSUInteger = 2;
pub const STORE_ACTION_STORE: NSUInteger = 1;

pub const PRIMITIVE_POINT: NSUInteger = 0;
pub const PRIMITIVE_LINE: NSUInteger = 1;
pub const PRIMITIVE_LINE_STRIP: NSUInteger = 2;
pub const PRIMITIVE_TRIANGLE: NSUInteger = 3;
pub const PRIMITIVE_TRIANGLE_STRIP: NSUInteger = 4;

pub const INDEX_TYPE_UINT16: NSUInteger = 0;
pub const INDEX_TYPE_UINT32: NSUInteger = 1;

pub const CULL_NONE: NSUInteger = 0;
pub const CULL_FRONT: NSUInteger = 1;
pub const CULL_BACK: NSUInteger = 2;
pub const WINDING_CLOCKWISE: NSUInteger = 0;
pub const FILL_MODE_FILL: NSUInteger = 0;
pub const FILL_MODE_LINES: NSUInteger = 1;

pub const COMPARE_ALWAYS: NSUInteger = 7;
pub const COMPARE_LESS_EQUAL: NSUInteger = 3;
pub const STENCIL_KEEP: NSUInteger = 0;
pub const STENCIL_REPLACE: NSUInteger = 2;

pub const BLEND_ZERO: NSUInteger = 0;
pub const BLEND_ONE: NSUInteger = 1;

pub const SAMPLER_FILTER_NEAREST: NSUInteger = 0;
pub const SAMPLER_FILTER_LINEAR: NSUInteger = 1;
pub const SAMPLER_MIP_NONE: NSUInteger = 0;
pub const SAMPLER_MIP_NEAREST: NSUInteger = 1;
pub const SAMPLER_MIP_LINEAR: NSUInteger = 2;

pub const VERTEX_STEP_CONSTANT: NSUInteger = 0;
pub const VERTEX_STEP_PER_VERTEX: NSUInteger = 1;

pub const VERTEX_FORMAT_INVALID: NSUInteger = 0;
pub const VERTEX_FORMAT_FLOAT4: NSUInteger = 31;

/// `MTLDataTypeBool`, for function constants.
pub const DATA_TYPE_BOOL: NSUInteger = 53;

/// `D3DCMPFUNC` to `MTLCompareFunction`. Direct3D numbers from one and Metal
/// from zero, in the same order; anything else reads as "always".
pub fn compare(d3d: u32) -> NSUInteger {
    if (1..=8).contains(&d3d) {
        NSUInteger::from(d3d - 1)
    } else {
        COMPARE_ALWAYS
    }
}

/// `D3DSTENCILOP` to `MTLStencilOperation`, which are in the same order.
pub fn stencil_op(d3d: u32) -> NSUInteger {
    if (1..=8).contains(&d3d) {
        NSUInteger::from(d3d - 1)
    } else {
        STENCIL_KEEP
    }
}

/// `D3DBLENDOP` to `MTLBlendOperation`, which are in the same order.
pub fn blend_op(d3d: u32) -> NSUInteger {
    if (1..=5).contains(&d3d) {
        NSUInteger::from(d3d - 1)
    } else {
        0
    }
}

/// `D3DBLEND` to `MTLBlendFactor`.
pub fn blend_factor(d3d: u32) -> NSUInteger {
    match d3d {
        1 => 0,   // ZERO
        2 => 1,   // ONE
        3 => 2,   // SRCCOLOR
        4 => 3,   // INVSRCCOLOR
        5 => 4,   // SRCALPHA
        6 => 5,   // INVSRCALPHA
        7 => 8,   // DESTALPHA
        8 => 9,   // INVDESTALPHA
        9 => 6,   // DESTCOLOR
        10 => 7,  // INVDESTCOLOR
        11 => 10, // SRCALPHASAT
        14 => 11, // BLENDFACTOR
        15 => 12, // INVBLENDFACTOR
        _ => 1,
    }
}

/// `D3DCOLORWRITEENABLE_*` to `MTLColorWriteMask`, whose bits run the other
/// way round.
pub fn color_write_mask(d3d: u32) -> NSUInteger {
    let mut mask = 0;
    if d3d & 1 != 0 {
        mask |= 8; // red
    }
    if d3d & 2 != 0 {
        mask |= 4; // green
    }
    if d3d & 4 != 0 {
        mask |= 2; // blue
    }
    if d3d & 8 != 0 {
        mask |= 1; // alpha
    }
    mask
}

/// `D3DTEXTUREADDRESS` to `MTLSamplerAddressMode`. The engine's POSIX
/// headers declare only three modes and number them from zero, unlike
/// Direct3D's own, which start at one and include the mirrored modes.
pub fn address_mode(d3d: u32) -> NSUInteger {
    match d3d {
        1 => 0, // CLAMP -> clamp to edge
        2 => 5, // BORDER -> clamp to border colour
        _ => 2, // WRAP -> repeat
    }
}

/// `D3DDECLTYPE` to `MTLVertexFormat`, or `VERTEX_FORMAT_INVALID` for the
/// packed ten-bit types nothing in the engine declares.
pub fn vertex_format(d3d: u8) -> NSUInteger {
    match d3d {
        0 => 28, // FLOAT1
        1 => 29, // FLOAT2
        2 => 30, // FLOAT3
        3 => 31, // FLOAT4
        // D3DCOLOR is blue-first in Direct3D, but the mesh builder is compiled
        // with OPENGL_SWAP_COLORS and writes red first, as ToGL needed.
        4 => 9,
        5 => 3,   // UBYTE4
        6 => 16,  // SHORT2
        7 => 18,  // SHORT4
        8 => 9,   // UBYTE4N
        9 => 22,  // SHORT2N
        10 => 24, // SHORT4N
        11 => 19, // USHORT2N
        12 => 21, // USHORT4N
        15 => 25, // FLOAT16_2
        16 => 27, // FLOAT16_4
        _ => VERTEX_FORMAT_INVALID,
    }
}

/// `D3DPRIMITIVETYPE` to `MTLPrimitiveType` and the number of vertices or
/// indices `count` primitives take. Fans have no Metal equivalent and come
/// back as `None`.
pub fn primitive(d3d: u32, count: u32) -> Option<(NSUInteger, u64)> {
    use crate::state::prim;
    let count = u64::from(count);
    Some(match d3d {
        prim::POINTLIST => (PRIMITIVE_POINT, count),
        prim::LINELIST => (PRIMITIVE_LINE, count * 2),
        prim::LINESTRIP => (PRIMITIVE_LINE_STRIP, count + 1),
        prim::TRIANGLELIST => (PRIMITIVE_TRIANGLE, count * 3),
        prim::TRIANGLESTRIP => (PRIMITIVE_TRIANGLE_STRIP, count + 2),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_functions_shift_by_one() {
        assert_eq!(compare(1), 0); // NEVER
        assert_eq!(compare(4), 3); // LESSEQUAL
        assert_eq!(compare(8), 7); // ALWAYS
        assert_eq!(compare(0), COMPARE_ALWAYS);
    }

    #[test]
    fn destination_factors_are_not_in_direct3d_order() {
        // Direct3D lists destination alpha before destination colour and
        // Metal the other way round, which is the one place a shift is wrong.
        assert_eq!(blend_factor(7), 8);
        assert_eq!(blend_factor(9), 6);
        assert_eq!(blend_factor(5), 4);
        assert_eq!(blend_factor(6), 5);
    }

    #[test]
    fn write_mask_bits_reverse() {
        assert_eq!(color_write_mask(0xF), 0xF);
        assert_eq!(color_write_mask(1), 8);
        assert_eq!(color_write_mask(8), 1);
        assert_eq!(color_write_mask(0), 0);
    }

    #[test]
    fn primitive_counts() {
        assert_eq!(primitive(4, 10), Some((PRIMITIVE_TRIANGLE, 30)));
        assert_eq!(primitive(5, 10), Some((PRIMITIVE_TRIANGLE_STRIP, 12)));
        assert_eq!(primitive(2, 3), Some((PRIMITIVE_LINE, 6)));
        assert_eq!(primitive(6, 3), None);
    }
}
