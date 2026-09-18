use source_audio::Wave;
use source_bsp::{Bsp, World};
use source_demo::{CommandData, Demo, ParseOptions as DemoParseOptions};
use source_dmx::Document as DmxDocument;
use source_keyvalues::{parse_bytes as parse_keyvalues, ParseOptions as KeyValuesParseOptions};
use source_net::{MessageStream, ParseOptions as NetParseOptions};
use source_save::{MapState, SaveContainer};
use source_scene::SceneImage;
use source_studio::{Mdl, Phy, Vtx, Vvd};
use source_vpk::{crc32, Archive, Entry, VPK_EMBEDDED_ARCHIVE};
use source_vtf::Texture;
use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_REPORTED_ERRORS: usize = 50;

#[derive(Default)]
struct Report {
    vpks: usize,
    loose_vmt: usize,
    loose_vtf: usize,
    packed_vmt: usize,
    packed_vtf: usize,
    packed_mdl: usize,
    packed_vvd: usize,
    packed_vtx: usize,
    packed_phy: usize,
    loose_wav: usize,
    packed_wav: usize,
    loose_dem: usize,
    packed_dem: usize,
    demo_packets: usize,
    demo_messages: usize,
    loose_pcf: usize,
    packed_pcf: usize,
    loose_sav: usize,
    loose_hl1: usize,
    embedded_files: usize,
    embedded_hl1: usize,
    packed_scene_images: usize,
    loose_bsp: usize,
    errors: Vec<String>,
    total_errors: usize,
    known_expected: HashSet<String>,
    known_seen: HashSet<String>,
}

impl Report {
    fn error(&mut self, message: String) {
        self.total_errors += 1;
        if self.errors.len() < MAX_REPORTED_ERRORS {
            self.errors.push(message);
        }
    }

    fn asset_error(&mut self, key: &str, message: String) {
        if self.known_expected.contains(key) {
            self.known_seen.insert(key.to_owned());
        } else {
            self.error(message);
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("source-content-audit: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().collect();
    let (root, known_expected, audit_archives) = match args.as_slice() {
        [_, root] => (Path::new(root), HashSet::new(), true),
        [_, flag, root] if flag == "--loose-only" => (Path::new(root), HashSet::new(), false),
        [_, flag, manifest, root] if flag == "--known-defects" => (
            Path::new(root),
            load_known_defects(Path::new(manifest))?,
            true,
        ),
        _ => {
            return Err(
                "usage: source-content-audit [--loose-only | --known-defects <manifest>] <game-content-root>".into(),
            );
        }
    };
    if !root.is_dir() {
        return Err(format!("content root is not a directory: {}", root.display()).into());
    }

    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    files.sort();
    let mut report = Report {
        known_expected,
        ..Report::default()
    };

    for path in &files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if name.ends_with("_dir.vpk") && audit_archives {
            audit_vpk(path, &mut report);
        } else if name.ends_with(".vmt") {
            report.loose_vmt += 1;
            if let Err(error) =
                fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        parse_keyvalues(&bytes, KeyValuesParseOptions::default())
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            {
                report.error(format!("{}: {error}", path.display()));
            }
        } else if name.ends_with(".vtf") {
            report.loose_vtf += 1;
            if let Err(error) =
                fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        Texture::parse(&bytes)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            {
                report.error(format!("{}: {error}", path.display()));
            }
        } else if name.ends_with(".wav") {
            report.loose_wav += 1;
            if let Err(error) =
                fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        Wave::parse(&bytes)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            {
                report.error(format!("{}: {error}", path.display()));
            }
        } else if name.ends_with(".bsp") && audit_archives {
            report.loose_bsp += 1;
            let asset_key = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            if let Err(error) =
                fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        let bsp = Bsp::parse(&bytes).map_err(|error| error.to_string())?;
                        World::parse(&bsp)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            {
                report.asset_error(&asset_key, format!("{}: {error}", path.display()));
            }
        } else if name.ends_with(".dem") {
            report.loose_dem += 1;
            if let Err(error) =
                fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        audit_demo(&bytes).map(|(packets, messages)| {
                            report.demo_packets += packets;
                            report.demo_messages += messages;
                        })
                    })
            {
                report.error(format!("{}: {error}", path.display()));
            }
        } else if name.ends_with(".pcf") || name.ends_with(".dmx") {
            report.loose_pcf += 1;
            if let Err(error) =
                fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        DmxDocument::parse(&bytes)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            {
                report.error(format!("{}: {error}", path.display()));
            }
        } else if name.ends_with(".sav") {
            audit_save(path, &mut report);
        } else if name.ends_with(".hl1") {
            report.loose_hl1 += 1;
            if let Err(error) =
                fs::read(path)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        MapState::parse(&bytes)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            {
                report.error(format!("{}: {error}", path.display()));
            }
        }
    }

    let mut missing_known: Vec<_> = report
        .known_expected
        .difference(&report.known_seen)
        .cloned()
        .collect();
    missing_known.sort();
    for key in missing_known {
        report.error(format!(
            "expected known defect was not reproduced (update the manifest): {key}"
        ));
    }
    println!(
        "audited {} VPK directories, {} BSP worlds, {} VMTs ({} packed), {} VTFs ({} packed), {} WAVs ({} packed), {} demos ({} packed; {} packets, {} network messages), {} DMX/PCFs ({} packed), {} saves ({} embedded files), {} map states ({} embedded), {} scene images, {} MDLs, {} VVDs, {} VTXs, {} PHYs",
        report.vpks,
        report.loose_bsp,
        report.loose_vmt + report.packed_vmt,
        report.packed_vmt,
        report.loose_vtf + report.packed_vtf,
        report.packed_vtf,
        report.loose_wav + report.packed_wav,
        report.packed_wav,
        report.loose_dem + report.packed_dem,
        report.packed_dem,
        report.demo_packets,
        report.demo_messages,
        report.loose_pcf + report.packed_pcf,
        report.packed_pcf,
        report.loose_sav,
        report.embedded_files,
        report.loose_hl1 + report.embedded_hl1,
        report.embedded_hl1,
        report.packed_scene_images,
        report.packed_mdl,
        report.packed_vvd,
        report.packed_vtx,
        report.packed_phy
    );
    for error in &report.errors {
        eprintln!("error: {error}");
    }
    println!(
        "reproduced {} of {} declared known defects",
        report.known_seen.len(),
        report.known_expected.len()
    );
    if report.total_errors > report.errors.len() {
        eprintln!(
            "error: {} additional errors omitted",
            report.total_errors - report.errors.len()
        );
    }
    if report.total_errors != 0 {
        return Err(format!("{} content files failed validation", report.total_errors).into());
    }
    Ok(())
}

fn audit_demo(bytes: &[u8]) -> Result<(usize, usize), String> {
    let demo = Demo::parse_with_options(
        bytes,
        DemoParseOptions {
            required_network_protocol: None,
            ..DemoParseOptions::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let options = NetParseOptions {
        network_protocol: demo.header().network_protocol,
        required_network_protocol: None,
        message_type_bits: if demo.header().network_protocol <= 7 {
            5
        } else {
            source_net::MESSAGE_TYPE_BITS
        },
        ..NetParseOptions::default()
    };
    let mut packet_count = 0usize;
    let mut message_count = 0usize;
    for command in demo.commands() {
        let packet = match &command.data {
            CommandData::Signon(packet) | CommandData::Packet(packet) => packet,
            _ => continue,
        };
        packet_count += 1;
        let stream = MessageStream::parse_with_options(
            packet.data,
            packet.data.len().saturating_mul(8),
            options,
        )
        .map_err(|error| format!("network packet at demo tick {}: {error}", command.tick))?;
        message_count += stream.messages().len();
    }
    Ok((packet_count, message_count))
}

fn audit_save(path: &Path, report: &mut Report) {
    report.loose_sav += 1;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            report.error(format!("{}: {error}", path.display()));
            return;
        }
    };
    let save = match SaveContainer::parse(&bytes) {
        Ok(save) => save,
        Err(error) => {
            report.error(format!("{}: {error}", path.display()));
            return;
        }
    };
    report.embedded_files += save.files.len();
    for file in save.files {
        if file.name.to_ascii_lowercase().ends_with(b".hl1") {
            report.embedded_hl1 += 1;
            if let Err(error) = MapState::parse(file.data) {
                report.error(format!(
                    "{}:{}: {error}",
                    path.display(),
                    String::from_utf8_lossy(file.name)
                ));
            }
        }
    }
}

fn audit_vpk(path: &Path, report: &mut Report) {
    let archive_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<archive>");
    let directory_bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            report.error(format!("{}: {error}", path.display()));
            return;
        }
    };
    let archive = match Archive::parse(&directory_bytes) {
        Ok(archive) => archive,
        Err(error) => {
            report.error(format!("{}: {error}", path.display()));
            return;
        }
    };
    report.vpks += 1;
    let mut chunks = HashMap::new();
    let mut model_checksums: HashMap<String, i32> = HashMap::new();
    let mut sidecar_checksums = Vec::new();
    for entry in archive.entries() {
        let asset_key = format!("{archive_name}:{}", entry.path);
        let lower = entry.path.to_ascii_lowercase();
        let is_vmt = lower.ends_with(".vmt");
        let is_vtf = lower.ends_with(".vtf");
        let is_wav = lower.ends_with(".wav");
        let is_dem = lower.ends_with(".dem");
        let is_pcf = lower.ends_with(".pcf") || lower.ends_with(".dmx");
        let is_scene_image = lower == "scenes/scenes.image";
        let model_kind = model_kind(&lower);
        if !is_vmt
            && !is_vtf
            && !is_wav
            && !is_dem
            && !is_pcf
            && !is_scene_image
            && model_kind.is_none()
        {
            continue;
        }
        if is_wav {
            report.packed_wav += 1;
        }
        if is_scene_image {
            report.packed_scene_images += 1;
        }
        if is_dem {
            report.packed_dem += 1;
        }
        if is_pcf {
            report.packed_pcf += 1;
        }
        let bytes = match read_entry(path, &archive, entry, &mut chunks) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.asset_error(
                    &asset_key,
                    format!("{}:{}: {error}", path.display(), entry.path),
                );
                continue;
            }
        };
        if is_vmt {
            report.packed_vmt += 1;
            if let Err(error) = parse_keyvalues(&bytes, KeyValuesParseOptions::default()) {
                report.asset_error(
                    &asset_key,
                    format!("{}:{}: {error}", path.display(), entry.path),
                );
            }
        } else if is_vtf {
            report.packed_vtf += 1;
            if let Err(error) = Texture::parse(&bytes) {
                report.asset_error(
                    &asset_key,
                    format!("{}:{}: {error}", path.display(), entry.path),
                );
            }
        } else if is_wav {
            if let Err(error) = Wave::parse(&bytes) {
                report.asset_error(
                    &asset_key,
                    format!("{}:{}: {error}", path.display(), entry.path),
                );
            }
        } else if is_dem {
            match audit_demo(&bytes) {
                Ok((packets, messages)) => {
                    report.demo_packets += packets;
                    report.demo_messages += messages;
                }
                Err(error) => {
                    report.asset_error(
                        &asset_key,
                        format!("{}:{}: {error}", path.display(), entry.path),
                    );
                }
            }
        } else if is_pcf {
            if let Err(error) = DmxDocument::parse(&bytes) {
                report.asset_error(
                    &asset_key,
                    format!("{}:{}: {error}", path.display(), entry.path),
                );
            }
        } else if is_scene_image {
            if let Err(error) = SceneImage::parse(&bytes) {
                report.asset_error(
                    &asset_key,
                    format!("{}:{}: {error}", path.display(), entry.path),
                );
            }
        } else if let Some(kind) = model_kind {
            match parse_model_asset(kind, &bytes, report) {
                Ok(checksum) => {
                    let base = model_base(&lower, kind).to_owned();
                    if kind == ModelKind::Mdl {
                        model_checksums.insert(base, checksum);
                    } else {
                        sidecar_checksums.push((base, kind, checksum, entry.path.clone()));
                    }
                }
                Err(error) => {
                    report.asset_error(
                        &asset_key,
                        format!("{}:{}: {error}", path.display(), entry.path),
                    );
                }
            }
        }
    }
    for (base, kind, checksum, entry_path) in sidecar_checksums {
        if let Some(expected) = model_checksums.get(&base) {
            if checksum != *expected {
                let asset_key = format!("{archive_name}:{entry_path}");
                report.asset_error(
                    &asset_key,
                    format!(
                        "{}:{base} ({kind:?}) checksum {checksum} does not match MDL checksum {expected}",
                        path.display()
                    ),
                );
            }
        }
    }
}

fn load_known_defects(path: &Path) -> Result<HashSet<String>, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let mut defects = HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line.split_once('\t').map_or(line, |(key, _)| key).trim();
        if !defects.insert(key.to_owned()) {
            return Err(format!("duplicate known-defect key in {}: {key}", path.display()).into());
        }
    }
    Ok(defects)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelKind {
    Mdl,
    Vvd,
    Vtx,
    Phy,
}

fn model_kind(path: &str) -> Option<ModelKind> {
    if path.ends_with(".mdl") {
        Some(ModelKind::Mdl)
    } else if path.ends_with(".vvd") {
        Some(ModelKind::Vvd)
    } else if path.ends_with(".vtx") {
        Some(ModelKind::Vtx)
    } else if path.ends_with(".phy") {
        Some(ModelKind::Phy)
    } else {
        None
    }
}

fn model_base(path: &str, kind: ModelKind) -> &str {
    match kind {
        ModelKind::Mdl => path.strip_suffix(".mdl").expect("classified MDL"),
        ModelKind::Vvd => path.strip_suffix(".vvd").expect("classified VVD"),
        ModelKind::Phy => path.strip_suffix(".phy").expect("classified PHY"),
        ModelKind::Vtx => path
            .strip_suffix(".dx90.vtx")
            .or_else(|| path.strip_suffix(".dx80.vtx"))
            .or_else(|| path.strip_suffix(".sw.vtx"))
            .or_else(|| path.strip_suffix(".vtx"))
            .expect("classified VTX"),
    }
}

fn parse_model_asset(kind: ModelKind, bytes: &[u8], report: &mut Report) -> Result<i32, String> {
    match kind {
        ModelKind::Mdl => {
            report.packed_mdl += 1;
            Mdl::parse(bytes)
                .map(|file| file.checksum)
                .map_err(|error| error.to_string())
        }
        ModelKind::Vvd => {
            report.packed_vvd += 1;
            Vvd::parse(bytes)
                .map(|file| file.checksum)
                .map_err(|error| error.to_string())
        }
        ModelKind::Vtx => {
            report.packed_vtx += 1;
            Vtx::parse(bytes)
                .map(|file| file.checksum)
                .map_err(|error| error.to_string())
        }
        ModelKind::Phy => {
            report.packed_phy += 1;
            Phy::parse(bytes)
                .map(|file| file.checksum)
                .map_err(|error| error.to_string())
        }
    }
}

fn read_entry(
    directory_path: &Path,
    archive: &Archive<'_>,
    entry: &Entry,
    chunks: &mut HashMap<u16, File>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if entry.archive_index == VPK_EMBEDDED_ARCHIVE {
        return archive
            .read_file(&entry.path, |_| unreachable!())
            .map_err(Into::into);
    }
    let file = if let Some(file) = chunks.get_mut(&entry.archive_index) {
        file
    } else {
        let index = entry.archive_index;
        chunks.insert(index, File::open(chunk_path(directory_path, index))?);
        chunks.get_mut(&index).expect("inserted chunk")
    };
    file.seek(SeekFrom::Start(u64::from(entry.offset)))?;
    let total = usize::try_from(entry.total_length())?;
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(&entry.preload);
    bytes.resize(total, 0);
    file.read_exact(&mut bytes[entry.preload.len()..])?;
    let actual = crc32(&bytes);
    if actual != entry.crc32 {
        return Err(format!(
            "CRC mismatch: expected {:08x}, got {actual:08x}",
            entry.crc32
        )
        .into());
    }
    Ok(bytes)
}

fn chunk_path(directory_file: &Path, index: u16) -> PathBuf {
    let name = directory_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let base = name
        .strip_suffix("_dir.vpk")
        .or_else(|| name.strip_suffix(".vpk"))
        .unwrap_or(name);
    directory_file.with_file_name(format!("{base}_{index:03}.vpk"))
}

fn collect_files(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for item in fs::read_dir(directory)? {
        let item = item?;
        let file_type = item.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = item.path();
        if file_type.is_dir() {
            collect_files(&path, output)?;
        } else if file_type.is_file() {
            output.push(path);
        }
    }
    Ok(())
}
