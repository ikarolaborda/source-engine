//! Direct3D 9 shader bytecode to Metal Shading Language.
//!
//! The game ships its shaders compiled, as vs_2_0 and ps_2_b token streams
//! for the most part, and `shaderapidx9` hands them to the device as they
//! are. ToGL turned them into GLSL; this turns them into Metal source the
//! device compiles at run time. vs_1_1 through vs_3_0 and ps_2_0 through
//! ps_3_0 are understood; ps_1_x is not, and nothing the game selects on a
//! shader model 2 device uses it.
//!
//! What the generated source looks like from outside (entry points, buffer
//! and texture indices, how interpolators are named so that any vertex
//! function links with any pixel function) is fixed by
//! `docs/rust-port/d3d9-metal.md`; the device is written against the same
//! document.
//!
//! Where Direct3D's documentation and its compiler disagree, the compiler
//! wins, because its output is what is being run: `exp` and `log` are base
//! 2, which is visible in the 1/ln 2 the compiler multiplies by wherever
//! the HLSL said `exp()`, and is also what ToGL does. `rsq` and `log` take
//! the absolute value of their operand and `pow` of its base, as the
//! reference rasterizer does. Before shader model 3 the two colour outputs
//! of a vertex shader are clamped to 0..1.

mod decode;
mod emit;
#[cfg(test)]
mod tests;

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Pixel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SamplerKind {
    #[default]
    None,
    D2,
    Cube,
    Volume,
}

/// One declared vertex shader input: `dcl_<usage><usage_index> v<register>`.
/// The vertex function reads it from `[[attribute(register)]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexInput {
    pub register: u8,
    /// A `D3DDECLUSAGE_*` value.
    pub usage: u8,
    pub usage_index: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Bit n set: sampler n is a hardware shadow-map lookup. It is declared
    /// `depth2d<float>` and read with `sample_compare`: at xy/w against z/w
    /// for a projective lookup, at xy against z otherwise.
    pub shadow_samplers: u32,
}

#[derive(Clone, Debug)]
pub struct Translated {
    pub stage: Stage,
    pub major: u8,
    pub minor: u8,
    /// Complete Metal source: includes, the shared prelude and the entry
    /// point, `vs_main` or `ps_main`.
    pub msl: String,
    /// Vertex stage only: one per declared input register.
    pub inputs: Vec<VertexInput>,
    /// The kind of every sampler register the shader declares or uses.
    pub samplers: [SamplerKind; 16],
    /// How many float constant registers the device must upload: the
    /// highest one read from the constant buffer, plus one. A shader that
    /// indexes its constants through a register needs all of them, which
    /// is 256 for a vertex shader.
    pub float_constants: u32,
    pub uses_int_constants: bool,
    pub uses_bool_constants: bool,
    /// Pixel stage: bit n set when `oCn` is written.
    pub color_outputs: u8,
    /// Pixel stage: `oDepth` is written.
    pub writes_depth: bool,
    /// Vertex stage: the point size is written.
    pub writes_point_size: bool,
}

#[derive(Debug)]
pub struct TranslateError {
    pub message: String,
    /// Index of the token the error was found at.
    pub token_offset: usize,
}

impl fmt::Display for TranslateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} (at token {})",
            self.message, self.token_offset
        )
    }
}

impl std::error::Error for TranslateError {}

/// Translates one shader. `bytecode` starts at the version token and must
/// contain the end token; anything after it is ignored. Malformed input is
/// an error, never a panic.
pub fn translate(bytecode: &[u32], options: &Options) -> Result<Translated, TranslateError> {
    let program = decode::decode(bytecode)?;
    emit::emit(&program, options)
}

/// The token stream in a shader file: little-endian 32-bit words. Bytes
/// past the last whole word are dropped.
pub fn words_from_bytes(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect()
}
