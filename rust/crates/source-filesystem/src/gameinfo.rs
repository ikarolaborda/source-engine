//! Read mounts needed before the native filesystem has initialized.
//!
//! `gameinfo.txt` owns their order, including duplicate `game` keys. This
//! bootstrap only installs GAME entries; the native filesystem later replaces
//! them with its complete read/write registry, including language and platform
//! policy. No writable path is created here.

use crate::{Position, ReadError, SearchPaths};
use source_keyvalues::{Document, Item, Node};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Parse(source_keyvalues::Error),
    Invalid(String),
    Mount(ReadError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
            Self::Invalid(message) => f.write_str(message),
            Self::Mount(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

/// Build the launcher's GAME mounts from the selected game's own declaration.
/// Relative `game` values are relative to `base`; absolute mod paths are valid.
/// An optional external content root fills missing runtime content, retaining
/// runtime overrides. Symlinked game assets are allowed just as they are in
/// the native filesystem's read registry.
pub fn load(base: &Path, game: &Path, external: Option<&Path>) -> Result<SearchPaths, Error> {
    let base = crate::absolute_path(base).map_err(Error::Mount)?;
    let game_dir = base.join(game);
    let info_path = game_dir.join("gameinfo.txt");
    let bytes = std::fs::read(&info_path).map_err(Error::Io)?;
    let document =
        source_keyvalues::parse_bytes(&bytes, Default::default()).map_err(Error::Parse)?;
    let entries = search_entries(&document)?;
    let mut paths = SearchPaths::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if !entry
            .name
            .split('+')
            .any(|id| id.trim().eq_ignore_ascii_case("game"))
        {
            continue;
        }
        if !condition_enabled(entry)? {
            continue;
        }
        let value = entry
            .string()
            .ok_or_else(|| Error::Invalid("GAME search path must be a string".into()))?;
        let value = value.replace('\\', "/");
        let candidates = locations(&base, &game_dir, external, game, &value)?;
        for candidate in candidates {
            for location in expand(&candidate)? {
                mount(&mut paths, &mut seen, &location)?;
            }
        }
    }
    if paths.is_empty() {
        return Err(Error::Invalid(format!(
            "{} has no available GAME search paths",
            info_path.display()
        )));
    }
    Ok(paths)
}

fn search_entries(document: &Document) -> Result<&[Node], Error> {
    // Fail explicitly rather than silently discarding inherited mounts.
    if document
        .items
        .iter()
        .any(|item| !matches!(item, Item::Node(_)))
    {
        return Err(Error::Invalid(
            "gameinfo #include/#base directives are not supported by the startup mount loader"
                .into(),
        ));
    }
    let root = document
        .roots()
        .find(|node| node.name.eq_ignore_ascii_case("GameInfo"))
        .ok_or_else(|| Error::Invalid("missing GameInfo block".into()))?;
    let filesystem = child(root, "FileSystem")?;
    let search = child(filesystem, "SearchPaths")?;
    // Conditions on containers must not be silently ignored either.
    for node in [root, filesystem, search] {
        if !condition_enabled(node)? {
            return Err(Error::Invalid(format!("disabled {} block", node.name)));
        }
    }
    search
        .children()
        .ok_or_else(|| Error::Invalid("SearchPaths must be a block".into()))
}

fn child<'a>(node: &'a Node, name: &str) -> Result<&'a Node, Error> {
    node.children()
        .unwrap_or_default()
        .iter()
        .find(|node| node.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| Error::Invalid(format!("missing {name} block")))
}

fn condition_enabled(node: &Node) -> Result<bool, Error> {
    let Some(condition) = &node.condition else {
        return Ok(true);
    };
    let condition = condition.trim_matches(['[', ']']);
    let (negate, symbol) = match condition.strip_prefix('!') {
        Some(symbol) => (true, symbol),
        None => (false, condition),
    };
    // Match EvaluateConditional in tier1/KeyValues.cpp, including its historical
    // WIN32=PC and LINUX=POSIX-desktop aliases and disabled OSX symbol.
    let enabled = match symbol.to_ascii_uppercase().as_str() {
        "$WIN32" => true,
        "$WINDOWS" => cfg!(windows),
        "$POSIX" | "$LINUX" => cfg!(unix),
        "$X360" | "$OSX" => false,
        "$DECK" => {
            std::env::var("SteamDeck").is_ok_and(|value| value.parse::<i32>().unwrap_or(0) != 0)
        }
        _ => {
            return Err(Error::Invalid(format!(
                "unsupported gameinfo condition {condition}"
            )))
        }
    };
    Ok(enabled ^ negate)
}

fn locations(
    base: &Path,
    game_dir: &Path,
    external: Option<&Path>,
    game: &Path,
    value: &str,
) -> Result<Vec<PathBuf>, Error> {
    let lower = value.to_ascii_lowercase();
    let (root, relative, external_relative) = if lower.starts_with("|gameinfo_path|") {
        let rest = &value[15..];
        (
            game_dir,
            rest,
            (!game.is_absolute()).then(|| game.join(rest)),
        )
    } else if lower.starts_with("|all_source_engine_paths|") {
        let rest = &value[25..];
        (base, rest, Some(PathBuf::from(rest)))
    } else {
        (base, value, Some(PathBuf::from(value)))
    };
    if relative.contains('|') || relative.contains('\0') || value.is_empty() {
        return Err(Error::Invalid(format!(
            "invalid gameinfo search path {value:?}"
        )));
    }
    let mut result = vec![root.join(relative)];
    if let (Some(external), Some(relative)) = (external, external_relative) {
        if !relative.is_absolute() {
            let fallback = external.join(relative);
            if fallback != result[0] {
                result.push(fallback);
            }
        }
    }
    Ok(result)
}

fn expand(path: &Path) -> Result<Vec<PathBuf>, Error> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new("."));
    if parent.to_string_lossy().contains(['*', '?']) {
        return Err(Error::Invalid(
            "wildcards in search-path parent directories are unsupported".into(),
        ));
    }
    if !name.contains(['*', '?']) {
        return Ok(vec![path.to_owned()]);
    }
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Error::Io(error)),
    };
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(Error::Io)?;
        let filename = entry.file_name();
        let filename = filename.to_string_lossy();
        if !filename.starts_with('.')
            && crate::wildcard_matches(name, &filename)
            && (entry.path().is_dir() || filename.to_ascii_lowercase().ends_with(".vpk"))
        {
            result.push(entry.path());
        }
    }
    result.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    // A multi-file VPK's numbered data chunks are not directory archives.
    let prefixes: Vec<_> = result
        .iter()
        .filter_map(|path| {
            path.to_str()
                .and_then(|path| path.strip_suffix("_dir.vpk"))
                .map(str::to_owned)
        })
        .collect();
    result.retain(|path| {
        let path = path.to_string_lossy();
        !prefixes.iter().any(|prefix| {
            path.strip_prefix(&format!("{prefix}_"))
                .and_then(|suffix| suffix.strip_suffix(".vpk"))
                .is_some_and(|index| {
                    index.len() == 3 && index.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
    });
    Ok(result)
}

fn mount(
    paths: &mut SearchPaths,
    seen: &mut HashSet<PathBuf>,
    location: &Path,
) -> Result<(), Error> {
    let is_vpk = location
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("vpk"));
    let location = if is_vpk && !location.is_file() {
        let stem = location.file_stem().unwrap_or_default().to_string_lossy();
        location.with_file_name(format!("{stem}_dir.vpk"))
    } else {
        location.to_owned()
    };
    let canonical = match std::fs::canonicalize(&location) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Error::Io(error)),
    };
    if !seen.insert(canonical) {
        return Ok(());
    }
    if is_vpk {
        paths
            .mount_vpk(&location, "GAME", Position::Tail)
            .map_err(Error::Mount)
    } else {
        paths
            .mount_directory_with_flags(&location, "GAME", Position::Tail, false, true)
            .map_err(Error::Mount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "source-gameinfo-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(root.join("ep2")).unwrap();
            Self(root)
        }
        fn info(&self, entries: &str) {
            std::fs::write(
                self.0.join("ep2/gameinfo.txt"),
                format!("GameInfo {{ FileSystem {{ SearchPaths {{ {entries} }} }} }}"),
            )
            .unwrap();
        }
        fn vpk(&self, path: &str, files: &[(&str, &[u8])]) {
            let mut builder = source_vpk::Builder::new();
            for (name, bytes) in files {
                builder.add_file(*name, bytes.to_vec()).unwrap();
            }
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, builder.build_v1_embedded().unwrap()).unwrap();
        }
        fn load(&self) -> Result<SearchPaths, Error> {
            load(&self.0, Path::new("ep2"), None)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn episode_archives_precede_base_and_loose_files() {
        let f = Fixture::new();
        f.info(r#"game+mod ep2/ep2_pak.vpk game |all_source_engine_paths|episodic/ep1_pak.vpk game hl2/hl2_pak.vpk game |gameinfo_path|. gamebin episodic/bin"#);
        f.vpk(
            "ep2/ep2_pak_dir.vpk",
            &[("maps/opening.bsp", b"episode two")],
        );
        f.vpk(
            "episodic/ep1_pak_dir.vpk",
            &[
                ("maps/opening.bsp", b"episode one"),
                ("maps/fallback.bsp", b"one"),
            ],
        );
        f.vpk(
            "hl2/hl2_pak_dir.vpk",
            &[("maps/opening.bsp", b"base"), ("cfg/valve.rc", b"exec")],
        );
        std::fs::create_dir_all(f.0.join("ep2/maps")).unwrap();
        std::fs::write(f.0.join("ep2/maps/opening.bsp"), b"loose").unwrap();
        let paths = f.load().unwrap();
        assert_eq!(paths.len(), 4);
        assert_eq!(
            paths.read("maps/opening.bsp", Some("GAME")).unwrap(),
            b"episode two"
        );
        assert_eq!(
            paths.read("maps/fallback.bsp", Some("GAME")).unwrap(),
            b"one"
        );
        assert_eq!(paths.read("cfg/valve.rc", Some("GAME")).unwrap(), b"exec");
    }

    #[test]
    fn custom_mounts_are_sorted_and_chunks_are_not_mounted() {
        let f = Fixture::new();
        f.info("game ep2/custom/* game ep2/ep2_pak.vpk");
        f.vpk("ep2/custom/b_dir.vpk", &[("same.txt", b"b")]);
        f.vpk("ep2/custom/a.vpk", &[("same.txt", b"a")]);
        f.vpk("ep2/custom/b_extra.vpk", &[("extra.txt", b"extra")]);
        f.vpk("ep2/ep2_pak_dir.vpk", &[("same.txt", b"stock")]);
        std::fs::write(f.0.join("ep2/custom/b_000.vpk"), b"not a directory").unwrap();
        let paths = f.load().unwrap();
        assert_eq!(paths.len(), 4);
        assert_eq!(paths.read("extra.txt", Some("GAME")).unwrap(), b"extra");
        assert_eq!(paths.read("same.txt", Some("GAME")).unwrap(), b"a");
    }

    #[test]
    fn external_content_fills_missing_mounts_after_runtime_overrides() {
        let f = Fixture::new();
        f.info("game ep2/ep2_pak.vpk game |gameinfo_path|.");
        f.vpk(
            "content/ep2/ep2_pak_dir.vpk",
            &[("packed.txt", b"external")],
        );
        std::fs::write(f.0.join("ep2/loose.txt"), b"runtime").unwrap();
        std::fs::write(f.0.join("content/ep2/loose.txt"), b"external").unwrap();
        let paths = load(&f.0, Path::new("ep2"), Some(&f.0.join("content"))).unwrap();
        assert_eq!(paths.read("packed.txt", Some("GAME")).unwrap(), b"external");
        assert_eq!(paths.read("loose.txt", Some("GAME")).unwrap(), b"runtime");
    }

    #[test]
    #[cfg(unix)]
    fn absolute_mods_and_symlinked_assets_work_without_an_external_root() {
        let f = Fixture::new();
        f.info("game |gameinfo_path|.");
        std::fs::create_dir_all(f.0.join("content")).unwrap();
        std::fs::write(f.0.join("content/sound.wav"), b"audio").unwrap();
        std::os::unix::fs::symlink(f.0.join("content"), f.0.join("ep2/sound")).unwrap();
        let paths = load(Path::new("/"), &f.0.join("ep2"), None).unwrap();
        assert_eq!(
            paths.read("sound/sound.wav", Some("GAME")).unwrap(),
            b"audio"
        );
    }

    #[test]
    fn malformed_present_archives_fail_instead_of_loading_another_games_map() {
        let f = Fixture::new();
        f.info("game ep2/broken.vpk game ep2");
        std::fs::write(f.0.join("ep2/broken.vpk"), b"invalid").unwrap();
        assert!(matches!(f.load(), Err(Error::Mount(ReadError::Vpk(_)))));
    }

    #[test]
    fn missing_optional_mounts_are_skipped_but_missing_gameinfo_is_an_error() {
        let f = Fixture::new();
        f.info("game ep2/custom/* game ep2/absent.vpk game ep2");
        assert_eq!(f.load().unwrap().len(), 1);
        assert!(load(&f.0, Path::new("unknown"), None).is_err());
        f.info("game ep2/absent.vpk");
        assert!(matches!(f.load(), Err(Error::Invalid(_))));
    }

    #[test]
    fn conditional_mounts_and_invalid_declarations_are_not_silently_ignored() {
        let f = Fixture::new();
        f.info("game ep2/broken.vpk [$X360] game ep2 [!$X360]");
        std::fs::write(f.0.join("ep2/broken.vpk"), b"invalid").unwrap();
        assert_eq!(f.load().unwrap().len(), 1);
        f.info("game ep2 [$UNKNOWN]");
        assert!(matches!(f.load(), Err(Error::Invalid(_))));
        f.info("game |unknown|ep2");
        assert!(matches!(f.load(), Err(Error::Invalid(_))));
    }
}
