#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(index) = source_pak::Index::parse(data) {
        for (at, entry) in index.entries().iter().enumerate() {
            assert_eq!(index.find(&entry.path), Some(at));
            assert!(entry.data_offset() + u64::from(entry.compressed_length) <= data.len() as u64);
        }
        for pattern in [
            "*",
            "*.*",
            "*.?mt",
            "../*",
            "MATERIALS/*",
            "materials/a*/../*",
        ] {
            let _ = source_filesystem::find_pack_index(&index, pattern);
        }
        // Exercise globs derived from mutated, valid archive names. Cap queries
        // independently of archive size so a seed cannot create quadratic work.
        for entry in index.entries().iter().take(16) {
            let pattern: String = entry
                .path
                .chars()
                .enumerate()
                .map(|(at, c)| if c != '/' && at % 3 == 0 { '?' } else { c })
                .collect();
            let _ = source_filesystem::find_pack_index(&index, &pattern);
        }
    }
    let limits = source_pak::Limits {
        max_archive_size: 1024 * 1024,
        max_entries: 128,
        max_entry_size: 64 * 1024,
        max_dictionary_size: 16 * 1024 * 1024,
        ..Default::default()
    };
    if let Ok(pak) = source_pak::Pak::parse_with_limits(data, limits) {
        let shared = source_filesystem::pack_archive::Archive::memory(data).unwrap();
        let mut paths = source_filesystem::SearchPaths::new();
        paths
            .mount_pak(data, "memory", "MOD", source_filesystem::Position::Head)
            .unwrap();
        assert!(paths.find("*", Some("BSP")).unwrap().is_empty());
        paths
            .mount_pak(data, "memory", "GAME", source_filesystem::Position::Tail)
            .unwrap();
        for pattern in ["*", "*.*", "MATERIALS/./sub/../*.?mt", "../*", "a*/../*"] {
            let indexed = source_filesystem::find_pack_index(shared.index(), pattern);
            let mounted = paths.find(pattern, Some("bSp"));
            match (indexed, mounted) {
                (Ok(indexed), Ok(mounted)) => {
                    let indexed: Vec<_> = indexed
                        .into_iter()
                        .map(|mut entry| {
                            entry.name = entry.name.rsplit('/').next().unwrap().to_owned();
                            entry
                        })
                        .collect();
                    assert_eq!(indexed, mounted);
                }
                (Err(_), Err(_)) => {}
                _ => panic!("map mount and archive wildcard normalization diverged"),
            }
        }
        for entry in pak.entries() {
            if entry.path == "__preload_section.pre" {
                assert!(shared.entry(&entry.path).is_none());
                continue;
            }
            // Parsing above bounds entry sizes for this fuzz workload. The
            // production shared owner has a larger dictionary limit; whenever
            // the stricter oracle succeeds, decoded bytes must still agree.
            if let Ok(expected) = pak.read(&entry.path) {
                assert_eq!(shared.read(&entry.path).unwrap(), expected);
            } else {
                let _ = shared.read(&entry.path);
            }
        }
    }
});
