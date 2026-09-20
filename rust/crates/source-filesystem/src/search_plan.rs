//! Ordered read-search snapshots and duplicate physical-store suppression.
//! Native resources are not retained here: results index the caller's snapshot.
use crate::selection::path_id_matches;
use std::collections::HashSet;

#[derive(Default)]
pub struct StoreVisits(HashSet<i32>);

impl StoreVisits {
    /// True for a store already encountered; signed IDs are opaque identities.
    pub fn mark(&mut self, store: i32) -> bool {
        !self.0.insert(store)
    }

    pub fn reset(&mut self) {
        self.0.clear();
    }
}

#[derive(Clone, Copy)]
pub enum Filter {
    All,
    CullPack,
    CullNonPack,
}

pub struct Candidate<'a> {
    pub path_id: &'a str,
    pub store_id: i32,
    pub by_request_only: bool,
    /// ZIP/BSP only; native VPK mounts are not GetPackFile() paths.
    pub pack: bool,
    pub map: bool,
    /// Platform-specific exclusion supplied by the transitional native adapter.
    pub excluded: bool,
}

pub struct SearchPlan {
    order: Vec<u32>,
    position: usize,
}

impl SearchPlan {
    pub fn new(paths: &[Candidate<'_>], requested: Option<&str>, filter: Filter) -> Self {
        let mut visits = StoreVisits::default();
        let order = paths
            .iter()
            .enumerate()
            .filter(|(_, path)| {
                let kind_matches = match filter {
                    Filter::All => true,
                    Filter::CullPack => !path.pack,
                    Filter::CullNonPack => path.pack,
                };
                kind_matches
                    && path_id_matches(path.path_id, requested, path.by_request_only, path.map)
                    && !path.excluded
                    && !visits.mark(path.store_id)
            })
            .map(|(index, _)| u32::try_from(index).expect("search snapshot exceeds u32 indices"))
            .collect();
        Self { order, position: 0 }
    }

    pub fn reset(&mut self) {
        self.position = 0;
    }
}

impl Iterator for SearchPlan {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        let index = *self.order.get(self.position)?;
        self.position += 1;
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_mounts_do_not_hide_later_aliases_and_reset_replays_order() {
        let ids = ["MOD", "GAME", "GAME", "GAME", "GAME", "GAME"];
        let paths: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(at, id)| Candidate {
                path_id: id,
                store_id: if at < 3 { -1 } else { i32::MIN },
                by_request_only: at == 1,
                pack: at >= 3,
                map: at >= 3,
                excluded: at == 3,
            })
            .collect();
        let mut plan = SearchPlan::new(&paths, Some("GAME"), Filter::All);
        assert_eq!(plan.by_ref().collect::<Vec<_>>(), [1, 4]);
        assert_eq!(plan.next(), None);
        plan.reset();
        assert_eq!(plan.collect::<Vec<_>>(), [1, 4]);
        assert_eq!(
            SearchPlan::new(&paths, Some("BSP"), Filter::All).collect::<Vec<_>>(),
            [4]
        );
        assert_eq!(
            SearchPlan::new(&paths, Some("GAME"), Filter::CullPack).collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(
            SearchPlan::new(&paths, Some("GAME"), Filter::CullNonPack).collect::<Vec<_>>(),
            [4]
        );
        assert_eq!(
            SearchPlan::new(&paths, None, Filter::All).collect::<Vec<_>>(),
            [0, 4]
        );
    }

    #[test]
    fn visits_keep_all_signed_store_ids_and_are_independent() {
        let mut a = StoreVisits::default();
        let mut b = StoreVisits::default();
        for id in [0, -1, i32::MIN, i32::MAX, 1, 65536] {
            assert!(!a.mark(id));
            assert!(a.mark(id));
            assert!(!b.mark(id));
        }
        a.reset();
        assert!(!a.mark(-1));
        assert!(b.mark(-1));
    }
}
