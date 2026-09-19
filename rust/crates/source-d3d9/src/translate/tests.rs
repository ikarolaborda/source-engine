//! Hand-assembled token streams. The corpus test in `tests/` covers what
//! the game really ships; these pin down the forms it happens not to use.

use super::*;

const VS_1_1: u32 = 0xFFFE_0101;
const VS_2_0: u32 = 0xFFFE_0200;
const VS_3_0: u32 = 0xFFFE_0300;
const PS_2_0: u32 = 0xFFFF_0200;
const PS_3_0: u32 = 0xFFFF_0300;
const END: u32 = 0x0000_FFFF;

// Register types, as the token encodes them.
const TEMP: u32 = 0;
const INPUT: u32 = 1;
const CONST: u32 = 2;
const ADDR: u32 = 3;
const TEXTURE: u32 = 3;
const RASTOUT: u32 = 4;
const OUTPUT: u32 = 6;
const CONSTINT: u32 = 7;
const COLOROUT: u32 = 8;
const SAMPLER: u32 = 10;
const CONSTBOOL: u32 = 14;
const LOOP: u32 = 15;
const MISC: u32 = 17;

const XYZW: u32 = 0xF;
const RELATIVE: u32 = 0x2000;

fn register(rtype: u32, num: u32) -> u32 {
    0x8000_0000 | ((rtype & 0x7) << 28) | ((rtype & 0x18) << 8) | num
}

fn dst(rtype: u32, num: u32, mask: u32) -> u32 {
    register(rtype, num) | (mask << 16)
}

fn saturated(token: u32) -> u32 {
    token | (1 << 20)
}

fn src(rtype: u32, num: u32) -> u32 {
    swizzled(rtype, num, [0, 1, 2, 3])
}

fn swizzled(rtype: u32, num: u32, swizzle: [u32; 4]) -> u32 {
    let bits = swizzle[0] | (swizzle[1] << 2) | (swizzle[2] << 4) | (swizzle[3] << 6);
    register(rtype, num) | (bits << 16)
}

fn negated(token: u32) -> u32 {
    token | (1 << 24)
}

/// An instruction with its length filled in, as shader model 2 wants it.
fn ins(opcode: u32, operands: &[u32]) -> Vec<u32> {
    let mut tokens = vec![opcode | ((operands.len() as u32) << 24)];
    tokens.extend_from_slice(operands);
    tokens
}

fn dcl(usage: u32, index: u32, target: u32) -> Vec<u32> {
    ins(31, &[0x8000_0000 | usage | (index << 16), target])
}

fn dcl_sampler(texture_type: u32, unit: u32) -> Vec<u32> {
    ins(
        31,
        &[0x8000_0000 | (texture_type << 27), dst(SAMPLER, unit, XYZW)],
    )
}

fn shader(version: u32, parts: &[Vec<u32>]) -> Vec<u32> {
    let mut tokens = vec![version];
    tokens.extend(parts.iter().flatten());
    tokens.push(END);
    tokens
}

fn translated(tokens: &[u32]) -> Translated {
    translate(tokens, &Options::default()).expect("the shader translates")
}

fn minimal_vertex_shader() -> Vec<u32> {
    shader(
        VS_2_0,
        &[
            dcl(0, 0, dst(INPUT, 0, XYZW)),
            dcl(5, 0, dst(INPUT, 1, XYZW)),
            ins(20, &[dst(RASTOUT, 0, XYZW), src(INPUT, 0), src(CONST, 0)]),
            ins(1, &[dst(OUTPUT, 0, XYZW), src(INPUT, 1)]),
        ],
    )
}

#[test]
fn minimal_vertex_shader_follows_the_conventions() {
    let result = translated(&minimal_vertex_shader());
    assert_eq!(result.stage, Stage::Vertex);
    assert_eq!((result.major, result.minor), (2, 0));
    assert_eq!(
        result.inputs,
        vec![
            VertexInput {
                register: 0,
                usage: 0,
                usage_index: 0
            },
            VertexInput {
                register: 1,
                usage: 5,
                usage_index: 0
            },
        ]
    );
    // m4x4 reads four rows.
    assert_eq!(result.float_constants, 4);
    assert!(!result.writes_point_size);

    let msl = &result.msl;
    assert!(msl.starts_with("#include <metal_stdlib>"));
    assert!(msl.contains("constant bool fc_alpha_test [[function_constant(0)]];"));
    assert!(msl.contains("vertex VsOut vs_main("));
    assert!(msl.contains("float4 v0 [[attribute(0)]];"));
    assert!(msl.contains("float4 v1 [[attribute(1)]];"));
    assert!(msl.contains("constant float4 *vc [[buffer(16)]]"));
    assert!(msl.contains("constant D3DExtra &ex [[buffer(17)]]"));
    assert!(msl.contains(
        "oPos = float4(dot(v0, vc[0]), dot(v0, vc[1]), dot(v0, vc[2]), dot(v0, vc[3]));"
    ));
    assert!(msl.contains("oT0 = v1;"));
    assert!(msl.contains("out.texcoord0 = oT0;"));

    // The whole fixed set is there, written or not.
    for index in 0..10 {
        assert!(msl.contains(&format!(
            "float4 texcoord{index} [[user(texcoord{index})]];"
        )));
    }
    assert!(msl.contains("float4 color0 [[user(color0)]];"));
    assert!(msl.contains("float4 color1 [[user(color1)]];"));
    assert!(msl.contains("float4 fog [[user(fog)]];"));
    assert!(msl.contains("out.texcoord9 = float4(0.0);"));

    // Clip plane first, then the half-pixel offset.
    let clip = msl
        .find("out.clip0 = dot(d3d_pos, ex.clip_plane0);")
        .expect("clip distance");
    let fixup = msl
        .find("d3d_pos.xy += ex.pos_fixup.xy * d3d_pos.w;")
        .expect("half-pixel offset");
    assert!(clip < fixup);
}

#[test]
fn write_masks_swizzles_and_negation() {
    let tokens = shader(
        VS_2_0,
        &[
            dcl(0, 0, dst(INPUT, 0, XYZW)),
            // mad r0.xz, v0.yyzw, -c1.x, r2
            ins(
                4,
                &[
                    dst(TEMP, 0, 0b0101),
                    swizzled(INPUT, 0, [1, 1, 2, 3]),
                    negated(swizzled(CONST, 1, [0, 0, 0, 0])),
                    src(TEMP, 2),
                ],
            ),
            // dp3_sat r1.yw, r0, c2
            ins(
                8,
                &[saturated(dst(TEMP, 1, 0b1010)), src(TEMP, 0), src(CONST, 2)],
            ),
            // rsq r1.x, r0.w
            ins(7, &[dst(TEMP, 1, 0b0001), swizzled(TEMP, 0, [3, 3, 3, 3])]),
            // crs r3.xy, r0, r1
            ins(33, &[dst(TEMP, 3, 0b0011), src(TEMP, 0), src(TEMP, 1)]),
            // slt r3.z, r0.x, r1.y
            ins(
                12,
                &[
                    dst(TEMP, 3, 0b0100),
                    swizzled(TEMP, 0, [0; 4]),
                    swizzled(TEMP, 1, [1; 4]),
                ],
            ),
        ],
    );
    let result = translated(&tokens);
    let msl = &result.msl;
    assert!(
        msl.contains("r0.xz = v0.yz * (-vc[1].xx) + r2.xz;"),
        "{msl}"
    );
    assert!(
        msl.contains("r1.yw = saturate(float2(dot(r0.xyz, vc[2].xyz)));"),
        "{msl}"
    );
    assert!(msl.contains("r1.x = rsqrt(abs(r0.w));"), "{msl}");
    assert!(msl.contains("r3.xy = cross(r0.xyz, r1.xyz).xy;"), "{msl}");
    assert!(msl.contains("r3.z = float(r0.x < r1.y);"), "{msl}");
    assert!(msl.contains("float4 r3 = float4(0.0);"));
    assert_eq!(result.float_constants, 3);
}

#[test]
fn defined_constants_are_local_and_exact() {
    let values = [
        1.0f32.to_bits(),
        0.1f32.to_bits(),
        (-0.0f32).to_bits(),
        f32::INFINITY.to_bits(),
    ];
    let mut def = vec![dst(CONST, 5, XYZW)];
    def.extend_from_slice(&values);
    let tokens = shader(
        PS_2_0,
        &[
            ins(81, &def),
            ins(48, &[dst(CONSTINT, 2, XYZW), 4, 0, 1, i32::MIN as u32]),
            ins(47, &[dst(CONSTBOOL, 1, XYZW), 1]),
            ins(1, &[dst(TEMP, 0, XYZW), src(CONST, 5)]),
            ins(2, &[dst(COLOROUT, 0, XYZW), src(TEMP, 0), src(CONST, 6)]),
        ],
    );
    let result = translated(&tokens);
    let msl = &result.msl;
    assert!(
        msl.contains(
            "const float4 c5 = float4(1.0, 0.1, as_type<float>(0x80000000u), as_type<float>(0x7f800000u));"
        ),
        "{msl}"
    );
    assert!(
        msl.contains("const int4 i2 = int4(4, 0, 1, (-2147483647 - 1));"),
        "{msl}"
    );
    assert!(msl.contains("const bool b1 = true;"), "{msl}");
    assert!(msl.contains("r0 = c5;"));
    assert!(msl.contains("oC0 = r0 + pc[6];"));
    // c5 never comes from the buffer; c6 does.
    assert_eq!(result.float_constants, 7);
    assert_eq!(result.color_outputs, 1);
}

#[test]
fn float_literals_round_trip() {
    let mut state = 0x1234_5678u32;
    let mut samples = vec![
        0u32,
        0x8000_0000,
        0x7F80_0000,
        0xFF80_0000,
        0x7FC0_0000,
        1,
        0x0080_0000,
        0x7F7F_FFFF,
    ];
    for _ in 0..20_000 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        samples.push(state);
    }
    for bits in samples {
        let mut def = vec![dst(CONST, 0, XYZW)];
        def.extend_from_slice(&[bits; 4]);
        let tokens = shader(
            PS_2_0,
            &[
                ins(81, &def),
                ins(1, &[dst(COLOROUT, 0, XYZW), src(CONST, 0)]),
            ],
        );
        let msl = translated(&tokens).msl;
        let start = msl
            .find("const float4 c0 = float4(")
            .expect("the definition")
            + 25;
        let literal = msl[start..]
            .split([',', ';'])
            .next()
            .expect("a literal")
            .trim();
        let back = match literal.strip_prefix("as_type<float>(0x") {
            Some(hex) => u32::from_str_radix(hex.trim_end_matches("u)"), 16).expect("hex bits"),
            None => {
                let wide: f64 = literal.parse().expect("a decimal literal");
                assert_eq!(
                    literal.parse::<f32>().expect("a decimal literal").to_bits(),
                    bits
                );
                (wide as f32).to_bits()
            }
        };
        assert_eq!(back, bits, "{literal}");
    }
}

#[test]
fn relative_addressing_reads_the_buffer() {
    let tokens = shader(
        VS_2_0,
        &[
            dcl(0, 0, dst(INPUT, 0, XYZW)),
            dcl(2, 0, dst(INPUT, 1, XYZW)),
            // def c10: relative reads must ignore it.
            ins(81, &[dst(CONST, 10, XYZW), 0, 0, 0, 0]),
            // mova a0.xy, v1.xyxx
            ins(
                46,
                &[dst(ADDR, 0, 0b0011), swizzled(INPUT, 1, [0, 1, 0, 0])],
            ),
            // dp4 r0.x, v0, c10[a0.y]
            ins(
                9,
                &[
                    dst(TEMP, 0, 0b0001),
                    src(INPUT, 0),
                    src(CONST, 10) | RELATIVE,
                    swizzled(ADDR, 0, [1, 1, 1, 1]),
                ],
            ),
            // m4x3 oPos.xyz, v0, c20[a0.x]
            ins(
                21,
                &[
                    dst(RASTOUT, 0, 0b0111),
                    src(INPUT, 0),
                    src(CONST, 20) | RELATIVE,
                    swizzled(ADDR, 0, [0, 0, 0, 0]),
                ],
            ),
        ],
    );
    let result = translated(&tokens);
    let msl = &result.msl;
    assert!(msl.contains("int4 a0 = int4(0);"));
    assert!(msl.contains("a0.xy = int2(rint(v1.xy));"), "{msl}");
    assert!(
        msl.contains("r0.x = dot(v0, vc[clamp(a0.y + 10, 0, 255)]);"),
        "{msl}"
    );
    assert!(
        msl.contains("dot(v0, vc[clamp(a0.x + 22, 0, 255)])"),
        "{msl}"
    );
    assert_eq!(result.float_constants, 256);
}

#[test]
fn vs_1_1_has_no_lengths_and_no_relative_token() {
    let tokens = vec![
        VS_1_1,
        31,
        0x8000_0000,
        dst(INPUT, 0, XYZW),
        // mov a0.x, v0.x: floors
        1,
        dst(ADDR, 0, 0b0001),
        swizzled(INPUT, 0, [0; 4]),
        // m4x4 oPos, v0, c4[a0.x]
        20,
        dst(RASTOUT, 0, XYZW),
        src(INPUT, 0),
        src(CONST, 4) | RELATIVE,
        // mov oD0, c1
        1,
        dst(5, 0, XYZW),
        src(CONST, 1),
        END,
    ];
    let result = translated(&tokens);
    let msl = &result.msl;
    assert_eq!((result.major, result.minor), (1, 1));
    assert!(msl.contains("a0.x = int(floor(v0.x));"), "{msl}");
    assert!(
        msl.contains("dot(v0, vc[clamp(a0.x + 7, 0, 255)])"),
        "{msl}"
    );
    assert!(msl.contains("out.color0 = saturate(oD0);"), "{msl}");
}

#[test]
fn if_else_endif_on_a_bool_constant() {
    let tokens = shader(
        VS_3_0,
        &[
            dcl(0, 0, dst(INPUT, 0, XYZW)),
            dcl(0, 0, dst(OUTPUT, 0, XYZW)),
            ins(40, &[src(CONSTBOOL, 3)]),
            ins(1, &[dst(OUTPUT, 0, XYZW), src(INPUT, 0)]),
            ins(42, &[]),
            // ifc_lt v0.x, c0.y
            vec![
                41 | (2 << 24) | (4 << 16),
                swizzled(INPUT, 0, [0; 4]),
                swizzled(CONST, 0, [1; 4]),
            ],
            ins(1, &[dst(OUTPUT, 0, XYZW), negated(src(INPUT, 0))]),
            ins(43, &[]),
            ins(43, &[]),
        ],
    );
    let result = translated(&tokens);
    let msl = &result.msl;
    assert!(result.uses_bool_constants);
    assert!(
        msl.contains("    if ((((ex.bools >> 3u) & 1u) != 0u)) {\n"),
        "{msl}"
    );
    assert!(msl.contains("        o0 = v0;\n"), "{msl}");
    assert!(msl.contains("    } else {\n"), "{msl}");
    assert!(msl.contains("        if (v0.x < vc[0].y) {\n"), "{msl}");
    assert!(msl.contains("            o0 = (-v0);\n"), "{msl}");
    assert!(msl.contains("float4 d3d_pos = o0;"), "{msl}");

    let unclosed = shader(VS_3_0, &[ins(40, &[src(CONSTBOOL, 3)])]);
    assert!(translate(&unclosed, &Options::default()).is_err());
    let stray = shader(VS_3_0, &[ins(43, &[])]);
    assert!(translate(&stray, &Options::default()).is_err());
}

#[test]
fn rep_and_loop() {
    let tokens = shader(
        VS_3_0,
        &[
            dcl(0, 0, dst(INPUT, 0, XYZW)),
            dcl(0, 0, dst(OUTPUT, 0, XYZW)),
            ins(48, &[dst(CONSTINT, 0, XYZW), 3, 0, 1, 0]),
            ins(38, &[src(CONSTINT, 0)]),
            ins(2, &[dst(TEMP, 0, XYZW), src(TEMP, 0), src(CONST, 0)]),
            // breakc_ge r0.x, c1.x
            vec![
                45 | (2 << 24) | (3 << 16),
                swizzled(TEMP, 0, [0; 4]),
                swizzled(CONST, 1, [0; 4]),
            ],
            ins(39, &[]),
            // loop aL, i1 / add r0, r0, c8[aL] / endloop
            ins(27, &[src(LOOP, 0), src(CONSTINT, 1)]),
            ins(
                2,
                &[
                    dst(TEMP, 0, XYZW),
                    src(TEMP, 0),
                    src(CONST, 8) | RELATIVE,
                    src(LOOP, 0),
                ],
            ),
            ins(29, &[]),
            ins(1, &[dst(OUTPUT, 0, XYZW), src(TEMP, 0)]),
        ],
    );
    let result = translated(&tokens);
    let msl = &result.msl;
    assert!(result.uses_int_constants);
    assert!(msl.contains("const int4 i0 = int4(3, 0, 1, 0);"));
    assert!(
        msl.contains("for (int rep0 = 0; rep0 < min(i0.x, 255); ++rep0) {"),
        "{msl}"
    );
    assert!(
        msl.contains("        if (r0.x >= vc[1].x) break;\n"),
        "{msl}"
    );
    assert!(
        msl.contains(
            "for (int lc1 = 0, aL1 = ex.ic[1].y; lc1 < min(ex.ic[1].x, 255); ++lc1, aL1 += ex.ic[1].z) {"
        ),
        "{msl}"
    );
    assert!(
        msl.contains("r0 = r0 + vc[clamp(aL1 + 8, 0, 255)];"),
        "{msl}"
    );

    let outside = shader(VS_3_0, &[ins(44, &[])]);
    assert!(translate(&outside, &Options::default()).is_err());
}

fn texture_lookups() -> Vec<u32> {
    shader(
        PS_2_0,
        &[
            dcl(0, 0, dst(TEXTURE, 0, XYZW)),
            dcl_sampler(2, 0),
            dcl_sampler(3, 1),
            dcl_sampler(4, 2),
            ins(66, &[dst(TEMP, 0, XYZW), src(TEXTURE, 0), src(SAMPLER, 0)]),
            // texldp
            vec![
                66 | (3 << 24) | (1 << 16),
                dst(TEMP, 1, XYZW),
                src(TEXTURE, 0),
                src(SAMPLER, 0),
            ],
            // texldb
            vec![
                66 | (3 << 24) | (2 << 16),
                dst(TEMP, 2, XYZW),
                src(TEXTURE, 0),
                src(SAMPLER, 0),
            ],
            ins(66, &[dst(TEMP, 3, XYZW), src(TEXTURE, 0), src(SAMPLER, 1)]),
            ins(
                66,
                &[
                    dst(TEMP, 4, XYZW),
                    swizzled(TEXTURE, 0, [2, 1, 0, 3]),
                    src(SAMPLER, 2),
                ],
            ),
            ins(1, &[dst(COLOROUT, 0, XYZW), src(TEMP, 0)]),
        ],
    )
}

#[test]
fn texld_texldp_texldb() {
    let result = translated(&texture_lookups());
    let msl = &result.msl;
    assert_eq!(result.samplers[0], SamplerKind::D2);
    assert_eq!(result.samplers[1], SamplerKind::Cube);
    assert_eq!(result.samplers[2], SamplerKind::Volume);
    assert_eq!(result.samplers[3], SamplerKind::None);
    assert!(msl.contains("texture2d<float> tex0 [[texture(0)]]"));
    assert!(msl.contains("sampler smp0 [[sampler(0)]]"));
    assert!(msl.contains("texturecube<float> tex1 [[texture(1)]]"));
    assert!(msl.contains("texture3d<float> tex2 [[texture(2)]]"));
    assert!(msl.contains("r0 = tex0.sample(smp0, t0.xy);"), "{msl}");
    assert!(
        msl.contains("r1 = tex0.sample(smp0, t0.xy / t0.w);"),
        "{msl}"
    );
    assert!(
        msl.contains("r2 = tex0.sample(smp0, t0.xy, bias(t0.w));"),
        "{msl}"
    );
    assert!(msl.contains("r3 = tex1.sample(smp1, t0.xyz);"), "{msl}");
    assert!(msl.contains("r4 = tex2.sample(smp2, t0.zyx);"), "{msl}");
    assert!(msl.contains("float4 texcoord0 [[user(texcoord0)]];"));
    assert!(msl.contains("fragment PsOut ps_main("));
    assert!(msl.contains("constant float4 *pc [[buffer(0)]]"));
    assert!(msl.contains("constant D3DExtra &ex [[buffer(1)]]"));
    assert!(msl.contains("if (fc_alpha_test) {"));
}

#[test]
fn shadow_samplers_compare() {
    let options = Options { shadow_samplers: 1 };
    let result = translate(&texture_lookups(), &options).expect("the shader translates");
    let msl = &result.msl;
    assert!(msl.contains("depth2d<float> tex0 [[texture(0)]]"));
    assert!(
        msl.contains("r0 = float4(tex0.sample_compare(smp0, t0.xy, t0.z));"),
        "{msl}"
    );
    assert!(
        msl.contains("r1 = float4(tex0.sample_compare(smp0, t0.xy / t0.w, t0.z / t0.w));"),
        "{msl}"
    );
    // The cube map in unit 1 is none of the option's business.
    assert!(msl.contains("texturecube<float> tex1 [[texture(1)]]"));
}

#[test]
fn shader_model_3_outputs_share_registers() {
    let tokens = shader(
        VS_3_0,
        &[
            dcl(0, 0, dst(INPUT, 0, XYZW)),
            dcl(0, 0, dst(OUTPUT, 0, XYZW)),
            dcl(5, 4, dst(OUTPUT, 5, 0b0111)),
            dcl(11, 0, dst(OUTPUT, 5, 0b1000)),
            dcl(3, 0, dst(OUTPUT, 6, XYZW)),
            dcl(4, 0, dst(OUTPUT, 7, 0b0001)),
            ins(1, &[dst(OUTPUT, 0, XYZW), src(INPUT, 0)]),
            ins(1, &[dst(OUTPUT, 5, XYZW), src(INPUT, 0)]),
            ins(1, &[dst(OUTPUT, 6, XYZW), src(INPUT, 0)]),
            ins(1, &[dst(OUTPUT, 7, 0b0001), swizzled(CONST, 0, [0; 4])]),
        ],
    );
    let result = translated(&tokens);
    let msl = &result.msl;
    assert!(result.writes_point_size);
    assert!(msl.contains("out.texcoord4.xyz = o5.xyz;"), "{msl}");
    assert!(msl.contains("out.fog.w = o5.w;"), "{msl}");
    assert!(msl.contains("float4 u3_0 [[user(u3_0)]];"), "{msl}");
    assert!(msl.contains("out.u3_0 = o6;"), "{msl}");
    assert!(msl.contains("float psize [[point_size]];"), "{msl}");
    assert!(msl.contains("out.psize = o7.x;"), "{msl}");
    assert!(msl.contains("float4 d3d_pos = o0;"), "{msl}");
    // Shader model 3 does not clamp colours, and the fixed set is still whole.
    assert!(msl.contains("out.color0 = float4(0.0);"));
    assert!(msl.contains("float4 texcoord9 [[user(texcoord9)]];"));
}

#[test]
fn shader_model_3_pixel_inputs() {
    let tokens = shader(
        PS_3_0,
        &[
            dcl(5, 0, dst(INPUT, 0, 0b0011)),
            dcl(5, 1, dst(INPUT, 0, 0b1100)),
            dcl(10, 1, dst(INPUT, 1, XYZW) | (4 << 20)),
            dcl(0, 0, dst(MISC, 0, XYZW)),
            dcl(0, 0, dst(MISC, 1, XYZW)),
            dcl_sampler(2, 0),
            // texldl r0, v0, s0
            ins(95, &[dst(TEMP, 0, XYZW), src(INPUT, 0), src(SAMPLER, 0)]),
            // texldd r1, v0, s0, v1, v1
            ins(
                93,
                &[
                    dst(TEMP, 1, XYZW),
                    src(INPUT, 0),
                    src(SAMPLER, 0),
                    src(INPUT, 1),
                    src(INPUT, 1),
                ],
            ),
            ins(
                4,
                &[dst(TEMP, 0, XYZW), src(TEMP, 0), src(MISC, 1), src(MISC, 0)],
            ),
            // texkill r0
            ins(65, &[dst(TEMP, 0, XYZW)]),
            ins(1, &[dst(COLOROUT, 0, XYZW), src(TEMP, 0)]),
            ins(1, &[dst(COLOROUT, 2, XYZW), src(TEMP, 1)]),
            ins(1, &[dst(9, 0, XYZW), swizzled(TEMP, 0, [2; 4])]),
        ],
    );
    let result = translated(&tokens);
    let msl = &result.msl;
    assert_eq!(result.color_outputs, 0b0101);
    assert!(result.writes_depth);
    assert!(msl.contains("float4 position [[position]];"), "{msl}");
    assert!(
        msl.contains("float4 texcoord0 [[user(texcoord0)]];"),
        "{msl}"
    );
    assert!(
        msl.contains("float4 texcoord1 [[user(texcoord1)]];"),
        "{msl}"
    );
    assert!(
        msl.contains("float4 color1 [[user(color1), centroid_perspective]];"),
        "{msl}"
    );
    assert!(msl.contains("v0.xy = in.texcoord0.xy;"), "{msl}");
    assert!(msl.contains("v0.zw = in.texcoord1.zw;"), "{msl}");
    assert!(msl.contains("bool d3d_front [[front_facing]]"), "{msl}");
    assert!(
        msl.contains("r0 = tex0.sample(smp0, v0.xy, level(v0.w));"),
        "{msl}"
    );
    assert!(
        msl.contains("r1 = tex0.sample(smp0, v0.xy, gradient2d(v1.xy, v1.xy));"),
        "{msl}"
    );
    assert!(
        msl.contains("if (any(r0 < 0.0)) discard_fragment();"),
        "{msl}"
    );
    assert!(msl.contains("float4 color2 [[color(2)]];"), "{msl}");
    assert!(msl.contains("float depth [[depth(any)]];"), "{msl}");
    assert!(msl.contains("out.depth = oDepth.x;"), "{msl}");
}

#[test]
fn what_is_not_supported_says_so() {
    let failure = |tokens: &[u32]| {
        translate(tokens, &Options::default())
            .expect_err("an error")
            .message
    };
    assert_eq!(failure(&[0xFFFF_0104, END]), "ps_1_x not supported");
    assert!(failure(&[0x1234_5678, END]).contains("not a shader version token"));
    assert!(failure(&[0xFFFE_0400, END]).contains("shader model 4"));
    assert!(failure(&[]).contains("empty"));
    // call l0
    assert!(failure(&shader(VS_3_0, &[ins(25, &[src(18, 0)])])).contains("subroutines"));
    // ps_1_x's _bias source modifier
    let biased = src(TEMP, 0) | (2 << 24);
    assert!(failure(&shader(
        PS_2_0,
        &[ins(1, &[dst(COLOROUT, 0, XYZW), biased])]
    ))
    .contains("source modifier"));
    // An input nobody declared.
    assert!(failure(&shader(
        VS_2_0,
        &[ins(1, &[dst(RASTOUT, 0, XYZW), src(INPUT, 3)])]
    ))
    .contains("never declared"));
    // An instruction whose length disagrees with its operands.
    let short = vec![2 | (2 << 24), dst(TEMP, 0, XYZW), src(TEMP, 0)];
    assert!(failure(&shader(VS_2_0, &[short])).contains("source operands"));
}

#[test]
fn comments_are_skipped() {
    let mut tokens = vec![VS_2_0, 0xFFFE | (3 << 16), 0x4241_5443, 0xDEAD_BEEF, 0xFFFF];
    tokens.extend(minimal_vertex_shader().into_iter().skip(1));
    assert!(translate(&tokens, &Options::default()).is_ok());

    let overlong = vec![VS_2_0, 0xFFFE | (300 << 16), 0, END];
    assert!(translate(&overlong, &Options::default()).is_err());
}

#[test]
fn malformed_streams_are_errors_not_panics() {
    let samples = [minimal_vertex_shader(), texture_lookups()];
    for whole in &samples {
        // Every truncation, which also loses the end token.
        for length in 0..whole.len() {
            assert!(
                translate(&whole[..length], &Options::default()).is_err(),
                "prefix of {length} tokens"
            );
        }
        // Every token replaced by every awkward value, one at a time.
        for at in 0..whole.len() {
            for replacement in [
                0,
                1,
                0xFFFF_FFFF,
                0x8000_0000,
                0x7FFF_FFFF,
                0xFFFF,
                0xFFFE,
                0x0F00_0000 | 46,
            ] {
                let mut corrupt = whole.clone();
                corrupt[at] = replacement;
                let _ = translate(&corrupt, &Options::default());
            }
            for bit in 0..32 {
                let mut corrupt = whole.clone();
                corrupt[at] ^= 1 << bit;
                let _ = translate(&corrupt, &Options::default());
            }
        }
    }
    // And noise after a plausible version token.
    let mut state = 0x9E37_79B9u32;
    for round in 0..2_000 {
        let version = [VS_1_1, VS_2_0, VS_3_0, PS_2_0, PS_3_0][round % 5];
        let mut tokens = vec![version];
        for _ in 0..(round % 40) {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            // Keep a good share of the tokens looking like instructions.
            tokens.push(if state & 1 == 0 {
                state
            } else {
                state & 0x0F00_007F
            });
        }
        tokens.push(END);
        let _ = translate(&tokens, &Options::default());
    }
}

#[test]
fn words_from_bytes_is_little_endian() {
    assert_eq!(
        words_from_bytes(&[0x00, 0x02, 0xFE, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0xAA]),
        vec![VS_2_0, END]
    );
    assert!(words_from_bytes(&[1, 2, 3]).is_empty());
}
