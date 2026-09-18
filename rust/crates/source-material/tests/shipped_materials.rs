//! Parses every material Half-Life 2 ships and checks that the texture paths
//! they resolve to actually exist in the archives.
//!
//! The unit tests pin the rules against one material. What they cannot show
//! is whether those rules hold across five thousand written by hand over
//! years, where case and separators are inconsistent and some materials are
//! patches of others. A path that is nearly right resolves to nothing and
//! textures the world with whatever the fallback is, silently, so the check
//! that matters is that each path is found.
//!
//! Shipped content is not redistributable, so this reports that it was
//! skipped when no installation is present.

use source_material::Material;
use source_vpk::OwnedArchive;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn content_root() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(content) = std::env::var("SOURCE_HL2_CONTENT_ROOT") {
        roots.push(PathBuf::from(content));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(
            PathBuf::from(home)
                .join("Library/Application Support/Steam/steamapps/common/Half-Life 2"),
        );
    }
    roots
        .into_iter()
        .find(|root| root.join("hl2/hl2_misc_dir.vpk").is_file())
}

/// One archive and the name it reads its numbered chunks under.
struct Pak {
    name: String,
    root: PathBuf,
    archive: OwnedArchive,
}

impl Pak {
    fn open(root: &Path, name: &str) -> Option<Self> {
        let bytes = std::fs::read(root.join(format!("hl2/{name}_dir.vpk"))).ok()?;
        Some(Self {
            name: name.to_owned(),
            root: root.to_owned(),
            archive: OwnedArchive::parse(&bytes).ok()?,
        })
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.archive
            .read_file(path, |index| {
                let chunk = self.root.join(format!("hl2/{}_{index:03}.vpk", self.name));
                std::fs::read(&chunk)
                    .map_err(|_| source_vpk::Error::MissingEntry(chunk.display().to_string()))
            })
            .ok()
    }

    fn holds(&self, path: &str) -> bool {
        self.archive.entry(path).is_some()
    }
}

#[test]
fn resolves_every_shipped_material_to_textures_that_exist() {
    let Some(root) = content_root() else {
        eprintln!("skipped: no installed Half-Life 2 content to read materials from");
        return;
    };

    let paks: Vec<Pak> = ["hl2_misc", "hl2_textures", "hl2_pak"]
        .iter()
        .filter_map(|name| Pak::open(&root, name))
        .collect();
    assert!(!paks.is_empty(), "the installation holds readable archives");

    // Materials are read once and kept, because a patch asks for the
    // material it includes and thousands of lookups through the archive
    // would otherwise dominate the run.
    let mut texts: HashMap<String, String> = HashMap::new();
    for pak in &paks {
        for entry in pak.archive.entries() {
            if entry.path.ends_with(".vmt") && !texts.contains_key(&entry.path) {
                if let Some(bytes) = pak.read(&entry.path) {
                    texts.insert(
                        entry.path.clone(),
                        String::from_utf8_lossy(&bytes).into_owned(),
                    );
                }
            }
        }
    }
    assert!(
        texts.len() > 4000,
        "Half-Life 2 ships thousands of materials, found {}",
        texts.len()
    );

    let mut parsed = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut with_base = 0usize;
    let mut missing_texture: Vec<(String, String)> = Vec::new();
    let mut patched = 0usize;

    for (path, text) in &texts {
        let material = match Material::parse(text) {
            Ok(material) => material,
            Err(source_material::Error::UnresolvedPatch(_)) => {
                patched += 1;
                match source_material::resolve(text, |include| texts.get(include).cloned()) {
                    Ok(material) => material,
                    Err(error) => {
                        failed.push((path.clone(), error.to_string()));
                        continue;
                    }
                }
            }
            Err(error) => {
                failed.push((path.clone(), error.to_string()));
                continue;
            }
        };
        parsed += 1;

        let Some(texture) = material.base_texture() else {
            continue;
        };
        with_base += 1;

        // A texture named by a material has to be found under the path this
        // crate produces, which is the whole point of normalizing it.
        //
        // Animated and compiled-in textures are named without a file: an
        // `_rt_` prefix is a render target the engine creates, and `env_cubemap`
        // is built from the map.
        if texture.contains("_rt_") || texture.contains("env_cubemap") {
            continue;
        }
        if !paks.iter().any(|pak| pak.holds(&texture)) {
            missing_texture.push((path.clone(), texture));
        }
    }

    assert!(
        failed.is_empty(),
        "{} of {} materials failed to parse, first few: {:?}",
        failed.len(),
        texts.len(),
        &failed[..failed.len().min(5)]
    );
    assert_eq!(parsed, texts.len(), "every material parsed");
    assert!(
        with_base * 10 > parsed * 7,
        "most materials name a base texture, {with_base} of {parsed} did"
    );

    // A handful of shipped materials do name textures that are not in the
    // base game's archives, because they belong to the episodes or were left
    // behind, so this bounds the shortfall rather than requiring none.
    let missing = missing_texture.len();
    assert!(
        missing * 100 < with_base,
        "{missing} of {with_base} named textures were not found, first few: {:?}",
        &missing_texture[..missing.min(10)]
    );

    eprintln!(
        "{parsed} materials parsed ({patched} patches), {with_base} name a base texture, {missing} of those not in the base archives: {:?}",
        &missing_texture[..missing.min(30)]
    );
}
