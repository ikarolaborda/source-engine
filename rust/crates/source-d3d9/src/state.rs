//! The Direct3D state block a draw is handed, laid out exactly as
//! `public/rust/source_d3d9.h` declares it, and the Direct3D constants needed
//! to read it.

pub const RENDER_STATES: usize = 256;
pub const SAMPLERS: usize = 16;
pub const SAMPLER_STATES: usize = 16;
pub const STREAMS: usize = 8;
pub const RENDER_TARGETS: usize = 4;
pub const VS_FLOAT_CONSTANTS: usize = 256;
pub const PS_FLOAT_CONSTANTS: usize = 224;
pub const INT_CONSTANTS: usize = 16;

pub type Handle = u64;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SurfaceRef {
    pub texture: Handle,
    pub face: u32,
    pub level: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Stream {
    pub buffer: Handle,
    pub offset: u32,
    pub stride: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub min_z: f32,
    pub max_z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
pub struct State {
    pub render_states: [u32; RENDER_STATES],
    pub sampler_states: [[u32; SAMPLER_STATES]; SAMPLERS],
    pub textures: [Handle; SAMPLERS],
    pub render_targets: [SurfaceRef; RENDER_TARGETS],
    pub depth_stencil: SurfaceRef,
    pub vertex_shader: Handle,
    pub pixel_shader: Handle,
    pub vertex_declaration: Handle,
    pub streams: [Stream; STREAMS],
    pub index_buffer: Handle,
    pub viewport: Viewport,
    pub scissor: Rect,
    pub clip_plane0: [f32; 4],
    pub vs_bools: u32,
    pub ps_bools: u32,
    pub vs_ints: [[i32; 4]; INT_CONSTANTS],
    pub ps_ints: [[i32; 4]; INT_CONSTANTS],
    pub vs_floats: [[f32; 4]; VS_FLOAT_CONSTANTS],
    pub ps_floats: [[f32; 4]; PS_FLOAT_CONSTANTS],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct VertexElement {
    pub stream: u16,
    pub offset: u16,
    pub kind: u8,
    pub method: u8,
    pub usage: u8,
    pub usage_index: u8,
}

#[repr(C)]
pub struct DisplayInfo {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub refresh_hz: u32,
    pub backing_scale: f32,
    pub recommended_memory: u64,
    pub name: [u8; 128],
}

pub const TEXTURE_2D: u32 = 0;
pub const TEXTURE_CUBE: u32 = 1;
pub const TEXTURE_VOLUME: u32 = 2;

pub const SHADER_VERTEX: i32 = 0;
pub const SHADER_PIXEL: i32 = 1;

/// `D3DRENDERSTATETYPE`.
pub mod rs {
    pub const ZENABLE: usize = 7;
    pub const FILLMODE: usize = 8;
    pub const ZWRITEENABLE: usize = 14;
    pub const ALPHATESTENABLE: usize = 15;
    pub const SRCBLEND: usize = 19;
    pub const DESTBLEND: usize = 20;
    pub const CULLMODE: usize = 22;
    pub const ZFUNC: usize = 23;
    pub const ALPHAREF: usize = 24;
    pub const ALPHAFUNC: usize = 25;
    pub const ALPHABLENDENABLE: usize = 27;
    pub const STENCILENABLE: usize = 52;
    pub const STENCILFAIL: usize = 53;
    pub const STENCILZFAIL: usize = 54;
    pub const STENCILPASS: usize = 55;
    pub const STENCILFUNC: usize = 56;
    pub const STENCILREF: usize = 57;
    pub const STENCILMASK: usize = 58;
    pub const STENCILWRITEMASK: usize = 59;
    pub const CLIPPLANEENABLE: usize = 152;
    pub const COLORWRITEENABLE: usize = 168;
    pub const BLENDOP: usize = 171;
    pub const SCISSORTESTENABLE: usize = 174;
    pub const SLOPESCALEDEPTHBIAS: usize = 175;
    pub const TWOSIDEDSTENCILMODE: usize = 185;
    pub const CCW_STENCILFAIL: usize = 186;
    pub const CCW_STENCILZFAIL: usize = 187;
    pub const CCW_STENCILPASS: usize = 188;
    pub const CCW_STENCILFUNC: usize = 189;
    pub const SRGBWRITEENABLE: usize = 194;
    pub const DEPTHBIAS: usize = 195;
    pub const SEPARATEALPHABLENDENABLE: usize = 206;
    pub const SRCBLENDALPHA: usize = 207;
    pub const DESTBLENDALPHA: usize = 208;
    pub const BLENDOPALPHA: usize = 209;
}

/// `D3DSAMPLERSTATETYPE`.
pub mod samp {
    pub const ADDRESSU: usize = 1;
    pub const ADDRESSV: usize = 2;
    pub const ADDRESSW: usize = 3;
    pub const BORDERCOLOR: usize = 4;
    pub const MAGFILTER: usize = 5;
    pub const MINFILTER: usize = 6;
    pub const MIPFILTER: usize = 7;
    pub const MIPMAPLODBIAS: usize = 8;
    pub const MAXMIPLEVEL: usize = 9;
    pub const MAXANISOTROPY: usize = 10;
    pub const SRGBTEXTURE: usize = 11;
    /// Not Direct3D's: ToGL added it so the engine could say which samplers
    /// are shadow-map lookups, which bytecode alone does not.
    pub const SHADOWFILTER: usize = 12;
}

pub const D3DUSAGE_RENDERTARGET: u32 = 0x0000_0001;
pub const D3DUSAGE_DEPTHSTENCIL: u32 = 0x0000_0002;
pub const D3DUSAGE_DYNAMIC: u32 = 0x0000_0200;
pub const D3DUSAGE_AUTOGENMIPMAP: u32 = 0x0000_0400;

pub const D3DLOCK_NOOVERWRITE: u32 = 0x0000_1000;
pub const D3DLOCK_DISCARD: u32 = 0x0000_2000;

pub const D3DCLEAR_TARGET: u32 = 1;
pub const D3DCLEAR_ZBUFFER: u32 = 2;
pub const D3DCLEAR_STENCIL: u32 = 4;

pub const D3DISSUE_END: u32 = 1;
pub const D3DISSUE_BEGIN: u32 = 2;

/// `D3DPRIMITIVETYPE`.
pub mod prim {
    pub const POINTLIST: u32 = 1;
    pub const LINELIST: u32 = 2;
    pub const LINESTRIP: u32 = 3;
    pub const TRIANGLELIST: u32 = 4;
    pub const TRIANGLESTRIP: u32 = 5;
    pub const TRIANGLEFAN: u32 = 6;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// The block is read through a pointer C++ filled in, so its layout is an
    /// ABI. These are the offsets `source_d3d9.h` gives on a 64-bit target.
    #[test]
    fn state_block_matches_the_header() {
        assert_eq!(size_of::<SurfaceRef>(), 16);
        assert_eq!(size_of::<Stream>(), 16);
        assert_eq!(size_of::<Viewport>(), 24);
        assert_eq!(size_of::<VertexElement>(), 8);

        let mut at = 0;
        assert_eq!(offset_of!(State, render_states), at);
        at += 4 * RENDER_STATES;
        assert_eq!(offset_of!(State, sampler_states), at);
        at += 4 * SAMPLERS * SAMPLER_STATES;
        assert_eq!(offset_of!(State, textures), at);
        at += 8 * SAMPLERS;
        assert_eq!(offset_of!(State, render_targets), at);
        at += 16 * RENDER_TARGETS;
        assert_eq!(offset_of!(State, depth_stencil), at);
        at += 16;
        assert_eq!(offset_of!(State, vertex_shader), at);
        at += 24;
        assert_eq!(offset_of!(State, streams), at);
        at += 16 * STREAMS;
        assert_eq!(offset_of!(State, index_buffer), at);
        at += 8;
        assert_eq!(offset_of!(State, viewport), at);
        at += 24;
        assert_eq!(offset_of!(State, scissor), at);
        at += 16;
        assert_eq!(offset_of!(State, clip_plane0), at);
        at += 16;
        assert_eq!(offset_of!(State, vs_bools), at);
        at += 8;
        assert_eq!(offset_of!(State, vs_ints), at);
        at += 2 * 16 * INT_CONSTANTS;
        assert_eq!(offset_of!(State, vs_floats), at);
        at += 16 * VS_FLOAT_CONSTANTS;
        assert_eq!(offset_of!(State, ps_floats), at);
        at += 16 * PS_FLOAT_CONSTANTS;
        assert_eq!(size_of::<State>(), at);
    }
}
