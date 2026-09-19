//! Every shader the game really creates, through the translator and then
//! through a Metal compiler.
//!
//! The corpus is bytecode dumped from the running game, one file per shader:
//! `<name>_<hash>.vso` and `.pso`, the raw little-endian token stream. It is
//! not in the repository, so the tests here look for it where
//! `SOURCE_SHADER_CORPUS` points and pass with a note when that is unset.
//!
//! ```text
//! SOURCE_SHADER_CORPUS=/path/to/shaders cargo test -p source-d3d9 --test translate_corpus -- --nocapture
//! ```
//!
//! `SOURCE_SHADER_OUT=/some/dir` keeps the generated `.metal` files there.
//!
//! The Metal compiler is `xcrun -sdk macosx metal` when the Metal toolchain
//! is installed (`xcodebuild -downloadComponent MetalToolchain`). When it is
//! not, the sources go through `MTLDevice.makeLibrary(source:)` instead, by
//! way of a few lines of Swift built on the spot: that is the compiler the
//! device uses at run time, and it needs nothing installed beyond Xcode's
//! command line tools.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

use source_d3d9::translate::{
    translate, words_from_bytes, Options, SamplerKind, Stage, Translated,
};

struct Shader {
    name: String,
    tokens: Vec<u32>,
}

fn corpus() -> Option<Vec<Shader>> {
    let Some(directory) = std::env::var_os("SOURCE_SHADER_CORPUS") else {
        println!("SOURCE_SHADER_CORPUS is not set: skipping the shader corpus");
        return None;
    };
    let directory = PathBuf::from(directory);
    let Ok(entries) = fs::read_dir(&directory) else {
        println!(
            "{} cannot be read: skipping the shader corpus",
            directory.display()
        );
        return None;
    };
    let mut shaders = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let extension = path.extension().and_then(|extension| extension.to_str());
        if !matches!(extension, Some("vso" | "pso")) {
            continue;
        }
        let name = path
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        let bytes = fs::read(&path).expect("a readable shader file");
        shaders.push(Shader {
            name,
            tokens: words_from_bytes(&bytes),
        });
    }
    shaders.sort_by(|left, right| left.name.cmp(&right.name));
    Some(shaders)
}

fn version_name(tokens: &[u32]) -> String {
    match tokens.first() {
        Some(&version) => {
            let stage = match version >> 16 {
                0xFFFE => "vs",
                0xFFFF => "ps",
                _ => return format!("{version:#010x}"),
            };
            format!("{stage}_{}_{}", (version >> 8) & 0xFF, version & 0xFF)
        }
        None => "empty".to_string(),
    }
}

// ------------------------------------------------------------------ compiling

const RUNTIME_COMPILER: &str = r#"
import Foundation
import Metal

// compile <list file> <worker> <workers>: compiles every line of the list
// whose index is this worker's share. One record per file on stdout: a line
// "@@ OK <path>" or "@@ FAIL <path>", the latter followed by the message.
guard let device = MTLCreateSystemDefaultDevice() ?? MTLCopyAllDevices().first else {
    FileHandle.standardError.write("no Metal device\n".data(using: .utf8)!)
    exit(2)
}
let arguments = CommandLine.arguments
let worker = Int(arguments[2])!
let workers = Int(arguments[3])!
let list = try! String(contentsOfFile: arguments[1], encoding: .utf8)
let options = MTLCompileOptions()
options.languageVersion = .version3_0
for (index, path) in list.split(separator: "\n").enumerated() where index % workers == worker {
    do {
        let source = try String(contentsOfFile: String(path), encoding: .utf8)
        _ = try device.makeLibrary(source: source, options: options)
        print("@@ OK \(path)")
    } catch {
        print("@@ FAIL \(path)\n\(error.localizedDescription)")
    }
}
"#;

enum Compiler {
    /// `xcrun -sdk macosx metal`
    Offline,
    /// The Swift helper above, built into the output directory.
    Runtime(PathBuf),
}

fn offline_compile(source: &Path) -> Result<(), String> {
    let output = Command::new("xcrun")
        .args(["-sdk", "macosx", "metal", "-std=metal3.0", "-c"])
        .arg(source)
        .args(["-o", "/dev/null"])
        .output()
        .map_err(|error| format!("xcrun did not run: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn find_compiler(directory: &Path) -> Compiler {
    let probe = directory.join("probe.metal");
    fs::write(&probe, "#include <metal_stdlib>\nusing namespace metal;\n")
        .expect("a writable directory");
    let offline = offline_compile(&probe);
    let _ = fs::remove_file(&probe);
    let Err(offline) = offline else {
        return Compiler::Offline;
    };

    let source = directory.join("runtime_compiler.swift");
    let binary = directory.join("runtime_compiler");
    fs::write(&source, RUNTIME_COMPILER).expect("a writable directory");
    let built = Command::new("swiftc")
        .arg("-O")
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output();
    match built {
        Ok(output) if output.status.success() => Compiler::Runtime(binary),
        Ok(output) => panic!(
            "no Metal compiler: the offline one says\n{offline}\nand the runtime helper does not build:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => panic!(
            "no Metal compiler: the offline one says\n{offline}\nand swiftc did not run: {error}"
        ),
    }
}

/// Compiles every file, in parallel, and returns the failures: (path, message).
fn compile_all(compiler: &Compiler, directory: &Path, files: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let workers = thread::available_parallelism()
        .map_or(4, usize::from)
        .min(files.len().max(1));
    let failures = Mutex::new(Vec::new());
    match compiler {
        Compiler::Offline => {
            let next = AtomicUsize::new(0);
            thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| loop {
                        let Some(file) = files.get(next.fetch_add(1, Ordering::Relaxed)) else {
                            break;
                        };
                        if let Err(message) = offline_compile(file) {
                            failures
                                .lock()
                                .expect("the lock")
                                .push((file.clone(), message));
                        }
                    });
                }
            });
        }
        Compiler::Runtime(binary) => {
            let list = directory.join("sources.txt");
            let lines: Vec<String> = files
                .iter()
                .map(|file| file.display().to_string())
                .collect();
            fs::write(&list, lines.join("\n")).expect("a writable directory");
            thread::scope(|scope| {
                for worker in 0..workers {
                    let (list, failures) = (&list, &failures);
                    scope.spawn(move || {
                        let output = Command::new(binary)
                            .arg(list)
                            .arg(worker.to_string())
                            .arg(workers.to_string())
                            .output()
                            .expect("the runtime compiler runs");
                        assert!(
                            output.status.success(),
                            "the runtime compiler failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let mut records = 0;
                        for record in stdout.split("@@ ").skip(1) {
                            records += 1;
                            if let Some(failure) = record.strip_prefix("FAIL ") {
                                let (path, message) =
                                    failure.split_once('\n').unwrap_or((failure, ""));
                                failures
                                    .lock()
                                    .expect("the lock")
                                    .push((PathBuf::from(path), message.to_string()));
                            }
                        }
                        let share = (0..files.len())
                            .filter(|index| index % workers == worker)
                            .count();
                        assert_eq!(
                            records, share,
                            "the runtime compiler lost track of its files"
                        );
                    });
                }
            });
            let _ = fs::remove_file(list);
        }
    }
    let mut failures = failures.into_inner().expect("the lock");
    failures.sort();
    failures
}

// ------------------------------------------------- shaders the game has none of

mod assemble {
    //! Just enough of an assembler for the forms the corpus lacks: shader
    //! model 3, flow control, vs_1_1.

    pub const END: u32 = 0x0000_FFFF;
    pub const TEMP: u32 = 0;
    pub const INPUT: u32 = 1;
    pub const CONST: u32 = 2;
    pub const ADDR: u32 = 3;
    pub const TEXTURE: u32 = 3;
    pub const RASTOUT: u32 = 4;
    pub const ATTROUT: u32 = 5;
    pub const OUTPUT: u32 = 6;
    pub const CONSTINT: u32 = 7;
    pub const COLOROUT: u32 = 8;
    pub const DEPTHOUT: u32 = 9;
    pub const SAMPLER: u32 = 10;
    pub const CONSTBOOL: u32 = 14;
    pub const LOOP: u32 = 15;
    pub const MISC: u32 = 17;
    pub const PREDICATE: u32 = 19;
    pub const XYZW: u32 = 0xF;
    pub const RELATIVE: u32 = 0x2000;

    pub fn register(rtype: u32, num: u32) -> u32 {
        0x8000_0000 | ((rtype & 0x7) << 28) | ((rtype & 0x18) << 8) | num
    }

    pub fn dst(rtype: u32, num: u32, mask: u32) -> u32 {
        register(rtype, num) | (mask << 16)
    }

    pub fn swizzled(rtype: u32, num: u32, swizzle: [u32; 4]) -> u32 {
        let bits = swizzle[0] | (swizzle[1] << 2) | (swizzle[2] << 4) | (swizzle[3] << 6);
        register(rtype, num) | (bits << 16)
    }

    pub fn src(rtype: u32, num: u32) -> u32 {
        swizzled(rtype, num, [0, 1, 2, 3])
    }

    pub fn modified(token: u32, modifier: u32) -> u32 {
        token | (modifier << 24)
    }

    pub fn ins(opcode: u32, operands: &[u32]) -> Vec<u32> {
        controlled(opcode, 0, operands)
    }

    pub fn controlled(opcode: u32, control: u32, operands: &[u32]) -> Vec<u32> {
        let mut tokens = vec![opcode | (control << 16) | ((operands.len() as u32) << 24)];
        tokens.extend_from_slice(operands);
        tokens
    }

    pub fn dcl(usage: u32, index: u32, target: u32) -> Vec<u32> {
        ins(31, &[0x8000_0000 | usage | (index << 16), target])
    }

    pub fn dcl_sampler(texture_type: u32, unit: u32) -> Vec<u32> {
        ins(
            31,
            &[0x8000_0000 | (texture_type << 27), dst(SAMPLER, unit, XYZW)],
        )
    }

    pub fn shader(version: u32, parts: &[Vec<u32>]) -> Vec<u32> {
        let mut tokens = vec![version];
        tokens.extend(parts.iter().flatten());
        tokens.push(END);
        tokens
    }
}

/// Shaders in the forms the corpus does not contain, so that those reach
/// the Metal compiler too: (name, tokens, options).
fn synthetic() -> Vec<(String, Vec<u32>, Options)> {
    use assemble::*;
    let x = [0u32; 4];
    let y = [1u32; 4];
    let w = [3u32; 4];

    let vs_3_0 = shader(
        0xFFFE_0300,
        &[
            dcl(0, 0, dst(INPUT, 0, XYZW)),
            dcl(3, 0, dst(INPUT, 1, XYZW)),
            dcl(2, 0, dst(INPUT, 2, XYZW)),
            dcl(0, 0, dst(OUTPUT, 0, XYZW)),
            dcl(5, 4, dst(OUTPUT, 1, 0b0111)),
            dcl(11, 0, dst(OUTPUT, 1, 0b1000)),
            dcl(3, 0, dst(OUTPUT, 2, XYZW)),
            dcl(4, 0, dst(OUTPUT, 3, 0b0001)),
            dcl(10, 0, dst(OUTPUT, 4, XYZW)),
            dcl(5, 12, dst(OUTPUT, 5, 0b0011)),
            dcl_sampler(2, 0),
            ins(48, &[dst(CONSTINT, 0, XYZW), 4, 1, 2, 0]),
            ins(47, &[dst(CONSTBOOL, 0, XYZW), 1]),
            ins(
                81,
                &[
                    dst(CONST, 90, XYZW),
                    0x3F80_0000,
                    0xBF00_0000,
                    0x7F80_0000,
                    0x8000_0000,
                ],
            ),
            ins(46, &[dst(ADDR, 0, XYZW), src(INPUT, 2)]),
            ins(20, &[dst(TEMP, 0, XYZW), src(INPUT, 0), src(CONST, 0)]),
            ins(
                21,
                &[
                    dst(TEMP, 1, 0b0111),
                    src(INPUT, 0),
                    src(CONST, 58) | RELATIVE,
                    swizzled(ADDR, 0, y),
                ],
            ),
            ins(22, &[dst(TEMP, 2, XYZW), src(INPUT, 1), src(CONST, 4)]),
            ins(23, &[dst(TEMP, 3, 0b0101), src(INPUT, 1), src(CONST, 8)]),
            ins(24, &[dst(TEMP, 3, 0b0011), src(INPUT, 1), src(CONST, 8)]),
            ins(40, &[src(CONSTBOOL, 0)]),
            ins(16, &[dst(TEMP, 4, XYZW), src(TEMP, 1)]),
            ins(17, &[dst(TEMP, 4, 0b0110), src(TEMP, 1), src(TEMP, 2)]),
            ins(42, &[]),
            controlled(41, 5, &[swizzled(TEMP, 0, x), swizzled(CONST, 90, y)]),
            ins(
                34,
                &[dst(TEMP, 4, XYZW), src(TEMP, 1), src(TEMP, 5), src(TEMP, 6)],
            ),
            ins(37, &[dst(TEMP, 4, 0b0011), swizzled(TEMP, 0, w)]),
            ins(43, &[]),
            ins(43, &[]),
            ins(38, &[src(CONSTINT, 0)]),
            ins(27, &[src(LOOP, 0), src(CONSTINT, 1)]),
            ins(
                2,
                &[
                    dst(TEMP, 4, XYZW),
                    src(TEMP, 4),
                    src(CONST, 20) | RELATIVE,
                    src(LOOP, 0),
                ],
            ),
            controlled(45, 1, &[swizzled(TEMP, 4, x), swizzled(CONST, 90, x)]),
            ins(29, &[]),
            ins(40, &[modified(src(CONSTBOOL, 5), 0)]),
            ins(44, &[]),
            ins(43, &[]),
            ins(39, &[]),
            controlled(
                94,
                4,
                &[dst(PREDICATE, 0, XYZW), src(TEMP, 4), src(CONST, 90)],
            ),
            // (p0) mov r4.xz, c90 / (!p0.y) add r4.w, r4.w, c90.x
            vec![
                1 | (3 << 24) | 0x1000_0000,
                dst(TEMP, 4, 0b0101),
                src(PREDICATE, 0),
                src(CONST, 90),
            ],
            vec![
                2 | (4 << 24) | 0x1000_0000,
                dst(TEMP, 4, 0b1000),
                modified(swizzled(PREDICATE, 0, y), 13),
                swizzled(TEMP, 4, w),
                swizzled(CONST, 90, x),
            ],
            ins(36, &[dst(TEMP, 5, 0b0011), src(TEMP, 4)]),
            ins(36, &[dst(TEMP, 5, 0b0111), modified(src(TEMP, 4), 11)]),
            ins(
                33,
                &[
                    dst(TEMP, 6, 0b0110),
                    src(TEMP, 5),
                    modified(src(TEMP, 4), 12),
                ],
            ),
            ins(95, &[dst(TEMP, 7, XYZW), src(TEMP, 0), src(SAMPLER, 0)]),
            ins(1, &[dst(OUTPUT, 0, XYZW), src(TEMP, 0)]),
            ins(1, &[dst(OUTPUT, 1, XYZW), src(TEMP, 7)]),
            ins(1, &[dst(OUTPUT, 2, XYZW), src(TEMP, 6)]),
            ins(1, &[dst(OUTPUT, 3, 0b0001), swizzled(TEMP, 3, x)]),
            ins(1, &[dst(OUTPUT, 4, XYZW), src(TEMP, 5)]),
            ins(1, &[dst(OUTPUT, 5, 0b0011), src(TEMP, 3)]),
        ],
    );

    let ps_3_0 = shader(
        0xFFFF_0300,
        &[
            dcl(5, 0, dst(INPUT, 0, 0b0011)),
            dcl(5, 1, dst(INPUT, 0, 0b1100)),
            dcl(10, 0, dst(INPUT, 1, XYZW) | (4 << 20)),
            dcl(3, 0, dst(INPUT, 2, 0b0111)),
            dcl(5, 12, dst(INPUT, 3, 0b0011)),
            dcl(11, 0, dst(INPUT, 4, 0b1000)),
            dcl(0, 0, dst(MISC, 0, XYZW)),
            dcl(0, 0, dst(MISC, 1, XYZW)),
            dcl_sampler(2, 0),
            dcl_sampler(3, 1),
            dcl_sampler(4, 2),
            dcl_sampler(2, 3),
            ins(48, &[dst(CONSTINT, 0, XYZW), 4, 0, 1, 0]),
            ins(
                66,
                &[
                    dst(TEMP, 0, XYZW),
                    src(INPUT, 0),
                    swizzled(SAMPLER, 0, [2, 1, 0, 3]),
                ],
            ),
            controlled(66, 1, &[dst(TEMP, 1, XYZW), src(INPUT, 2), src(SAMPLER, 1)]),
            controlled(66, 2, &[dst(TEMP, 2, XYZW), src(INPUT, 2), src(SAMPLER, 2)]),
            controlled(66, 1, &[dst(TEMP, 3, XYZW), src(INPUT, 0), src(SAMPLER, 3)]),
            ins(66, &[dst(TEMP, 3, 0b0001), src(INPUT, 0), src(SAMPLER, 3)]),
            controlled(
                66,
                2,
                &[dst(TEMP, 3, 0b0010), src(INPUT, 0), src(SAMPLER, 3)],
            ),
            ins(95, &[dst(TEMP, 3, 0b0100), src(INPUT, 0), src(SAMPLER, 3)]),
            ins(
                93,
                &[
                    dst(TEMP, 3, 0b1000),
                    src(INPUT, 0),
                    src(SAMPLER, 3),
                    src(INPUT, 1),
                    src(INPUT, 1),
                ],
            ),
            ins(95, &[dst(TEMP, 4, XYZW), src(INPUT, 2), src(SAMPLER, 1)]),
            ins(
                93,
                &[
                    dst(TEMP, 5, 0b0111),
                    src(INPUT, 0),
                    src(SAMPLER, 0),
                    src(INPUT, 1),
                    src(INPUT, 3),
                ],
            ),
            ins(
                93,
                &[
                    dst(TEMP, 5, XYZW),
                    src(INPUT, 2),
                    src(SAMPLER, 1),
                    src(INPUT, 1),
                    src(INPUT, 1),
                ],
            ),
            ins(
                93,
                &[
                    dst(TEMP, 5, XYZW),
                    src(INPUT, 2),
                    src(SAMPLER, 2),
                    src(INPUT, 1),
                    src(INPUT, 1),
                ],
            ),
            ins(91, &[dst(TEMP, 6, XYZW), src(TEMP, 0)]),
            ins(92, &[dst(TEMP, 6, 0b0011), src(TEMP, 1)]),
            ins(
                88,
                &[
                    dst(TEMP, 7, 0b1011),
                    src(TEMP, 6),
                    src(TEMP, 0),
                    src(TEMP, 1),
                ],
            ),
            ins(
                80,
                &[
                    dst(TEMP, 7, 0b0100),
                    src(TEMP, 6),
                    src(TEMP, 0),
                    src(TEMP, 1),
                ],
            ),
            ins(
                90,
                &[
                    dst(TEMP, 8, 0b0001),
                    src(TEMP, 7),
                    src(TEMP, 6),
                    swizzled(TEMP, 5, w),
                ],
            ),
            ins(38, &[src(CONSTINT, 0)]),
            ins(
                4,
                &[dst(TEMP, 8, XYZW), src(TEMP, 8), src(MISC, 1), src(MISC, 0)],
            ),
            ins(39, &[]),
            ins(
                2,
                &[dst(TEMP, 8, XYZW), src(TEMP, 8), swizzled(INPUT, 4, w)],
            ),
            ins(65, &[dst(TEMP, 8, 0b0111)]),
            ins(65, &[dst(TEMP, 8, 0b1000)]),
            ins(1, &[dst(COLOROUT, 0, XYZW), src(TEMP, 8)]),
            ins(1, &[dst(COLOROUT, 1, XYZW), src(TEMP, 3)]),
            ins(1, &[dst(COLOROUT, 3, XYZW), src(TEMP, 4)]),
            ins(1, &[dst(DEPTHOUT, 0, XYZW), swizzled(TEMP, 2, x)]),
        ],
    );

    let vs_1_1 = vec![
        0xFFFE_0101,
        31,
        0x8000_0000,
        dst(INPUT, 0, XYZW),
        31,
        0x8000_0002,
        dst(INPUT, 1, XYZW),
        1,
        dst(ADDR, 0, 0b0001),
        swizzled(INPUT, 1, x),
        20,
        dst(RASTOUT, 0, XYZW),
        src(INPUT, 0),
        src(CONST, 4) | RELATIVE,
        1,
        dst(ATTROUT, 1, XYZW),
        src(CONST, 1),
        1,
        dst(RASTOUT, 1, 0b0001),
        swizzled(CONST, 2, x),
        1,
        dst(RASTOUT, 2, 0b0001),
        swizzled(CONST, 2, y),
        1,
        dst(OUTPUT, 7, 0b0011),
        src(INPUT, 0),
        END,
    ];

    let writes_nothing = shader(0xFFFF_0200, &[dcl(0, 0, dst(TEXTURE, 0, XYZW))]);
    let second_target_only = shader(
        0xFFFF_0200,
        &[
            dcl(0, 0, dst(INPUT, 1, XYZW)),
            ins(1, &[dst(COLOROUT, 1, XYZW), src(INPUT, 1)]),
        ],
    );

    vec![
        ("synthetic_vs_3_0".to_string(), vs_3_0, Options::default()),
        (
            "synthetic_ps_3_0".to_string(),
            ps_3_0.clone(),
            Options::default(),
        ),
        (
            "synthetic_ps_3_0_shadow".to_string(),
            ps_3_0,
            Options {
                shadow_samplers: 1 << 3,
            },
        ),
        ("synthetic_vs_1_1".to_string(), vs_1_1, Options::default()),
        (
            "synthetic_ps_writes_nothing".to_string(),
            writes_nothing,
            Options::default(),
        ),
        (
            "synthetic_ps_second_target_only".to_string(),
            second_target_only,
            Options::default(),
        ),
    ]
}

// ----------------------------------------------------------------- the tests

#[test]
fn corpus_translates_and_compiles() {
    let Some(shaders) = corpus() else {
        return;
    };

    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let mut translate_failures = Vec::new();
    // Identical source is compiled once: source -> the shaders that produced it.
    let mut sources: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut translations = 0;
    for shader in &shaders {
        *versions.entry(version_name(&shader.tokens)).or_default() += 1;
        match translate(&shader.tokens, &Options::default()) {
            Ok(translated) => {
                translations += 1;
                // Whatever reads a 2D texture must also compile as a shadow lookup.
                let flat = (0..16)
                    .filter(|&unit| translated.samplers[unit] == SamplerKind::D2)
                    .fold(0u32, |mask, unit| mask | (1 << unit));
                if translated.stage == Stage::Pixel
                    && flat != 0
                    && shader.name.contains("flashlight")
                {
                    let shadowed = translate(
                        &shader.tokens,
                        &Options {
                            shadow_samplers: flat,
                        },
                    )
                    .expect("options do not decide whether a shader translates");
                    sources
                        .entry(shadowed.msl)
                        .or_default()
                        .push(format!("{}.shadow", shader.name));
                }
                sources
                    .entry(translated.msl)
                    .or_default()
                    .push(shader.name.clone());
            }
            Err(error) => translate_failures.push(format!("{}: {error}", shader.name)),
        }
    }
    for (name, tokens, options) in synthetic() {
        match translate(&tokens, &options) {
            Ok(translated) => sources.entry(translated.msl).or_default().push(name),
            Err(error) => translate_failures.push(format!("{name}: {error}")),
        }
    }

    let (directory, keep) = match std::env::var_os("SOURCE_SHADER_OUT") {
        Some(directory) => (PathBuf::from(directory), true),
        None => (
            std::env::temp_dir().join(format!("source-d3d9-corpus-{}", std::process::id())),
            false,
        ),
    };
    fs::create_dir_all(&directory).expect("an output directory");
    let mut files = Vec::new();
    let mut names: BTreeMap<PathBuf, &Vec<String>> = BTreeMap::new();
    for (source, producers) in &sources {
        let file = directory.join(format!("{}.metal", producers[0]));
        fs::write(&file, source).expect("a writable output directory");
        names.insert(file.clone(), producers);
        files.push(file);
    }

    let compiler = find_compiler(&directory);
    let compiler_name = match compiler {
        Compiler::Offline => "xcrun metal",
        Compiler::Runtime(_) => "MTLDevice.makeLibrary (the Metal toolchain is not installed)",
    };
    let compile_failures = compile_all(&compiler, &directory, &files);

    println!("shader corpus: {} files", shaders.len());
    for (version, count) in &versions {
        println!("  {version}: {count}");
    }
    println!(
        "translated: {translations} of {}, {} failed",
        shaders.len(),
        translate_failures.len()
    );
    for failure in &translate_failures {
        println!("  {failure}");
    }
    println!(
        "compiled with {compiler_name}: {} distinct sources, {} failed",
        files.len(),
        compile_failures.len()
    );
    for (file, message) in compile_failures.iter().take(20) {
        let producers = names.get(file).map_or(0, |producers| producers.len());
        println!("--- {} ({producers} shaders)\n{message}", file.display());
    }
    if keep {
        println!("sources kept in {}", directory.display());
    } else if compile_failures.is_empty() {
        let _ = fs::remove_dir_all(&directory);
    } else {
        println!("sources left in {}", directory.display());
    }

    assert!(
        translate_failures.is_empty(),
        "{} shaders do not translate",
        translate_failures.len()
    );
    assert!(
        compile_failures.is_empty(),
        "{} sources do not compile",
        compile_failures.len()
    );
}

/// The names inside every `[[user(...)]]` of the source.
fn interpolators(translated: &Translated) -> BTreeSet<String> {
    translated
        .msl
        .split("[[user(")
        .skip(1)
        .filter_map(|rest| rest.split_once(')').map(|(name, _)| name.to_string()))
        .collect()
}

/// `<base>_vs20_<hash>.vso` -> `<base>`.
fn base_name(name: &str) -> &str {
    let stage = name.rfind("_vs").max(name.rfind("_ps"));
    stage.map_or(name, |at| &name[..at])
}

/// Metal refuses a pipeline whose fragment function reads an interpolator
/// the vertex function does not write, so every one a pixel shader declares
/// has to exist in the vertex shaders it can be drawn with.
#[test]
fn corpus_interpolators_link() {
    let Some(shaders) = corpus() else {
        return;
    };
    let mut vertex = Vec::new();
    let mut pixel = Vec::new();
    for shader in &shaders {
        let Ok(translated) = translate(&shader.tokens, &Options::default()) else {
            continue;
        };
        let entry = (shader.name.as_str(), interpolators(&translated));
        match translated.stage {
            Stage::Vertex => vertex.push(entry),
            Stage::Pixel => pixel.push(entry),
        }
    }
    if vertex.is_empty() || pixel.is_empty() {
        println!("the corpus lacks one of the stages: nothing to link");
        return;
    }

    let mut pairs = 0;
    let mut related_pairs = 0;
    let mut broken = Vec::new();
    let mut exotic: BTreeSet<&str> = BTreeSet::new();
    for (index, (pixel_name, reads)) in pixel.iter().enumerate() {
        exotic.extend(
            reads
                .iter()
                .map(String::as_str)
                .filter(|name| name.starts_with('u')),
        );
        // The vertex shaders of the same material shader, and a few others
        // so that every pixel shader is paired with something.
        let related: Vec<usize> = (0..vertex.len())
            .filter(|&at| base_name(vertex[at].0) == base_name(pixel_name))
            .take(3)
            .collect();
        related_pairs += related.len();
        let others = (0..3).map(|step| (index * 7 + step * 131) % vertex.len());
        for at in related
            .into_iter()
            .chain(others)
            .collect::<BTreeSet<usize>>()
        {
            let (vertex_name, writes) = &vertex[at];
            pairs += 1;
            for missing in reads.difference(writes) {
                broken.push(format!(
                    "{pixel_name} reads {missing}, which {vertex_name} does not write"
                ));
            }
        }
    }

    println!(
        "linked {pairs} pairs ({related_pairs} of the same shader) from {} vertex and {} pixel shaders",
        vertex.len(),
        pixel.len()
    );
    println!("interpolators outside colour, texcoord and fog: {exotic:?}");
    for line in broken.iter().take(20) {
        println!("  {line}");
    }
    assert!(
        pairs >= 200 || pixel.len() * 3 < 200,
        "only {pairs} pairs were checked"
    );
    assert!(
        broken.is_empty(),
        "{} interpolators would not link",
        broken.len()
    );
}
