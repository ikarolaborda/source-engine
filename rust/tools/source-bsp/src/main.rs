use source_bsp::{Bsp, World};
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

fn usage() -> ! {
    eprintln!("usage:\n  source-bsp info <map.bsp>\n  source-bsp world-info <map.bsp>\n  source-bsp verify-roundtrip <map.bsp>\n  source-bsp repack <input.bsp> <output.bsp>");
    std::process::exit(2);
}

fn main() {
    if let Err(error) = run() {
        eprintln!("source-bsp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    match args.as_slice() {
        [_, command, input] if command == "info" => info(Path::new(input)),
        [_, command, input] if command == "world-info" => world_info(Path::new(input)),
        [_, command, input] if command == "verify-roundtrip" => verify_roundtrip(Path::new(input)),
        [_, command, input, output] if command == "repack" => {
            repack(Path::new(input), Path::new(output))
        }
        _ => usage(),
    }
}

fn world_info(path: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let bsp = Bsp::parse(&bytes)?;
    let world = World::parse(&bsp)?;
    println!(
        "planes={} nodes={} leaves={} clusters={}",
        world.planes().len(),
        world.nodes().len(),
        world.leaves().len(),
        world.cluster_count()
    );
    Ok(())
}

fn verify_roundtrip(path: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let original = Bsp::parse(&bytes)?;
    let rebuilt_bytes = original.to_builder().build()?;
    let rebuilt = Bsp::parse(&rebuilt_bytes)?;
    if original.header().version != rebuilt.header().version
        || original.header().map_revision != rebuilt.header().map_revision
    {
        return Err("BSP header changed during round trip".into());
    }
    for index in 0..source_bsp::BSP_LUMP_COUNT {
        let left = original.header().lumps[index];
        let right = rebuilt.header().lumps[index];
        if left.version != right.version
            || left.uncompressed_size != right.uncompressed_size
            || original.lump(index) != rebuilt.lump(index)
        {
            return Err(format!("BSP lump {index} changed during round trip").into());
        }
    }
    println!(
        "verified {} lumps ({} -> {} bytes)",
        source_bsp::BSP_LUMP_COUNT,
        bytes.len(),
        rebuilt_bytes.len()
    );
    Ok(())
}

fn info(path: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let bsp = Bsp::parse(&bytes)?;
    println!(
        "version={} map_revision={}",
        bsp.header().version,
        bsp.header().map_revision
    );
    for (index, lump) in bsp.header().lumps.iter().enumerate() {
        if lump.length != 0 {
            println!(
                "lump={index:02} offset={} length={} version={} uncompressed_size={}",
                lump.offset, lump.length, lump.version, lump.uncompressed_size
            );
        }
    }
    Ok(())
}

fn repack(input: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(input)?;
    let bsp = Bsp::parse(&bytes)?;
    let repacked = bsp.to_builder().build()?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, repacked)?;
    Ok(())
}
