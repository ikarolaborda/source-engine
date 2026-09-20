//! Ordered mount ownership; resource representation is supplied by the host.
use std::sync::atomic::{AtomicU32, Ordering};
pub const MAX_MOUNTS: usize = 65_536;

pub struct StoreIds {
    next: AtomicU32,
}
impl Default for StoreIds {
    fn default() -> Self {
        Self::new()
    }
}
impl StoreIds {
    pub const fn new() -> Self {
        Self {
            next: AtomicU32::new(1),
        }
    }
    pub fn allocate(&self) -> Option<i32> {
        self.next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                if n <= i32::MAX as u32 {
                    Some(n + 1)
                } else {
                    None
                }
            })
            .ok()
            .map(|id| id as i32)
    }
}

pub struct MountTable<T> {
    entries: Vec<T>,
}

impl<T> Default for MountTable<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> MountTable<T> {
    pub fn entries(&self) -> &[T] {
        &self.entries
    }
    pub fn insert(&mut self, index: usize, entry: T) -> Result<(), T> {
        if index > self.entries.len() || self.entries.len() >= MAX_MOUNTS {
            return Err(entry);
        }
        self.entries.insert(index, entry);
        Ok(())
    }
    pub fn remove(&mut self, index: usize, fast: bool) -> Option<T> {
        if index >= self.entries.len() {
            return None;
        }
        Some(if fast {
            self.entries.swap_remove(index)
        } else {
            self.entries.remove(index)
        })
    }
    pub fn take_all(&mut self) -> Vec<T> {
        std::mem::take(&mut self.entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn store_ids_stop_after_the_last_positive_id_without_wrapping() {
        let ids = StoreIds {
            next: AtomicU32::new(i32::MAX as u32),
        };
        assert_eq!(ids.allocate(), Some(i32::MAX));
        assert_eq!(ids.allocate(), None);
        assert_eq!(ids.allocate(), None);
        let ids = StoreIds::new();
        assert_eq!(ids.allocate(), Some(1));
        assert_eq!(ids.allocate(), Some(2));
    }

    #[test]
    fn native_order_and_fast_remove_are_distinct() {
        let mut mounts = MountTable::default();
        for (index, value) in [(0, 2), (0, 1), (2, 4), (2, 3)] {
            mounts.insert(index, value).unwrap();
        }
        assert_eq!(mounts.entries(), [1, 2, 3, 4]);
        assert_eq!(mounts.insert(5, 99), Err(99));
        assert_eq!(mounts.remove(4, true), None);
        assert_eq!(mounts.remove(1, false), Some(2));
        assert_eq!(mounts.entries(), [1, 3, 4]);
        assert_eq!(mounts.remove(0, true), Some(1));
        assert_eq!(mounts.entries(), [4, 3]);
        assert_eq!(mounts.take_all(), [4, 3]);
        assert!(mounts.entries().is_empty());
    }

    #[test]
    fn removed_resources_survive_independent_snapshot_owners() {
        let mut mounts = MountTable::default();
        let entry = Arc::new(42);
        let weak = Arc::downgrade(&entry);
        mounts.insert(0, entry).unwrap();
        let snapshot = mounts.entries().to_vec();
        drop(mounts.take_all());
        assert_eq!(**snapshot.first().unwrap(), 42);
        assert!(weak.upgrade().is_some());
        drop(snapshot);
        assert!(weak.upgrade().is_none());
        for index in 0..MAX_MOUNTS {
            mounts.insert(index, Arc::new(0)).unwrap();
        }
        assert!(mounts.insert(MAX_MOUNTS, Arc::new(1)).is_err());
    }
}
