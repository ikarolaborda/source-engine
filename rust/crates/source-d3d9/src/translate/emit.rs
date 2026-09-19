//! Instruction list -> Metal Shading Language.
//!
//! Every register is a `float4` local (the address register an `int4`, the
//! predicate a `bool4`), so an instruction becomes one assignment whose
//! right-hand side is evaluated in full before anything is stored: no
//! instruction can observe its own partial result, whatever its write mask
//! and swizzles alias. The conventions the generated code follows are in
//! `docs/rust-port/d3d9-metal.md`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::decode::*;
use super::{Options, SamplerKind, Stage, TranslateError, Translated, VertexInput};

const COMPONENTS: [char; 4] = ['x', 'y', 'z', 'w'];

const PRELUDE: &str = "\
#include <metal_stdlib>
using namespace metal;

struct D3DExtra {
    int4   ic[16];        // integer constants i0..i15
    float4 clip_plane0;   // clip-space plane; all zero when disabled
    float4 pos_fixup;     // x = 1/viewport_w, y = -1/viewport_h
    uint   bools;         // bit n is bool constant bN
    uint   alpha_func;    // D3DCMP_* 1..8
    float  alpha_ref;     // 0..1
    uint   flags;
};
constant bool fc_alpha_test [[function_constant(0)]];
";

const LIT_HELPER: &str = "
static float4 d3d_lit(float4 s)
{
    float4 r = float4(1.0, 0.0, 0.0, 1.0);
    if (s.x > 0.0) {
        r.y = s.x;
        if (s.y > 0.0) {
            r.z = pow(s.y, clamp(s.w, -127.9961, 127.9961));
        }
    }
    return r;
}
";

/// Interpolators every vertex function declares whether it writes them or
/// not: Metal refuses a pipeline whose fragment function reads one the
/// vertex function lacks.
fn fixed_semantics() -> Vec<(u8, u8)> {
    let mut fixed = vec![(USAGE_COLOR, 0), (USAGE_COLOR, 1)];
    fixed.extend((0..10).map(|index| (USAGE_TEXCOORD, index)));
    fixed.push((USAGE_FOG, 0));
    fixed
}

/// The name an interpolator has on both sides of the link, as the struct
/// member and inside `[[user(...)]]`.
fn semantic_name(usage: u8, index: u8) -> String {
    match (usage, index) {
        (USAGE_TEXCOORD, _) => format!("texcoord{index}"),
        (USAGE_COLOR, _) => format!("color{index}"),
        (USAGE_FOG, 0) => "fog".to_string(),
        _ => format!("u{usage}_{index}"),
    }
}

fn mask_suffix(mask: u8) -> String {
    if mask == 0xF {
        return String::new();
    }
    let mut suffix = String::from(".");
    suffix.extend(
        (0..4)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| COMPONENTS[bit]),
    );
    suffix
}

fn mask_components(mask: u8) -> Vec<u8> {
    (0..4u8).filter(|bit| mask & (1 << bit) != 0).collect()
}

fn vector_type(scalar: &str, width: usize) -> String {
    if width == 1 {
        scalar.to_string()
    } else {
        format!("{scalar}{width}")
    }
}

/// A float literal that reaches the GPU with exactly these bits. Decimal
/// when the decimal survives both ways the Metal compiler may read it (as a
/// float, or as a double it then narrows), the bit pattern otherwise, which
/// also covers infinities, NaNs and negative zero.
fn float_literal(bits: u32) -> String {
    let value = f32::from_bits(bits);
    if value.is_finite() && !(value == 0.0 && value.is_sign_negative()) {
        let text = format!("{value:?}");
        let direct = text.parse::<f32>().ok().map(f32::to_bits);
        let narrowed = text.parse::<f64>().ok().map(|wide| (wide as f32).to_bits());
        if direct == Some(bits) && narrowed == Some(bits) {
            return text;
        }
    }
    format!("as_type<float>({bits:#010x}u)")
}

fn int_literal(value: i32) -> String {
    if value == i32::MIN {
        "(-2147483647 - 1)".to_string()
    } else {
        value.to_string()
    }
}

/// A vertex output interpolator, (usage, usage index), and the
/// (register, write mask) pairs it is assembled from.
type Interpolator = ((u8, u8), Vec<(String, u8)>);

/// What an instruction computed, before the write mask is applied.
enum Value {
    /// Already as wide as the write mask.
    Masked(String),
    /// A scalar every written component receives.
    Scalar(String),
    /// A vector of this many components the write mask selects from. The
    /// expression must accept a swizzle suffix as written.
    Vector(String, u8),
    /// One scalar per component; only the written ones need be present.
    Components([Option<String>; 4]),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Float,
    Int,
    Bool,
}

enum Block {
    If,
    Else,
    Rep,
    Loop(String),
}

struct OutputDecl {
    usage: u8,
    usage_index: u8,
    register: u32,
    mask: u8,
}

struct PixelInputDecl {
    usage: u8,
    usage_index: u8,
    register: u32,
    mask: u8,
    centroid: bool,
}

struct Emitter<'a> {
    program: &'a Program,
    options: &'a Options,
    vertex: bool,
    sm3: bool,
    body: String,
    blocks: Vec<Block>,
    loops_opened: usize,

    def_float: BTreeMap<u32, [u32; 4]>,
    def_int: BTreeMap<u32, [i32; 4]>,
    def_bool: BTreeMap<u32, bool>,

    vertex_inputs: Vec<VertexInput>,
    vertex_outputs: Vec<OutputDecl>,
    pixel_inputs: Vec<PixelInputDecl>,
    /// Pixel `tn` registers a `dcl` marked centroid, before shader model 3.
    centroid_texcoords: u32,
    centroid_colors: u32,
    samplers: [SamplerKind; 16],

    temps: u64,
    inputs_read: u32,
    texcoords_read: u32,
    misc_read: u8,
    address_used: bool,
    predicate_used: bool,
    highest_constant: Option<u32>,
    relative_constants: bool,
    uses_int_constants: bool,
    uses_bool_constants: bool,
    needs_lit: bool,

    rast_written: u8,
    attr_written: u8,
    outputs_written: u32,
    colors_written: u8,
    depth_written: bool,
}

pub fn emit(program: &Program, options: &Options) -> Result<Translated, TranslateError> {
    let mut emitter = Emitter {
        program,
        options,
        vertex: program.stage == Stage::Vertex,
        sm3: program.major >= 3,
        body: String::new(),
        blocks: Vec::new(),
        loops_opened: 0,
        def_float: program.def_float.iter().copied().collect(),
        def_int: program.def_int.iter().copied().collect(),
        def_bool: program.def_bool.iter().copied().collect(),
        vertex_inputs: Vec::new(),
        vertex_outputs: Vec::new(),
        pixel_inputs: Vec::new(),
        centroid_texcoords: 0,
        centroid_colors: 0,
        samplers: [SamplerKind::None; 16],
        temps: 0,
        inputs_read: 0,
        texcoords_read: 0,
        misc_read: 0,
        address_used: false,
        predicate_used: false,
        highest_constant: None,
        relative_constants: false,
        uses_int_constants: false,
        uses_bool_constants: false,
        needs_lit: false,
        rast_written: 0,
        attr_written: 0,
        outputs_written: 0,
        colors_written: 0,
        depth_written: false,
    };
    emitter.declarations()?;
    for instr in &program.instrs {
        emitter.instruction(instr)?;
    }
    if !emitter.blocks.is_empty() {
        return error(
            "flow control block is never closed",
            program.instrs.last().map_or(0, |i| i.offset),
        );
    }
    Ok(emitter.finish())
}

impl Emitter<'_> {
    fn declarations(&mut self) -> Result<(), TranslateError> {
        for decl in &self.program.decls {
            let dst = &decl.dst;
            if dst.rel.is_some() {
                return error("declaration with relative addressing", decl.offset);
            }
            match dst.rtype {
                RegType::Sampler => {
                    if dst.num >= 16 {
                        return error(format!("sampler s{} is out of range", dst.num), decl.offset);
                    }
                    self.samplers[dst.num as usize] = match decl.texture_type {
                        2 => SamplerKind::D2,
                        3 => SamplerKind::Cube,
                        4 => SamplerKind::Volume,
                        other => {
                            return error(
                                format!("sampler texture type {other} is unknown"),
                                decl.offset,
                            )
                        }
                    };
                }
                RegType::Input if self.vertex => {
                    if dst.num >= 16 {
                        return error(format!("input v{} is out of range", dst.num), decl.offset);
                    }
                    if self
                        .vertex_inputs
                        .iter()
                        .all(|input| u32::from(input.register) != dst.num)
                    {
                        self.vertex_inputs.push(VertexInput {
                            register: dst.num as u8,
                            usage: decl.usage,
                            usage_index: decl.usage_index,
                        });
                    }
                }
                RegType::Input => {
                    if dst.num >= 16 {
                        return error(format!("input v{} is out of range", dst.num), decl.offset);
                    }
                    if self.sm3 {
                        self.pixel_inputs.push(PixelInputDecl {
                            usage: decl.usage,
                            usage_index: decl.usage_index,
                            register: dst.num,
                            mask: dst.mask,
                            centroid: dst.centroid,
                        });
                    } else if dst.centroid {
                        self.centroid_colors |= 1 << dst.num;
                    }
                }
                RegType::AddrOrTexture if !self.vertex => {
                    if dst.num >= 16 {
                        return error(format!("input t{} is out of range", dst.num), decl.offset);
                    }
                    if dst.centroid {
                        self.centroid_texcoords |= 1 << dst.num;
                    }
                }
                RegType::Output if self.vertex && self.sm3 => {
                    if dst.num >= 16 {
                        return error(format!("output o{} is out of range", dst.num), decl.offset);
                    }
                    self.vertex_outputs.push(OutputDecl {
                        usage: decl.usage,
                        usage_index: decl.usage_index,
                        register: dst.num,
                        mask: dst.mask,
                    });
                }
                // vPos and vFace: what they are is in the register itself.
                RegType::MiscType if !self.vertex => {}
                _ => {
                    return error(
                        "declaration of a register that cannot be declared",
                        decl.offset,
                    )
                }
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- text

    fn line(&mut self, text: &str) {
        for _ in 0..=self.blocks.len() {
            self.body.push_str("    ");
        }
        self.body.push_str(text);
        self.body.push('\n');
    }

    // ------------------------------------------------------------- sources

    /// The register a source names, as an expression of its full width,
    /// without swizzle or modifier.
    fn source_register(
        &mut self,
        src: &Src,
        offset: usize,
    ) -> Result<(String, Kind), TranslateError> {
        if src.rel.is_some() && src.rtype != RegType::Const {
            return error(
                "relative addressing of a register that is not a float constant",
                offset,
            );
        }
        let num = src.num;
        Ok(match src.rtype {
            RegType::Temp => (self.temp(num, offset)?, Kind::Float),
            RegType::Input if self.vertex => {
                if !self
                    .vertex_inputs
                    .iter()
                    .any(|input| u32::from(input.register) == num)
                {
                    return error(format!("input v{num} is read and never declared"), offset);
                }
                self.inputs_read |= 1 << num;
                (format!("v{num}"), Kind::Float)
            }
            RegType::Input => {
                if self.sm3 {
                    if !self.pixel_inputs.iter().any(|input| input.register == num) {
                        return error(format!("input v{num} is read and never declared"), offset);
                    }
                } else if num >= 2 {
                    return error(format!("input v{num} is out of range"), offset);
                }
                self.inputs_read |= 1 << num;
                (format!("v{num}"), Kind::Float)
            }
            RegType::Const => (self.constant(num, src.rel, offset)?, Kind::Float),
            RegType::AddrOrTexture if !self.vertex => {
                if num >= 16 {
                    return error(format!("input t{num} is out of range"), offset);
                }
                self.texcoords_read |= 1 << num;
                (format!("t{num}"), Kind::Float)
            }
            RegType::ConstInt => (self.int_constant(num, offset)?, Kind::Int),
            RegType::ConstBool => {
                if num >= 32 {
                    return error(format!("bool constant b{num} is out of range"), offset);
                }
                if self.def_bool.contains_key(&num) {
                    (format!("b{num}"), Kind::Bool)
                } else {
                    self.uses_bool_constants = true;
                    (format!("(((ex.bools >> {num}u) & 1u) != 0u)"), Kind::Bool)
                }
            }
            RegType::MiscType if !self.vertex => match num {
                0 => {
                    self.misc_read |= 1;
                    ("vPos".to_string(), Kind::Float)
                }
                1 => {
                    self.misc_read |= 2;
                    ("vFace".to_string(), Kind::Float)
                }
                _ => return error(format!("misc register {num} is unknown"), offset),
            },
            RegType::Predicate => {
                self.predicate_used = true;
                ("p0".to_string(), Kind::Bool)
            }
            other => {
                return error(
                    format!("{other:?} register cannot be a source operand"),
                    offset,
                )
            }
        })
    }

    fn temp(&mut self, num: u32, offset: usize) -> Result<String, TranslateError> {
        if num >= 64 {
            return error(format!("temporary r{num} is out of range"), offset);
        }
        self.temps |= 1 << num;
        Ok(format!("r{num}"))
    }

    fn constant(
        &mut self,
        num: u32,
        rel: Option<Relative>,
        offset: usize,
    ) -> Result<String, TranslateError> {
        let buffer = if self.vertex { "vc" } else { "pc" };
        match rel {
            // Always the uploaded constants, whatever the shader defines:
            // an index is only known on the GPU.
            Some(rel) => {
                self.relative_constants = true;
                let index = self.relative_index(rel, offset)?;
                let last = self.relative_limit() - 1;
                Ok(format!("{buffer}[clamp({index} + {num}, 0, {last})]"))
            }
            None if self.def_float.contains_key(&num) => Ok(format!("c{num}")),
            None => {
                self.highest_constant = Some(
                    self.highest_constant
                        .map_or(num, |highest| highest.max(num)),
                );
                Ok(format!("{buffer}[{num}]"))
            }
        }
    }

    /// How many constants the device uploads for a shader that indexes
    /// them: all a Direct3D 9 device of this stage has.
    fn relative_limit(&self) -> u32 {
        if self.vertex {
            256
        } else {
            224
        }
    }

    fn relative_index(&mut self, rel: Relative, offset: usize) -> Result<String, TranslateError> {
        match rel.rtype {
            RegType::Loop => self.loop_register(offset),
            _ if self.vertex => {
                self.address_used = true;
                Ok(format!("a0.{}", COMPONENTS[rel.component as usize]))
            }
            _ => error("relative addressing through a0 in a pixel shader", offset),
        }
    }

    fn loop_register(&self, offset: usize) -> Result<String, TranslateError> {
        let innermost = self.blocks.iter().rev().find_map(|block| match block {
            Block::Loop(name) => Some(name.clone()),
            _ => None,
        });
        match innermost {
            Some(name) => Ok(name),
            None => error("aL is used outside a loop", offset),
        }
    }

    fn int_constant(&mut self, num: u32, offset: usize) -> Result<String, TranslateError> {
        if num >= 16 {
            return error(format!("integer constant i{num} is out of range"), offset);
        }
        if self.def_int.contains_key(&num) {
            Ok(format!("i{num}"))
        } else {
            self.uses_int_constants = true;
            Ok(format!("ex.ic[{num}]"))
        }
    }

    /// A float source narrowed to `components`, which index the operand's
    /// swizzle: component 2 of `r1.yzxw` is `r1.x`.
    fn source(
        &mut self,
        src: &Src,
        components: &[u8],
        offset: usize,
    ) -> Result<String, TranslateError> {
        let (register, kind) = self.source_register(src, offset)?;
        let register = match kind {
            Kind::Float => register,
            Kind::Int => format!("float4({register})"),
            Kind::Bool => return error("bool register used as a number", offset),
        };
        let swizzle: String = components
            .iter()
            .map(|&at| COMPONENTS[src.swizzle[at as usize] as usize])
            .collect();
        let narrowed = if swizzle == "xyzw" {
            register
        } else {
            format!("{register}.{swizzle}")
        };
        Ok(match src.modifier {
            MOD_NONE => narrowed,
            MOD_NEG => format!("(-{narrowed})"),
            MOD_ABS => format!("abs({narrowed})"),
            MOD_ABSNEG => format!("(-abs({narrowed}))"),
            _ => return error("source modifier ! on a register that is not a bool", offset),
        })
    }

    /// A source used as a condition: a bool constant or predicate components.
    fn condition(
        &mut self,
        src: &Src,
        components: &[u8],
        offset: usize,
    ) -> Result<String, TranslateError> {
        let (register, kind) = self.source_register(src, offset)?;
        if kind != Kind::Bool {
            return error("condition is not a bool register", offset);
        }
        let narrowed = if src.rtype == RegType::Predicate {
            let swizzle: String = components
                .iter()
                .map(|&at| COMPONENTS[src.swizzle[at as usize] as usize])
                .collect();
            format!("{register}.{swizzle}")
        } else {
            register
        };
        Ok(match src.modifier {
            MOD_NONE => narrowed,
            MOD_NOT => format!("(!{narrowed})"),
            _ => return error("numeric source modifier on a bool register", offset),
        })
    }

    // -------------------------------------------------------- destinations

    fn destination(&mut self, dst: &Dst, offset: usize) -> Result<(String, Kind), TranslateError> {
        if dst.rel.is_some() {
            return error(
                "relative addressing of a destination register is not supported",
                offset,
            );
        }
        let num = dst.num;
        Ok(match dst.rtype {
            RegType::Temp => (self.temp(num, offset)?, Kind::Float),
            RegType::AddrOrTexture if self.vertex => {
                if num != 0 {
                    return error(format!("address register a{num} is out of range"), offset);
                }
                self.address_used = true;
                ("a0".to_string(), Kind::Int)
            }
            RegType::RastOut if self.vertex && !self.sm3 => {
                let name = match num {
                    0 => "oPos",
                    1 => "oFog",
                    2 => "oPts",
                    _ => return error(format!("rasterizer output {num} is unknown"), offset),
                };
                self.rast_written |= 1 << num;
                (name.to_string(), Kind::Float)
            }
            RegType::AttrOut if self.vertex && !self.sm3 => {
                if num >= 2 {
                    return error(format!("output oD{num} is out of range"), offset);
                }
                self.attr_written |= 1 << num;
                (format!("oD{num}"), Kind::Float)
            }
            RegType::Output if self.vertex => {
                let limit = if self.sm3 { 16 } else { 8 };
                if num >= limit {
                    return error(format!("output register {num} is out of range"), offset);
                }
                self.outputs_written |= 1 << num;
                (self.output_name(num), Kind::Float)
            }
            RegType::ColorOut if !self.vertex => {
                if num >= 4 {
                    return error(format!("output oC{num} is out of range"), offset);
                }
                self.colors_written |= 1 << num;
                (format!("oC{num}"), Kind::Float)
            }
            RegType::DepthOut if !self.vertex => {
                self.depth_written = true;
                ("oDepth".to_string(), Kind::Float)
            }
            RegType::Predicate => {
                self.predicate_used = true;
                ("p0".to_string(), Kind::Bool)
            }
            other => {
                return error(
                    format!("{other:?} register cannot be a destination"),
                    offset,
                )
            }
        })
    }

    fn output_name(&self, num: u32) -> String {
        if self.sm3 {
            format!("o{num}")
        } else {
            format!("oT{num}")
        }
    }

    /// Applies the write mask, `_sat` and the predicate, and stores.
    fn store(&mut self, instr: &Instr, dst: &Dst, value: Value) -> Result<(), TranslateError> {
        let (target, kind) = self.destination(dst, instr.offset)?;
        let width = dst.mask.count_ones() as usize;
        let scalar = match kind {
            Kind::Float => "float",
            Kind::Int => "int",
            Kind::Bool => "bool",
        };
        let mut rhs = match value {
            Value::Masked(text) => text,
            Value::Scalar(text) if width == 1 => text,
            Value::Scalar(text) => format!("{}({text})", vector_type(scalar, width)),
            Value::Vector(text, available) => {
                if u32::from(dst.mask) >> available != 0 {
                    return error(
                        format!(
                            "{} cannot write every component of its mask",
                            opcode_name(instr.opcode)
                        ),
                        instr.offset,
                    );
                }
                if (available == 4 && dst.mask == 0xF) || (available == 3 && dst.mask == 0x7) {
                    text
                } else {
                    format!("{text}{}", mask_suffix(dst.mask))
                }
            }
            Value::Components(parts) => {
                let mut chosen = Vec::with_capacity(width);
                for at in mask_components(dst.mask) {
                    match &parts[at as usize] {
                        Some(part) => chosen.push(part.clone()),
                        None => {
                            return error(
                                format!(
                                    "{} cannot write component {}",
                                    opcode_name(instr.opcode),
                                    COMPONENTS[at as usize]
                                ),
                                instr.offset,
                            )
                        }
                    }
                }
                if width == 1 {
                    chosen.remove(0)
                } else {
                    format!("{}({})", vector_type(scalar, width), chosen.join(", "))
                }
            }
        };
        if dst.saturate {
            if kind != Kind::Float {
                return error("_sat on a register that is not a float", instr.offset);
            }
            rhs = format!("saturate({rhs})");
        }
        let target = format!("{target}{}", mask_suffix(dst.mask));
        if let Some(predicate) = &instr.predicate {
            let chosen = self.condition(predicate, &mask_components(dst.mask), instr.offset)?;
            rhs = format!("select({target}, {rhs}, {chosen})");
        }
        self.line(&format!("{target} = {rhs};"));
        Ok(())
    }

    // -------------------------------------------------------- instructions

    fn instruction(&mut self, instr: &Instr) -> Result<(), TranslateError> {
        let offset = instr.offset;
        let comment = format!("// {}", self.disassemble(instr));
        if !matches!(instr.opcode, OP_ELSE | OP_ENDIF | OP_ENDREP | OP_ENDLOOP) {
            self.line(&comment);
        }

        let Some(dst) = &instr.dst else {
            return self.flow_control(instr);
        };
        let written = mask_components(dst.mask);
        let width = written.len();
        let srcs = &instr.srcs;
        // Component-wise sources, narrowed to the components being written.
        let each = |emitter: &mut Self, at: usize| emitter.source(&srcs[at], &written, offset);

        let value = match instr.opcode {
            OP_MOV if dst.rtype == RegType::AddrOrTexture && self.vertex => {
                // vs_1_1 has no mova and its mov to a0 truncates downwards.
                let rounding = if self.program.major < 2 {
                    "floor"
                } else {
                    "rint"
                };
                Value::Masked(format!(
                    "{}({rounding}({}))",
                    vector_type("int", width),
                    each(self, 0)?
                ))
            }
            OP_MOVA => Value::Masked(format!(
                "{}(rint({}))",
                vector_type("int", width),
                each(self, 0)?
            )),
            OP_MOV => Value::Masked(each(self, 0)?),
            OP_ADD => Value::Masked(format!("{} + {}", each(self, 0)?, each(self, 1)?)),
            OP_SUB => Value::Masked(format!("{} - {}", each(self, 0)?, each(self, 1)?)),
            OP_MUL => Value::Masked(format!("{} * {}", each(self, 0)?, each(self, 1)?)),
            OP_MAD => Value::Masked(format!(
                "{} * {} + {}",
                each(self, 0)?,
                each(self, 1)?,
                each(self, 2)?
            )),
            OP_RCP => Value::Masked(format!("1.0 / {}", each(self, 0)?)),
            OP_RSQ => Value::Masked(format!("rsqrt(abs({}))", each(self, 0)?)),
            OP_MIN => Value::Masked(format!("min({}, {})", each(self, 0)?, each(self, 1)?)),
            OP_MAX => Value::Masked(format!("max({}, {})", each(self, 0)?, each(self, 1)?)),
            OP_SLT => Value::Masked(format!(
                "{}({} < {})",
                vector_type("float", width),
                each(self, 0)?,
                each(self, 1)?
            )),
            OP_SGE => Value::Masked(format!(
                "{}({} >= {})",
                vector_type("float", width),
                each(self, 0)?,
                each(self, 1)?
            )),
            // Base 2, both of them: the assembler's exp and log are not the
            // natural ones, which is why the compiler scales by 1/ln 2
            // wherever the HLSL said exp().
            OP_EXP => Value::Masked(format!("exp2({})", each(self, 0)?)),
            OP_LOG => Value::Masked(format!("log2(abs({}))", each(self, 0)?)),
            OP_POW => Value::Masked(format!("pow(abs({}), {})", each(self, 0)?, each(self, 1)?)),
            OP_FRC => Value::Masked(format!("fract({})", each(self, 0)?)),
            OP_ABS => Value::Masked(format!("abs({})", each(self, 0)?)),
            OP_SGN => Value::Masked(format!("sign({})", each(self, 0)?)),
            OP_LRP => Value::Masked(format!(
                "mix({2}, {1}, {0})",
                each(self, 0)?,
                each(self, 1)?,
                each(self, 2)?
            )),
            OP_CMP => Value::Masked(format!(
                "select({2}, {1}, {0} >= 0.0)",
                each(self, 0)?,
                each(self, 1)?,
                each(self, 2)?
            )),
            OP_CND => Value::Masked(format!(
                "select({2}, {1}, {0} > 0.5)",
                each(self, 0)?,
                each(self, 1)?,
                each(self, 2)?
            )),
            OP_DSX => Value::Masked(format!("dfdx({})", each(self, 0)?)),
            OP_DSY => Value::Masked(format!("dfdy({})", each(self, 0)?)),
            OP_DP3 => Value::Scalar(format!(
                "dot({}, {})",
                self.source(&srcs[0], &[0, 1, 2], offset)?,
                self.source(&srcs[1], &[0, 1, 2], offset)?
            )),
            OP_DP4 => Value::Scalar(format!(
                "dot({}, {})",
                self.source(&srcs[0], &[0, 1, 2, 3], offset)?,
                self.source(&srcs[1], &[0, 1, 2, 3], offset)?
            )),
            OP_DP2ADD => Value::Scalar(format!(
                "dot({}, {}) + {}",
                self.source(&srcs[0], &[0, 1], offset)?,
                self.source(&srcs[1], &[0, 1], offset)?,
                self.source(&srcs[2], &[0], offset)?
            )),
            OP_CRS => Value::Vector(
                format!(
                    "cross({}, {})",
                    self.source(&srcs[0], &[0, 1, 2], offset)?,
                    self.source(&srcs[1], &[0, 1, 2], offset)?
                ),
                3,
            ),
            OP_NRM => {
                let vector = self.source(&srcs[0], &[0, 1, 2], offset)?;
                if dst.mask == 0x7 {
                    Value::Masked(format!("normalize({vector})"))
                } else {
                    Value::Masked(format!(
                        "{} * rsqrt(dot({vector}, {vector}))",
                        each(self, 0)?
                    ))
                }
            }
            OP_LIT => {
                self.needs_lit = true;
                Value::Vector(
                    format!("d3d_lit({})", self.source(&srcs[0], &[0, 1, 2, 3], offset)?),
                    4,
                )
            }
            OP_DST => Value::Components([
                Some("1.0".to_string()),
                Some(format!(
                    "{} * {}",
                    self.source(&srcs[0], &[1], offset)?,
                    self.source(&srcs[1], &[1], offset)?
                )),
                Some(self.source(&srcs[0], &[2], offset)?),
                Some(self.source(&srcs[1], &[3], offset)?),
            ]),
            OP_SINCOS => {
                let angle = self.source(&srcs[0], &[0], offset)?;
                Value::Components([
                    Some(format!("cos({angle})")),
                    Some(format!("sin({angle})")),
                    None,
                    None,
                ])
            }
            OP_M4X4 | OP_M4X3 | OP_M3X4 | OP_M3X3 | OP_M3X2 => self.matrix(instr, dst)?,
            OP_TEXLD | OP_TEXLDL | OP_TEXLDD => self.texture(instr)?,
            OP_SETP => {
                let comparison = self.comparison(instr.control, offset)?;
                Value::Masked(format!(
                    "({} {comparison} {})",
                    each(self, 0)?,
                    each(self, 1)?
                ))
            }
            OP_TEXKILL => {
                if self.vertex {
                    return error("texkill in a vertex shader", offset);
                }
                let (register, _) = self.destination_as_source(dst, offset)?;
                let tested = format!("{register}{}", mask_suffix(dst.mask));
                if width == 1 {
                    self.line(&format!("if ({tested} < 0.0) discard_fragment();"));
                } else {
                    self.line(&format!("if (any({tested} < 0.0)) discard_fragment();"));
                }
                return Ok(());
            }
            other => {
                return error(
                    format!("opcode {other} ({}) is not supported", opcode_name(other)),
                    offset,
                )
            }
        };
        self.store(instr, dst, value)
    }

    /// texkill names the register it tests in destination form.
    fn destination_as_source(
        &mut self,
        dst: &Dst,
        offset: usize,
    ) -> Result<(String, Kind), TranslateError> {
        let src = Src {
            rtype: dst.rtype,
            num: dst.num,
            swizzle: [0, 1, 2, 3],
            modifier: MOD_NONE,
            rel: dst.rel,
        };
        self.source_register(&src, offset)
    }

    /// `mNxM dst, vector, cK`: component i is the dot product with `c(K+i)`.
    fn matrix(&mut self, instr: &Instr, dst: &Dst) -> Result<Value, TranslateError> {
        let (rows, columns): (u32, &[u8]) = match instr.opcode {
            OP_M4X4 => (4, &[0, 1, 2, 3]),
            OP_M4X3 => (3, &[0, 1, 2, 3]),
            OP_M3X4 => (4, &[0, 1, 2]),
            OP_M3X3 => (3, &[0, 1, 2]),
            _ => (2, &[0, 1, 2]),
        };
        let vector = self.source(&instr.srcs[0], columns, instr.offset)?;
        let mut parts = [None, None, None, None];
        for row in 0..rows {
            if dst.mask & (1 << row) == 0 {
                continue;
            }
            let mut matrix_row = instr.srcs[1];
            matrix_row.num += row;
            let matrix_row = self.source(&matrix_row, columns, instr.offset)?;
            parts[row as usize] = Some(format!("dot({vector}, {matrix_row})"));
        }
        Ok(Value::Components(parts))
    }

    fn texture(&mut self, instr: &Instr) -> Result<Value, TranslateError> {
        let offset = instr.offset;
        let sampler = &instr.srcs[1];
        if sampler.rtype != RegType::Sampler || sampler.rel.is_some() {
            return error(
                "texture lookup through something that is not a sampler",
                offset,
            );
        }
        let unit = sampler.num as usize;
        if unit >= 16 {
            return error(format!("sampler s{unit} is out of range"), offset);
        }
        // Only ps_1_x may sample without a declaration, and it is always 2D.
        if self.samplers[unit] == SamplerKind::None {
            self.samplers[unit] = SamplerKind::D2;
        }
        let kind = self.samplers[unit];
        let shadow = kind == SamplerKind::D2 && self.options.shadow_samplers & (1 << unit) != 0;

        let coordinate = &instr.srcs[0];
        let lookup: &[u8] = if kind == SamplerKind::D2 {
            &[0, 1]
        } else {
            &[0, 1, 2]
        };
        let mut at = self.source(coordinate, lookup, offset)?;
        let mut depth = if shadow {
            Some(self.source(coordinate, &[2], offset)?)
        } else {
            None
        };
        let fourth = self.source(coordinate, &[3], offset)?;

        let mut how = String::new();
        match (instr.opcode, instr.control) {
            (OP_TEXLD, 0) => {}
            (OP_TEXLD, 1) => {
                at = format!("{at} / {fourth}");
                depth = depth.map(|depth| format!("{depth} / {fourth}"));
            }
            // A depth comparison has no biased form in Metal.
            (OP_TEXLD, 2) if shadow => {}
            (OP_TEXLD, 2) => how = format!(", bias({fourth})"),
            (OP_TEXLD, other) => return error(format!("texld variant {other} is unknown"), offset),
            (OP_TEXLDL, _) => how = format!(", level({fourth})"),
            _ => {
                let gradient = match kind {
                    SamplerKind::Cube => "gradientcube",
                    SamplerKind::Volume => "gradient3d",
                    _ => "gradient2d",
                };
                let dx = self.source(&instr.srcs[2], lookup, offset)?;
                let dy = self.source(&instr.srcs[3], lookup, offset)?;
                if !shadow {
                    how = format!(", {gradient}({dx}, {dy})");
                }
            }
        }
        // A vertex function has no derivatives to pick a level with.
        if self.vertex && how.is_empty() {
            how = ", level(0.0)".to_string();
        }

        let mut fetched = match depth {
            Some(depth) => {
                format!("float4(tex{unit}.sample_compare(smp{unit}, {at}, {depth}{how}))")
            }
            None => format!("tex{unit}.sample(smp{unit}, {at}{how})"),
        };
        if sampler.swizzle != [0, 1, 2, 3] {
            fetched.push('.');
            fetched.extend(
                sampler
                    .swizzle
                    .iter()
                    .map(|&from| COMPONENTS[from as usize]),
            );
        }
        Ok(Value::Vector(fetched, 4))
    }

    fn comparison(&self, control: u8, offset: usize) -> Result<&'static str, TranslateError> {
        Ok(match control & 0x7 {
            1 => ">",
            2 => "==",
            3 => ">=",
            4 => "<",
            5 => "!=",
            6 => "<=",
            other => return error(format!("comparison {other} is unknown"), offset),
        })
    }

    fn flow_control(&mut self, instr: &Instr) -> Result<(), TranslateError> {
        let offset = instr.offset;
        let srcs = &instr.srcs;
        match instr.opcode {
            OP_NOP => {}
            OP_IF => {
                let condition = self.condition(&srcs[0], &[0], offset)?;
                self.line(&format!("if ({condition}) {{"));
                self.blocks.push(Block::If);
            }
            OP_IFC => {
                let comparison = self.comparison(instr.control, offset)?;
                let left = self.source(&srcs[0], &[0], offset)?;
                let right = self.source(&srcs[1], &[0], offset)?;
                self.line(&format!("if ({left} {comparison} {right}) {{"));
                self.blocks.push(Block::If);
            }
            OP_ELSE => {
                if !matches!(self.blocks.pop(), Some(Block::If)) {
                    return error("else without an if", offset);
                }
                self.line("} else {");
                self.blocks.push(Block::Else);
            }
            OP_ENDIF => {
                if !matches!(self.blocks.pop(), Some(Block::If | Block::Else)) {
                    return error("endif without an if", offset);
                }
                self.line("}");
            }
            OP_REP => {
                if srcs[0].rtype != RegType::ConstInt {
                    return error("rep count is not an integer constant", offset);
                }
                let count = self.int_constant(srcs[0].num, offset)?;
                let counter = format!("rep{}", self.loops_opened);
                self.loops_opened += 1;
                // Direct3D 9 runs a loop 255 times at the most; a constant
                // nobody set must not hang the GPU.
                self.line(&format!(
                    "for (int {counter} = 0; {counter} < min({count}.x, 255); ++{counter}) {{"
                ));
                self.blocks.push(Block::Rep);
            }
            OP_LOOP => {
                if srcs[0].rtype != RegType::Loop || srcs[1].rtype != RegType::ConstInt {
                    return error("loop operands are not aL and an integer constant", offset);
                }
                let setup = self.int_constant(srcs[1].num, offset)?;
                let counter = format!("lc{}", self.loops_opened);
                let register = format!("aL{}", self.loops_opened);
                self.loops_opened += 1;
                self.line(&format!(
                    "for (int {counter} = 0, {register} = {setup}.y; {counter} < min({setup}.x, 255); \
                     ++{counter}, {register} += {setup}.z) {{"
                ));
                self.blocks.push(Block::Loop(register));
            }
            OP_ENDREP => {
                if !matches!(self.blocks.pop(), Some(Block::Rep)) {
                    return error("endrep without a rep", offset);
                }
                self.line("}");
            }
            OP_ENDLOOP => {
                if !matches!(self.blocks.pop(), Some(Block::Loop(_))) {
                    return error("endloop without a loop", offset);
                }
                self.line("}");
            }
            OP_BREAK | OP_BREAKC | OP_BREAKP => {
                if !self
                    .blocks
                    .iter()
                    .any(|block| matches!(block, Block::Rep | Block::Loop(_)))
                {
                    return error("break outside a loop", offset);
                }
                match instr.opcode {
                    OP_BREAK => self.line("break;"),
                    OP_BREAKC => {
                        let comparison = self.comparison(instr.control, offset)?;
                        let left = self.source(&srcs[0], &[0], offset)?;
                        let right = self.source(&srcs[1], &[0], offset)?;
                        self.line(&format!("if ({left} {comparison} {right}) break;"));
                    }
                    _ => {
                        let condition = self.condition(&srcs[0], &[0], offset)?;
                        self.line(&format!("if ({condition}) break;"));
                    }
                }
            }
            OP_CALL | OP_CALLNZ | OP_LABEL | OP_RET => {
                return error(
                    format!(
                        "subroutines ({}) are not supported",
                        opcode_name(instr.opcode)
                    ),
                    offset,
                )
            }
            other => {
                return error(
                    format!("opcode {other} ({}) is not supported", opcode_name(other)),
                    offset,
                )
            }
        }
        Ok(())
    }

    // --------------------------------------------------------- disassembly

    fn register_name(&self, rtype: RegType, num: u32, rel: Option<Relative>) -> String {
        let name = match rtype {
            RegType::Temp => format!("r{num}"),
            RegType::Input => format!("v{num}"),
            RegType::Const => format!("c{num}"),
            RegType::AddrOrTexture if self.vertex => format!("a{num}"),
            RegType::AddrOrTexture => format!("t{num}"),
            RegType::RastOut => ["oPos", "oFog", "oPts"]
                .get(num as usize)
                .unwrap_or(&"oRast?")
                .to_string(),
            RegType::AttrOut => format!("oD{num}"),
            RegType::Output => self.output_name(num),
            RegType::ConstInt => format!("i{num}"),
            RegType::ColorOut => format!("oC{num}"),
            RegType::DepthOut => "oDepth".to_string(),
            RegType::Sampler => format!("s{num}"),
            RegType::ConstBool => format!("b{num}"),
            RegType::Loop => "aL".to_string(),
            RegType::MiscType => ["vPos", "vFace"]
                .get(num as usize)
                .unwrap_or(&"vMisc?")
                .to_string(),
            RegType::Label => format!("l{num}"),
            RegType::Predicate => format!("p{num}"),
        };
        match rel {
            Some(rel) if rel.rtype == RegType::Loop => format!("{name}[aL]"),
            Some(rel) => format!("{name}[a0.{}]", COMPONENTS[rel.component as usize]),
            None => name,
        }
    }

    fn disassemble_source(&self, src: &Src) -> String {
        let mut text = self.register_name(src.rtype, src.num, src.rel);
        if src.swizzle != [0, 1, 2, 3] {
            text.push('.');
            if src.swizzle.iter().all(|&from| from == src.swizzle[0]) {
                text.push(COMPONENTS[src.swizzle[0] as usize]);
            } else {
                text.extend(src.swizzle.iter().map(|&from| COMPONENTS[from as usize]));
            }
        }
        match src.modifier {
            MOD_NEG => format!("-{text}"),
            MOD_ABS => format!("{text}_abs"),
            MOD_ABSNEG => format!("-{text}_abs"),
            MOD_NOT => format!("!{text}"),
            _ => text,
        }
    }

    fn disassemble(&self, instr: &Instr) -> String {
        let mut text = match (instr.opcode, instr.control) {
            (OP_TEXLD, 1) => "texldp".to_string(),
            (OP_TEXLD, 2) => "texldb".to_string(),
            (OP_IFC | OP_BREAKC | OP_SETP, control) => {
                let comparison =
                    ["", "_gt", "_eq", "_ge", "_lt", "_ne", "_le", ""][(control & 0x7) as usize];
                let base = if instr.opcode == OP_IFC {
                    "if"
                } else {
                    opcode_name(instr.opcode)
                };
                format!("{base}{comparison}")
            }
            (opcode, _) => opcode_name(opcode).to_string(),
        };
        let mut operands = Vec::new();
        if let Some(dst) = &instr.dst {
            if dst.saturate {
                text.push_str("_sat");
            }
            operands.push(format!(
                "{}{}",
                self.register_name(dst.rtype, dst.num, dst.rel),
                mask_suffix(dst.mask)
            ));
        }
        if let Some(predicate) = &instr.predicate {
            text = format!("({}) {text}", self.disassemble_source(predicate));
        }
        operands.extend(instr.srcs.iter().map(|src| self.disassemble_source(src)));
        if operands.is_empty() {
            text
        } else {
            format!("{text} {}", operands.join(", "))
        }
    }

    // ------------------------------------------------------------ assembly

    fn finish(self) -> Translated {
        let mut msl = String::from(PRELUDE);
        if self.needs_lit {
            msl.push_str(LIT_HELPER);
        }
        let stage = if self.vertex { "vs" } else { "ps" };
        let _ = writeln!(
            msl,
            "\n// {stage}_{}_{}",
            self.program.major, self.program.minor
        );

        let mut locals = String::new();
        for (num, value) in &self.def_float {
            let value: Vec<String> = value.iter().map(|&bits| float_literal(bits)).collect();
            let _ = writeln!(
                locals,
                "    const float4 c{num} = float4({});",
                value.join(", ")
            );
        }
        for (num, value) in &self.def_int {
            let value: Vec<String> = value.iter().map(|&each| int_literal(each)).collect();
            let _ = writeln!(
                locals,
                "    const int4 i{num} = int4({});",
                value.join(", ")
            );
        }
        for (num, value) in &self.def_bool {
            let _ = writeln!(locals, "    const bool b{num} = {value};");
        }

        let mut textures = String::new();
        for (unit, kind) in self.samplers.iter().enumerate() {
            let shadow = self.options.shadow_samplers & (1 << unit) != 0;
            let texture = match kind {
                SamplerKind::None => continue,
                SamplerKind::D2 if shadow => "depth2d<float>",
                SamplerKind::D2 => "texture2d<float>",
                SamplerKind::Cube => "texturecube<float>",
                SamplerKind::Volume => "texture3d<float>",
            };
            let _ = write!(
                textures,
                ",\n    {texture} tex{unit} [[texture({unit})]],\n    sampler smp{unit} [[sampler({unit})]]"
            );
        }

        let (signature, epilogue) = if self.vertex {
            self.vertex_interface(&mut msl, &mut locals, &textures)
        } else {
            self.pixel_interface(&mut msl, &mut locals, &textures)
        };

        for num in (0..64).filter(|num| self.temps & (1u64 << num) != 0) {
            let _ = writeln!(locals, "    float4 r{num} = float4(0.0);");
        }
        if self.address_used {
            locals.push_str("    int4 a0 = int4(0);\n");
        }
        if self.predicate_used {
            locals.push_str("    bool4 p0 = bool4(false);\n");
        }

        msl.push_str(&signature);
        msl.push_str("{\n");
        msl.push_str(&locals);
        msl.push('\n');
        msl.push_str(&self.body);
        msl.push('\n');
        msl.push_str(&epilogue);
        msl.push_str("}\n");

        let writes_point_size = self.vertex_point_size().is_some();
        let float_constants = if self.relative_constants {
            self.relative_limit()
                .max(self.highest_constant.map_or(0, |highest| highest + 1))
        } else {
            self.highest_constant.map_or(0, |highest| highest + 1)
        };
        Translated {
            stage: self.program.stage,
            major: self.program.major,
            minor: self.program.minor,
            msl,
            inputs: self.vertex_inputs,
            samplers: self.samplers,
            float_constants,
            uses_int_constants: self.uses_int_constants,
            uses_bool_constants: self.uses_bool_constants,
            color_outputs: self.colors_written,
            writes_depth: self.depth_written,
            writes_point_size,
        }
    }

    /// The expression holding the point size, when the shader writes one.
    fn vertex_point_size(&self) -> Option<String> {
        if !self.vertex {
            return None;
        }
        if !self.sm3 {
            return (self.rast_written & 4 != 0).then(|| "oPts.x".to_string());
        }
        self.vertex_outputs
            .iter()
            .find(|output| {
                output.usage == USAGE_PSIZE && self.outputs_written & (1 << output.register) != 0
            })
            .map(|output| {
                let first = output.mask.trailing_zeros() as usize;
                format!("o{}.{}", output.register, COMPONENTS[first])
            })
    }

    /// Declares the structs, returns the function signature and epilogue.
    fn vertex_interface(
        &self,
        msl: &mut String,
        locals: &mut String,
        textures: &str,
    ) -> (String, String) {
        let read: Vec<&VertexInput> = self
            .vertex_inputs
            .iter()
            .filter(|input| self.inputs_read & (1 << input.register) != 0)
            .collect();
        if !read.is_empty() {
            msl.push_str("struct VsIn {\n");
            for input in &read {
                let register = input.register;
                let _ = writeln!(msl, "    float4 v{register} [[attribute({register})]];");
                let _ = writeln!(locals, "    const float4 v{register} = in.v{register};");
            }
            msl.push_str("};\n");
        }

        // What each interpolator is assembled from: (register, mask).
        let mut position: Option<String> = None;
        let mut semantics: Vec<Interpolator> = fixed_semantics()
            .into_iter()
            .map(|semantic| (semantic, Vec::new()))
            .collect();
        let mut feed = |semantic: (u8, u8), register: String, mask: u8| match semantics
            .iter_mut()
            .find(|(existing, _)| *existing == semantic)
        {
            Some((_, sources)) => sources.push((register, mask)),
            None => semantics.push((semantic, vec![(register, mask)])),
        };
        if self.sm3 {
            for output in &self.vertex_outputs {
                let register = format!("o{}", output.register);
                match (output.usage, output.usage_index) {
                    (USAGE_POSITION, 0) => position = Some(register),
                    (USAGE_PSIZE, _) => {}
                    semantic => feed(semantic, register, output.mask),
                }
            }
            let declared = self
                .vertex_outputs
                .iter()
                .fold(0u32, |all, output| all | (1 << output.register));
            for num in (0..16).filter(|num| (self.outputs_written | declared) & (1 << num) != 0) {
                let _ = writeln!(locals, "    float4 o{num} = float4(0.0);");
            }
        } else {
            position = Some("oPos".to_string());
            locals.push_str("    float4 oPos = float4(0.0);\n");
            if self.rast_written & 2 != 0 {
                locals.push_str("    float4 oFog = float4(0.0);\n");
                feed((USAGE_FOG, 0), "oFog".to_string(), 0xF);
            }
            if self.rast_written & 4 != 0 {
                locals.push_str("    float4 oPts = float4(0.0);\n");
            }
            for num in (0..2u8).filter(|num| self.attr_written & (1 << num) != 0) {
                let _ = writeln!(locals, "    float4 oD{num} = float4(0.0);");
                feed((USAGE_COLOR, num), format!("oD{num}"), 0xF);
            }
            for num in (0..8u8).filter(|num| self.outputs_written & (1 << num) != 0) {
                let _ = writeln!(locals, "    float4 oT{num} = float4(0.0);");
                feed((USAGE_TEXCOORD, num), format!("oT{num}"), 0xF);
            }
        }
        let point_size = self.vertex_point_size();

        msl.push_str("struct VsOut {\n    float4 position [[position]];\n    float clip0 [[clip_distance]];\n");
        if point_size.is_some() {
            msl.push_str("    float psize [[point_size]];\n");
        }
        for ((usage, index), _) in &semantics {
            let name = semantic_name(*usage, *index);
            let _ = writeln!(msl, "    float4 {name} [[user({name})]];");
        }
        msl.push_str("};\n\n");

        let mut signature = String::from("vertex VsOut vs_main(\n");
        if !read.is_empty() {
            signature.push_str("    VsIn in [[stage_in]],\n");
        }
        signature.push_str(
            "    constant float4 *vc [[buffer(16)]],\n    constant D3DExtra &ex [[buffer(17)]]",
        );
        signature.push_str(textures);
        signature.push_str(")\n");

        let mut epilogue = String::from("    VsOut out;\n");
        for ((usage, index), sources) in &semantics {
            let name = semantic_name(*usage, *index);
            // Before shader model 3 the two colours are clamped on their
            // way out of the vertex stage.
            let clamp = !self.sm3 && *usage == USAGE_COLOR;
            match sources.as_slice() {
                [] => {
                    let _ = writeln!(epilogue, "    out.{name} = float4(0.0);");
                }
                [(register, 0xF)] if clamp => {
                    let _ = writeln!(epilogue, "    out.{name} = saturate({register});");
                }
                [(register, 0xF)] => {
                    let _ = writeln!(epilogue, "    out.{name} = {register};");
                }
                several => {
                    let _ = writeln!(epilogue, "    out.{name} = float4(0.0);");
                    for (register, mask) in several {
                        let suffix = mask_suffix(*mask);
                        let _ = writeln!(epilogue, "    out.{name}{suffix} = {register}{suffix};");
                    }
                }
            }
        }
        if let Some(point_size) = point_size {
            let _ = writeln!(epilogue, "    out.psize = {point_size};");
        }
        let position = position.unwrap_or_else(|| "float4(0.0)".to_string());
        let _ = writeln!(epilogue, "    float4 d3d_pos = {position};");
        epilogue.push_str(
            "    out.clip0 = dot(d3d_pos, ex.clip_plane0);\n    \
             d3d_pos.xy += ex.pos_fixup.xy * d3d_pos.w;\n    \
             out.position = d3d_pos;\n    \
             return out;\n",
        );
        (signature, epilogue)
    }

    fn pixel_interface(
        &self,
        msl: &mut String,
        locals: &mut String,
        textures: &str,
    ) -> (String, String) {
        // Interpolator members, in first-use order: (name, centroid).
        let mut members: Vec<(String, bool)> = Vec::new();
        let mut member = |name: String, centroid: bool| match members
            .iter_mut()
            .find(|(existing, _)| *existing == name)
        {
            Some((_, already)) => *already |= centroid,
            None => members.push((name, centroid)),
        };
        if self.sm3 {
            for num in (0..16u32).filter(|num| self.inputs_read & (1 << num) != 0) {
                let parts: Vec<&PixelInputDecl> = self
                    .pixel_inputs
                    .iter()
                    .filter(|input| input.register == num)
                    .collect();
                for part in &parts {
                    member(semantic_name(part.usage, part.usage_index), part.centroid);
                }
                if let [only] = parts.as_slice() {
                    let name = semantic_name(only.usage, only.usage_index);
                    let _ = writeln!(locals, "    const float4 v{num} = in.{name};");
                } else {
                    let _ = writeln!(locals, "    float4 v{num} = float4(0.0);");
                    for part in parts {
                        let name = semantic_name(part.usage, part.usage_index);
                        let suffix = mask_suffix(part.mask);
                        let _ = writeln!(locals, "    v{num}{suffix} = in.{name}{suffix};");
                    }
                }
            }
        } else {
            for num in (0..2u8).filter(|num| self.inputs_read & (1 << num) != 0) {
                let name = semantic_name(USAGE_COLOR, num);
                let _ = writeln!(locals, "    const float4 v{num} = in.{name};");
                member(name, self.centroid_colors & (1 << num) != 0);
            }
            for num in (0..16u8).filter(|num| self.texcoords_read & (1 << num) != 0) {
                let name = semantic_name(USAGE_TEXCOORD, num);
                let _ = writeln!(locals, "    const float4 t{num} = in.{name};");
                member(name, self.centroid_texcoords & (1 << num) != 0);
            }
        }
        if self.misc_read & 1 != 0 {
            // Direct3D 9 puts pixel centres on the integers.
            locals.push_str("    const float4 vPos = float4(floor(in.position.xy), 0.0, 0.0);\n");
        }
        if self.misc_read & 2 != 0 {
            locals.push_str("    const float4 vFace = float4(d3d_front ? 1.0 : -1.0);\n");
        }

        let has_input = !members.is_empty() || self.misc_read & 1 != 0;
        if has_input {
            msl.push_str("struct PsIn {\n");
            if self.misc_read & 1 != 0 {
                msl.push_str("    float4 position [[position]];\n");
            }
            for (name, centroid) in &members {
                let interpolation = if *centroid {
                    ", centroid_perspective"
                } else {
                    ""
                };
                let _ = writeln!(msl, "    float4 {name} [[user({name}){interpolation}]];");
            }
            msl.push_str("};\n");
        }

        // A shader that writes nothing still has to return something.
        let colors = if self.colors_written == 0 && !self.depth_written {
            1
        } else {
            self.colors_written
        };
        msl.push_str("struct PsOut {\n");
        for num in (0..4).filter(|num| colors & (1 << num) != 0) {
            let _ = writeln!(msl, "    float4 color{num} [[color({num})]];");
            let _ = writeln!(locals, "    float4 oC{num} = float4(0.0);");
        }
        if self.depth_written {
            msl.push_str("    float depth [[depth(any)]];\n");
            locals.push_str("    float4 oDepth = float4(0.0);\n");
        }
        msl.push_str("};\n\n");

        let mut signature = String::from("fragment PsOut ps_main(\n");
        if has_input {
            signature.push_str("    PsIn in [[stage_in]],\n");
        }
        if self.misc_read & 2 != 0 {
            signature.push_str("    bool d3d_front [[front_facing]],\n");
        }
        signature.push_str(
            "    constant float4 *pc [[buffer(0)]],\n    constant D3DExtra &ex [[buffer(1)]]",
        );
        signature.push_str(textures);
        signature.push_str(")\n");

        let mut epilogue = String::new();
        if self.colors_written & 1 != 0 {
            epilogue.push_str(
                "    if (fc_alpha_test) {\n        \
                 const float d3d_alpha = oC0.w;\n        \
                 const float d3d_ref = ex.alpha_ref;\n        \
                 bool d3d_pass = true;\n        \
                 switch (ex.alpha_func) {\n        \
                 case 1u: d3d_pass = false; break;\n        \
                 case 2u: d3d_pass = d3d_alpha < d3d_ref; break;\n        \
                 case 3u: d3d_pass = d3d_alpha == d3d_ref; break;\n        \
                 case 4u: d3d_pass = d3d_alpha <= d3d_ref; break;\n        \
                 case 5u: d3d_pass = d3d_alpha > d3d_ref; break;\n        \
                 case 6u: d3d_pass = d3d_alpha != d3d_ref; break;\n        \
                 case 7u: d3d_pass = d3d_alpha >= d3d_ref; break;\n        \
                 default: break;\n        \
                 }\n        \
                 if (!d3d_pass) discard_fragment();\n    \
                 }\n",
            );
        }
        epilogue.push_str("    PsOut out;\n");
        for num in (0..4).filter(|num| colors & (1 << num) != 0) {
            let _ = writeln!(epilogue, "    out.color{num} = oC{num};");
        }
        if self.depth_written {
            epilogue.push_str("    out.depth = oDepth.x;\n");
        }
        epilogue.push_str("    return out;\n");
        (signature, epilogue)
    }
}
