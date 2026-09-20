#![no_main]
use libfuzzer_sys::fuzz_target;
use source_filesystem::search_plan::{Candidate, Filter, SearchPlan};

fuzz_target!(|data: &[u8]| {
    let Some((&mode, data)) = data.split_first() else {
        return;
    };
    let ids = ["GAME", "game", "MOD", "BSP", "", "private", "éID", "ÉID"];
    let request = if mode & 8 != 0 {
        None
    } else {
        Some(ids[usize::from(mode & 7)])
    };
    let filter = match mode % 3 {
        0 => Filter::All,
        1 => Filter::CullPack,
        _ => Filter::CullNonPack,
    };
    let paths: Vec<_> = data
        .chunks_exact(6)
        .take(256)
        .map(|chunk| {
            let flags = chunk[5];
            Candidate {
                path_id: ids[usize::from(chunk[0] & 7)],
                store_id: i32::from_le_bytes(chunk[1..5].try_into().unwrap()),
                pack: flags & 1 != 0,
                map: flags & 3 == 3,
                by_request_only: flags & 4 != 0,
                excluded: flags & 8 != 0,
            }
        })
        .collect();
    // Deliberately linear reference visits, independent of the production set.
    let mut seen = Vec::new();
    let mut expected = Vec::new();
    for (at, path) in paths.iter().enumerate() {
        if (mode % 3 == 1 && path.pack) || (mode % 3 == 2 && !path.pack) {
            continue;
        }
        match request {
            None if path.by_request_only => continue,
            Some(id) if id.eq_ignore_ascii_case("BSP") => {
                if !path.map || !path.path_id.eq_ignore_ascii_case("GAME") {
                    continue;
                }
            }
            Some(id) if !path.path_id.eq_ignore_ascii_case(id) => continue,
            _ => {}
        }
        if path.excluded || seen.contains(&path.store_id) {
            continue;
        }
        seen.push(path.store_id);
        expected.push(at as u32);
    }
    let mut plan = SearchPlan::new(&paths, request, filter);
    assert_eq!(plan.by_ref().collect::<Vec<_>>(), expected);
    assert_eq!(plan.next(), None);
    plan.reset();
    assert_eq!(plan.collect::<Vec<_>>(), expected);
});
