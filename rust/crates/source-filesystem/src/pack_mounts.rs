//! Numbered ZIP discovery in mount-precedence order. Discovery does not parse
//! archives: a present but malformed ZIP must not hide later numbered ZIPs.
use crate::{FindEntry, MAX_VIRTUAL_PATH_BYTES};
use std::io;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Naming {
    Desktop,
    Xbox360,
}

// Bound both work and snapshot storage even for an adversarial directory.
const MAX_SERIES: u32 = 65_536;

/// Snapshot candidates, highest priority first. Each series ends at the first
/// failed metadata lookup, matching the native stat-based discovery policy
/// (including permission errors). Symlinks are followed, directories included;
/// archive opening/validation remains a separate operation. Xbox localized
/// archives precede the base series. The caller decides whether localization
/// applies to this mount. No partial snapshot is returned on invalid input or
/// the explicit series limit. Returned paths retain the supplied root spelling.
pub fn discover(root: &Path, naming: Naming, language: Option<&str>) -> io::Result<Vec<FindEntry>> {
    discover_with(root, naming, language, |path| {
        std::fs::metadata(path).is_ok()
    })
}

fn discover_with(
    root: &Path,
    naming: Naming,
    language: Option<&str>,
    mut exists: impl FnMut(&Path) -> bool,
) -> io::Result<Vec<FindEntry>> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid pack discovery path or language",
        )
    };
    let root_text = root.to_str().ok_or_else(invalid)?;
    if root_text.is_empty()
        || root_text.contains('\0')
        || root_text.len() > MAX_VIRTUAL_PATH_BYTES
        || language.is_some_and(|s| {
            naming != Naming::Xbox360
                || s.is_empty()
                || s.len() > 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return Err(invalid());
    }
    let mut entries = Vec::new();
    // Append base then localized; reverse once to prioritize both descending
    // numbers and localized overrides. Never sort lexically (zip10 vs zip2).
    for locale in std::iter::once(None).chain(language.map(Some)) {
        for number in 0..=MAX_SERIES {
            let name = match (naming, locale) {
                (Naming::Desktop, _) => format!("zip{number}.zip"),
                (Naming::Xbox360, None) => format!("zip{number}.360.zip"),
                (Naming::Xbox360, Some(language)) => format!("zip{number}_{language}.360.zip"),
            };
            let path = root.join(name);
            let name = path.to_str().ok_or_else(invalid)?;
            if name.len() > MAX_VIRTUAL_PATH_BYTES {
                return Err(invalid());
            }
            if !exists(&path) {
                break;
            }
            if number == MAX_SERIES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "pack series limit exceeded",
                ));
            }
            entries.push(FindEntry {
                name: name.to_owned(),
                is_directory: false,
            });
        }
    }
    entries.reverse();
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_precedence_and_first_gap() {
        let mut inspected = Vec::new();
        let entries = discover_with(Path::new("content"), Naming::Desktop, None, |path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            inspected.push(name.to_owned());
            name != "zip12.zip"
        })
        .unwrap();
        assert_eq!(entries.len(), 12);
        for (entry, number) in entries.iter().zip((0..12).rev()) {
            assert_eq!(
                Path::new(&entry.name),
                Path::new("content").join(format!("zip{number}.zip"))
            );
            assert!(!entry.is_directory);
        }
        assert_eq!(inspected.len(), 13);
        assert_eq!(inspected.last().unwrap(), "zip12.zip");
        assert!(
            discover_with(Path::new("empty"), Naming::Desktop, None, |_| false)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn localized_series_is_independent_and_overrides_base() {
        let entries = discover_with(
            Path::new("content"),
            Naming::Xbox360,
            Some("french"),
            |path| {
                matches!(
                    path.file_name().unwrap().to_str().unwrap(),
                    "zip0.360.zip" | "zip1.360.zip" | "zip0_french.360.zip" | "zip1_french.360.zip"
                )
            },
        )
        .unwrap();
        let names: Vec<_> = entries
            .iter()
            .map(|e| Path::new(&e.name).file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "zip1_french.360.zip",
                "zip0_french.360.zip",
                "zip1.360.zip",
                "zip0.360.zip"
            ]
        );
        let entries = discover_with(
            Path::new("content"),
            Naming::Xbox360,
            Some("french"),
            |path| path.file_name().unwrap() == "zip0_french.360.zip",
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn rejects_invalid_inputs_before_io_and_bounds_work() {
        for (root, naming, language) in [
            ("", Naming::Desktop, None),
            ("a\0b", Naming::Desktop, None),
            ("content", Naming::Desktop, Some("french")),
            ("content", Naming::Xbox360, Some("")),
            ("content", Naming::Xbox360, Some("../escape")),
        ] {
            assert_eq!(
                discover_with(Path::new(root), naming, language, |_| panic!(
                    "invalid input reached IO"
                ))
                .unwrap_err()
                .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let mut calls = 0;
        assert_eq!(
            discover_with(Path::new("content"), Naming::Desktop, None, |_| {
                calls += 1;
                true
            })
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(calls, MAX_SERIES + 1);
    }
}
