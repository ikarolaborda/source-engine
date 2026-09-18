use source_vpk::{crc32, Archive, Builder, Error as VpkError, VPK_EMBEDDED_ARCHIVE};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

fn usage() -> ! {
    eprintln!("usage:\n  source-vpk list <archive_dir.vpk>\n  source-vpk verify <archive_dir.vpk>\n  source-vpk extract <archive_dir.vpk> <output-dir>\n  source-vpk extract-file <archive_dir.vpk> <vpk-path> <output-file>\n  source-vpk pack <input-dir> <archive_dir.vpk>");
    std::process::exit(2);
}

fn main() {
    if let Err(error) = run() {
        eprintln!("source-vpk: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    match args.as_slice() {
        [_, command, archive] if command == "list" => list(Path::new(archive)),
        [_, command, archive] if command == "verify" => verify(Path::new(archive)),
        [_, command, archive, output] if command == "extract" => {
            extract(Path::new(archive), Path::new(output))
        }
        [_, command, archive, entry, output] if command == "extract-file" => {
            extract_file(Path::new(archive), entry, Path::new(output))
        }
        [_, command, input, archive] if command == "pack" => {
            pack(Path::new(input), Path::new(archive))
        }
        _ => usage(),
    }
}

fn verify(path: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let archive = Archive::parse(&bytes)?;
    let mut chunks: HashMap<u16, File> = HashMap::new();
    let mut entries: Vec<_> = archive.entries().iter().collect();
    entries.sort_by_key(|entry| (entry.archive_index, entry.offset));
    for entry in entries {
        if entry.archive_index == VPK_EMBEDDED_ARCHIVE {
            archive.read_file(&entry.path, |_| unreachable!())?;
            continue;
        }

        let chunk = if let Some(chunk) = chunks.get_mut(&entry.archive_index) {
            chunk
        } else {
            let index = entry.archive_index;
            let chunk =
                File::open(chunk_path(path, index)).map_err(|_| VpkError::MissingArchive(index))?;
            chunks.insert(index, chunk);
            chunks.get_mut(&index).unwrap()
        };
        chunk.seek(SeekFrom::Start(u64::from(entry.offset)))?;
        let mut contents = Vec::with_capacity(entry.total_length() as usize);
        contents.extend_from_slice(&entry.preload);
        contents.resize(entry.total_length() as usize, 0);
        chunk.read_exact(&mut contents[entry.preload.len()..])?;
        let actual = crc32(&contents);
        if actual != entry.crc32 {
            return Err(VpkError::CrcMismatch {
                path: entry.path.clone(),
                expected: entry.crc32,
                actual,
            }
            .into());
        }
    }
    println!("verified {} entries", archive.entries().len());
    Ok(())
}

fn list(path: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let archive = Archive::parse(&bytes)?;
    for entry in archive.entries() {
        println!(
            "{:08x} total={:10} preload={:5} archive={:05} offset={:10} stored={:10} {}",
            entry.crc32,
            entry.total_length(),
            entry.preload.len(),
            entry.archive_index,
            entry.offset,
            entry.length,
            entry.path
        );
    }
    Ok(())
}

fn extract(path: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let archive = Archive::parse(&bytes)?;
    fs::create_dir_all(output)?;
    for entry in archive.entries() {
        let data = archive.read_file(&entry.path, |index| {
            fs::read(chunk_path(path, index)).map_err(|_| VpkError::MissingArchive(index))
        })?;
        let destination = output.join(&entry.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, data)?;
    }
    Ok(())
}

fn extract_file(path: &Path, entry_path: &str, output: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let archive = Archive::parse(&bytes)?;
    let data = archive.read_file(entry_path, |index| {
        fs::read(chunk_path(path, index)).map_err(|_| VpkError::MissingArchive(index))
    })?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, data)?;
    Ok(())
}

fn pack(input: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    if !input.is_dir() {
        return Err(format!("input is not a directory: {}", input.display()).into());
    }
    let mut paths = Vec::new();
    collect_files(input, input, &mut paths)?;
    paths.sort();

    let mut builder = Builder::new();
    for path in paths {
        let relative = path.strip_prefix(input)?;
        let vpk_path = relative
            .to_str()
            .ok_or_else(|| format!("non-UTF-8 path: {}", relative.display()))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        builder.add_file(vpk_path, fs::read(path)?)?;
    }
    let bytes = builder.build_v1_embedded()?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, bytes)?;
    Ok(())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    for item in fs::read_dir(directory)? {
        let item = item?;
        let file_type = item.file_type()?;
        let path = item.path();
        if file_type.is_symlink() {
            return Err(format!(
                "refusing symlink below {}: {}",
                root.display(),
                path.display()
            )
            .into());
        }
        if file_type.is_dir() {
            collect_files(root, &path, output)?;
        } else if file_type.is_file() {
            output.push(path);
        }
    }
    Ok(())
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
