//! Direct3D 9 shader token stream -> instruction list.
//!
//! Every read is bounds-checked: the input comes from disk and from the
//! game, and a truncated or corrupt stream must be an error, never a panic.

use super::{Stage, TranslateError};

// D3DSIO_* opcodes.
pub const OP_NOP: u16 = 0;
pub const OP_MOV: u16 = 1;
pub const OP_ADD: u16 = 2;
pub const OP_SUB: u16 = 3;
pub const OP_MAD: u16 = 4;
pub const OP_MUL: u16 = 5;
pub const OP_RCP: u16 = 6;
pub const OP_RSQ: u16 = 7;
pub const OP_DP3: u16 = 8;
pub const OP_DP4: u16 = 9;
pub const OP_MIN: u16 = 10;
pub const OP_MAX: u16 = 11;
pub const OP_SLT: u16 = 12;
pub const OP_SGE: u16 = 13;
pub const OP_EXP: u16 = 14;
pub const OP_LOG: u16 = 15;
pub const OP_LIT: u16 = 16;
pub const OP_DST: u16 = 17;
pub const OP_LRP: u16 = 18;
pub const OP_FRC: u16 = 19;
pub const OP_M4X4: u16 = 20;
pub const OP_M4X3: u16 = 21;
pub const OP_M3X4: u16 = 22;
pub const OP_M3X3: u16 = 23;
pub const OP_M3X2: u16 = 24;
pub const OP_CALL: u16 = 25;
pub const OP_CALLNZ: u16 = 26;
pub const OP_LOOP: u16 = 27;
pub const OP_RET: u16 = 28;
pub const OP_ENDLOOP: u16 = 29;
pub const OP_LABEL: u16 = 30;
pub const OP_DCL: u16 = 31;
pub const OP_POW: u16 = 32;
pub const OP_CRS: u16 = 33;
pub const OP_SGN: u16 = 34;
pub const OP_ABS: u16 = 35;
pub const OP_NRM: u16 = 36;
pub const OP_SINCOS: u16 = 37;
pub const OP_REP: u16 = 38;
pub const OP_ENDREP: u16 = 39;
pub const OP_IF: u16 = 40;
pub const OP_IFC: u16 = 41;
pub const OP_ELSE: u16 = 42;
pub const OP_ENDIF: u16 = 43;
pub const OP_BREAK: u16 = 44;
pub const OP_BREAKC: u16 = 45;
pub const OP_MOVA: u16 = 46;
pub const OP_DEFB: u16 = 47;
pub const OP_DEFI: u16 = 48;
pub const OP_TEXKILL: u16 = 65;
pub const OP_TEXLD: u16 = 66;
pub const OP_CND: u16 = 80;
pub const OP_DEF: u16 = 81;
pub const OP_CMP: u16 = 88;
pub const OP_DP2ADD: u16 = 90;
pub const OP_DSX: u16 = 91;
pub const OP_DSY: u16 = 92;
pub const OP_TEXLDD: u16 = 93;
pub const OP_SETP: u16 = 94;
pub const OP_TEXLDL: u16 = 95;
pub const OP_BREAKP: u16 = 96;
pub const OP_PHASE: u16 = 0xFFFD;
pub const OP_COMMENT: u16 = 0xFFFE;
pub const OP_END: u16 = 0xFFFF;

// D3DDECLUSAGE_* values the emitter gives special meaning to.
pub const USAGE_POSITION: u8 = 0;
pub const USAGE_PSIZE: u8 = 4;
pub const USAGE_TEXCOORD: u8 = 5;
pub const USAGE_COLOR: u8 = 10;
pub const USAGE_FOG: u8 = 11;

// D3DSPSM_* source modifiers that survive past ps_1_x.
pub const MOD_NONE: u8 = 0;
pub const MOD_NEG: u8 = 1;
pub const MOD_ABS: u8 = 11;
pub const MOD_ABSNEG: u8 = 12;
pub const MOD_NOT: u8 = 13;

/// D3DSHADER_PARAM_REGISTER_TYPE. Register type 3 is the address register
/// in a vertex shader and a texture coordinate in a pixel shader; type 6 is
/// `oTn` before vs_3_0 and the generic `on` from it on. Both keep one name
/// here and the emitter reads them by stage and version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegType {
    Temp,
    Input,
    Const,
    AddrOrTexture,
    RastOut,
    AttrOut,
    Output,
    ConstInt,
    ColorOut,
    DepthOut,
    Sampler,
    ConstBool,
    Loop,
    MiscType,
    Label,
    Predicate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relative {
    pub rtype: RegType,
    /// Which component of the address register indexes.
    pub component: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Src {
    pub rtype: RegType,
    pub num: u32,
    pub swizzle: [u8; 4],
    pub modifier: u8,
    pub rel: Option<Relative>,
}

#[derive(Clone, Copy, Debug)]
pub struct Dst {
    pub rtype: RegType,
    pub num: u32,
    /// Bit n set: component n is written. Never zero.
    pub mask: u8,
    pub saturate: bool,
    pub centroid: bool,
    pub rel: Option<Relative>,
}

#[derive(Clone, Debug)]
pub struct Instr {
    pub opcode: u16,
    /// The opcode-specific control bits: texld's projected/biased variants
    /// and the comparison of ifc, breakc and setp.
    pub control: u8,
    pub dst: Option<Dst>,
    pub predicate: Option<Src>,
    pub srcs: Vec<Src>,
    pub offset: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Decl {
    pub usage: u8,
    pub usage_index: u8,
    /// D3DSAMPLER_TEXTURE_TYPE for a sampler declaration: 2, 3 or 4.
    pub texture_type: u8,
    pub dst: Dst,
    pub offset: usize,
}

#[derive(Debug)]
pub struct Program {
    pub stage: Stage,
    pub major: u8,
    pub minor: u8,
    pub decls: Vec<Decl>,
    pub def_float: Vec<(u32, [u32; 4])>,
    pub def_int: Vec<(u32, [i32; 4])>,
    pub def_bool: Vec<(u32, bool)>,
    pub instrs: Vec<Instr>,
}

pub fn error<T>(message: impl Into<String>, token_offset: usize) -> Result<T, TranslateError> {
    Err(TranslateError {
        message: message.into(),
        token_offset,
    })
}

pub fn opcode_name(opcode: u16) -> &'static str {
    match opcode {
        OP_NOP => "nop",
        OP_MOV => "mov",
        OP_ADD => "add",
        OP_SUB => "sub",
        OP_MAD => "mad",
        OP_MUL => "mul",
        OP_RCP => "rcp",
        OP_RSQ => "rsq",
        OP_DP3 => "dp3",
        OP_DP4 => "dp4",
        OP_MIN => "min",
        OP_MAX => "max",
        OP_SLT => "slt",
        OP_SGE => "sge",
        OP_EXP => "exp",
        OP_LOG => "log",
        OP_LIT => "lit",
        OP_DST => "dst",
        OP_LRP => "lrp",
        OP_FRC => "frc",
        OP_M4X4 => "m4x4",
        OP_M4X3 => "m4x3",
        OP_M3X4 => "m3x4",
        OP_M3X3 => "m3x3",
        OP_M3X2 => "m3x2",
        OP_CALL => "call",
        OP_CALLNZ => "callnz",
        OP_LOOP => "loop",
        OP_RET => "ret",
        OP_ENDLOOP => "endloop",
        OP_LABEL => "label",
        OP_DCL => "dcl",
        OP_POW => "pow",
        OP_CRS => "crs",
        OP_SGN => "sgn",
        OP_ABS => "abs",
        OP_NRM => "nrm",
        OP_SINCOS => "sincos",
        OP_REP => "rep",
        OP_ENDREP => "endrep",
        OP_IF => "if",
        OP_IFC => "ifc",
        OP_ELSE => "else",
        OP_ENDIF => "endif",
        OP_BREAK => "break",
        OP_BREAKC => "breakc",
        OP_MOVA => "mova",
        OP_DEFB => "defb",
        OP_DEFI => "defi",
        OP_TEXKILL => "texkill",
        OP_TEXLD => "texld",
        OP_CND => "cnd",
        OP_DEF => "def",
        OP_CMP => "cmp",
        OP_DP2ADD => "dp2add",
        OP_DSX => "dsx",
        OP_DSY => "dsy",
        OP_TEXLDD => "texldd",
        OP_SETP => "setp",
        OP_TEXLDL => "texldl",
        OP_BREAKP => "breakp",
        OP_PHASE => "phase",
        _ => "?",
    }
}

/// Whether the instruction's first operand is a destination, and how many
/// sources it needs at the least. `None` for an opcode this translator does
/// not know, which includes all of ps_1_x's texture instructions.
fn operands(opcode: u16, major: u8) -> Option<(bool, usize)> {
    Some(match opcode {
        OP_NOP | OP_RET | OP_ENDLOOP | OP_ENDREP | OP_ELSE | OP_ENDIF | OP_BREAK => (false, 0),
        OP_MOV | OP_RCP | OP_RSQ | OP_EXP | OP_LOG | OP_LIT | OP_FRC | OP_ABS | OP_NRM
        | OP_MOVA | OP_DSX | OP_DSY => (true, 1),
        OP_ADD | OP_SUB | OP_MUL | OP_DP3 | OP_DP4 | OP_MIN | OP_MAX | OP_SLT | OP_SGE | OP_DST
        | OP_M4X4 | OP_M4X3 | OP_M3X4 | OP_M3X3 | OP_M3X2 | OP_POW | OP_CRS | OP_SETP
        | OP_TEXLDL => (true, 2),
        OP_MAD | OP_LRP | OP_CMP | OP_CND | OP_DP2ADD | OP_SGN => (true, 3),
        OP_TEXLDD => (true, 4),
        OP_TEXLD => (true, 2),
        OP_TEXKILL => (true, 0),
        // Before shader model 3 the instruction carries the two Taylor
        // series constants it was once implemented with.
        OP_SINCOS => (true, if major >= 3 { 1 } else { 3 }),
        OP_CALL | OP_LABEL | OP_REP | OP_IF | OP_BREAKP => (false, 1),
        OP_CALLNZ | OP_LOOP | OP_IFC | OP_BREAKC => (false, 2),
        _ => return None,
    })
}

struct Reader<'a> {
    words: &'a [u32],
    pos: usize,
}

impl Reader<'_> {
    fn next(&mut self) -> Result<u32, TranslateError> {
        match self.words.get(self.pos) {
            Some(&word) => {
                self.pos += 1;
                Ok(word)
            }
            None => error("token stream ends inside an instruction", self.pos),
        }
    }
}

fn register_type(token: u32, offset: usize) -> Result<RegType, TranslateError> {
    let raw = ((token >> 28) & 0x7) | ((token >> 8) & 0x18);
    Ok(match raw {
        0 => RegType::Temp,
        1 => RegType::Input,
        2 => RegType::Const,
        3 => RegType::AddrOrTexture,
        4 => RegType::RastOut,
        5 => RegType::AttrOut,
        6 => RegType::Output,
        7 => RegType::ConstInt,
        8 => RegType::ColorOut,
        9 => RegType::DepthOut,
        10 => RegType::Sampler,
        // c2048.., c4096.. and c6144..: only software vertex processing
        // has that many, and the device never promises them.
        11..=13 => {
            return error(
                "float constant registers above c2047 are not supported",
                offset,
            )
        }
        14 => RegType::ConstBool,
        15 => RegType::Loop,
        17 => RegType::MiscType,
        18 => RegType::Label,
        19 => RegType::Predicate,
        other => return error(format!("unknown register type {other}"), offset),
    })
}

/// The token after an operand that has the relative-addressing bit set.
/// vs_1_1 has no such token: it can only mean `a0.x`.
fn read_relative(reader: &mut Reader, major: u8) -> Result<Relative, TranslateError> {
    if major < 2 {
        return Ok(Relative {
            rtype: RegType::AddrOrTexture,
            component: 0,
        });
    }
    let offset = reader.pos;
    let token = reader.next()?;
    let rtype = register_type(token, offset)?;
    if !matches!(rtype, RegType::AddrOrTexture | RegType::Loop) {
        return error(
            "relative addressing through a register that is neither a0 nor aL",
            offset,
        );
    }
    Ok(Relative {
        rtype,
        component: ((token >> 16) & 0x3) as u8,
    })
}

fn read_src(reader: &mut Reader, major: u8) -> Result<Src, TranslateError> {
    let offset = reader.pos;
    let token = reader.next()?;
    let rtype = register_type(token, offset)?;
    let modifier = ((token >> 24) & 0xF) as u8;
    if !matches!(
        modifier,
        MOD_NONE | MOD_NEG | MOD_ABS | MOD_ABSNEG | MOD_NOT
    ) {
        return error(
            format!("source modifier {modifier} belongs to ps_1_x and is not supported"),
            offset,
        );
    }
    let mut swizzle = [0u8; 4];
    for (component, slot) in swizzle.iter_mut().enumerate() {
        *slot = ((token >> (16 + 2 * component)) & 0x3) as u8;
    }
    let rel = if token & 0x2000 != 0 {
        Some(read_relative(reader, major)?)
    } else {
        None
    };
    Ok(Src {
        rtype,
        num: token & 0x7FF,
        swizzle,
        modifier,
        rel,
    })
}

fn read_dst(reader: &mut Reader, major: u8) -> Result<Dst, TranslateError> {
    let offset = reader.pos;
    let token = reader.next()?;
    let rtype = register_type(token, offset)?;
    let mask = ((token >> 16) & 0xF) as u8;
    if mask == 0 {
        return error("destination with an empty write mask", offset);
    }
    if (token >> 24) & 0xF != 0 {
        return error(
            "destination shift scale belongs to ps_1_x and is not supported",
            offset,
        );
    }
    let rel = if token & 0x2000 != 0 {
        Some(read_relative(reader, major)?)
    } else {
        None
    };
    Ok(Dst {
        rtype,
        num: token & 0x7FF,
        mask,
        saturate: token & (1 << 20) != 0,
        centroid: token & (4 << 20) != 0,
        rel,
    })
}

pub fn decode(words: &[u32]) -> Result<Program, TranslateError> {
    let mut reader = Reader { words, pos: 0 };
    let version = match reader.next() {
        Ok(version) => version,
        Err(_) => return error("empty token stream", 0),
    };
    let stage = match version >> 16 {
        0xFFFE => Stage::Vertex,
        0xFFFF => Stage::Pixel,
        _ => return error(format!("{version:#010x} is not a shader version token"), 0),
    };
    let major = ((version >> 8) & 0xFF) as u8;
    let minor = (version & 0xFF) as u8;
    if !(1..=3).contains(&major) {
        return error(format!("shader model {major} is not supported"), 0);
    }
    if stage == Stage::Pixel && major == 1 {
        return error("ps_1_x not supported", 0);
    }

    let mut program = Program {
        stage,
        major,
        minor,
        decls: Vec::new(),
        def_float: Vec::new(),
        def_int: Vec::new(),
        def_bool: Vec::new(),
        instrs: Vec::new(),
    };

    loop {
        let offset = reader.pos;
        let token = match reader.next() {
            Ok(token) => token,
            Err(_) => return error("token stream has no end token", offset),
        };
        let opcode = (token & 0xFFFF) as u16;
        match opcode {
            OP_END => return Ok(program),
            OP_COMMENT => {
                let length = ((token >> 16) & 0x7FFF) as usize;
                if length > words.len() - reader.pos {
                    return error("comment runs past the end of the token stream", offset);
                }
                reader.pos += length;
                continue;
            }
            _ => {}
        }
        if token & 0x8000_0000 != 0 {
            return error(format!("{token:#010x} is not an instruction token"), offset);
        }

        // From shader model 2 on the instruction says how long it is, and
        // that is checked against what its operands turned out to occupy.
        let declared_end = (major >= 2).then(|| reader.pos + ((token >> 24) & 0xF) as usize);
        if declared_end.is_some_and(|end| end > words.len()) {
            return error("instruction runs past the end of the token stream", offset);
        }

        match opcode {
            OP_DCL => {
                let usage_token = reader.next()?;
                let dst = read_dst(&mut reader, major)?;
                program.decls.push(Decl {
                    usage: (usage_token & 0x1F) as u8,
                    usage_index: ((usage_token >> 16) & 0xF) as u8,
                    texture_type: ((usage_token >> 27) & 0xF) as u8,
                    dst,
                    offset,
                });
            }
            OP_DEF => {
                let dst = read_dst(&mut reader, major)?;
                if dst.rtype != RegType::Const {
                    return error("def of a register that is not a float constant", offset);
                }
                let mut value = [0u32; 4];
                for slot in &mut value {
                    *slot = reader.next()?;
                }
                program.def_float.push((dst.num, value));
            }
            OP_DEFI => {
                let dst = read_dst(&mut reader, major)?;
                if dst.rtype != RegType::ConstInt {
                    return error("defi of a register that is not an integer constant", offset);
                }
                let mut value = [0i32; 4];
                for slot in &mut value {
                    *slot = reader.next()? as i32;
                }
                program.def_int.push((dst.num, value));
            }
            OP_DEFB => {
                let dst = read_dst(&mut reader, major)?;
                if dst.rtype != RegType::ConstBool {
                    return error("defb of a register that is not a bool constant", offset);
                }
                let value = reader.next()? != 0;
                program.def_bool.push((dst.num, value));
            }
            _ => {
                let Some((has_dst, min_srcs)) = operands(opcode, major) else {
                    return error(
                        format!("opcode {opcode} ({}) is not supported", opcode_name(opcode)),
                        offset,
                    );
                };
                let dst = if has_dst {
                    Some(read_dst(&mut reader, major)?)
                } else {
                    None
                };
                let predicate = if token & 0x1000_0000 != 0 {
                    if dst.is_none() {
                        return error("predicated instruction without a destination", offset);
                    }
                    Some(read_src(&mut reader, major)?)
                } else {
                    None
                };
                let mut srcs = Vec::with_capacity(min_srcs);
                match declared_end {
                    Some(end) => {
                        while reader.pos < end {
                            srcs.push(read_src(&mut reader, major)?);
                        }
                    }
                    None => {
                        for _ in 0..min_srcs {
                            srcs.push(read_src(&mut reader, major)?);
                        }
                    }
                }
                if srcs.len() < min_srcs {
                    return error(
                        format!(
                            "{} has {} source operands and needs {min_srcs}",
                            opcode_name(opcode),
                            srcs.len()
                        ),
                        offset,
                    );
                }
                program.instrs.push(Instr {
                    opcode,
                    control: ((token >> 16) & 0xFF) as u8,
                    dst,
                    predicate,
                    srcs,
                    offset,
                });
            }
        }

        if declared_end.is_some_and(|end| end != reader.pos) {
            return error(
                format!(
                    "{} occupies a different number of tokens than it declares",
                    opcode_name(opcode)
                ),
                offset,
            );
        }
    }
}
