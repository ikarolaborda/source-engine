#![no_main]
use libfuzzer_sys::fuzz_target;
use source_filesystem::mount_table::MountTable;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Lease {
    value: u8,
    dropped: Arc<AtomicUsize>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

fuzz_target!(|data: &[u8]| {
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut allocated = 0;
    let mut table = MountTable::default();
    let mut expected = Vec::new();
    let mut snapshot = Vec::new();
    let mut expected_snapshot = Vec::new();
    for command in data.chunks_exact(3).take(1024) {
        let index = usize::from(command[1]);
        match command[0] % 5 {
            0 => {
                let entry = Arc::new(Lease {
                    value: command[2],
                    dropped: Arc::clone(&dropped),
                });
                allocated += 1;
                let result = table.insert(index, entry);
                if index <= expected.len() {
                    assert!(result.is_ok());
                    expected.insert(index, command[2]);
                } else {
                    assert!(result.is_err());
                }
            }
            1 | 2 => {
                let fast = command[0] % 5 == 2;
                let removed = table.remove(index, fast);
                if index < expected.len() {
                    assert_eq!(removed.as_ref().unwrap().value, expected[index]);
                    if fast {
                        expected.swap_remove(index);
                    } else {
                        expected.remove(index);
                    }
                } else {
                    assert!(removed.is_none());
                }
            }
            3 => {
                snapshot = table.entries().to_vec();
                expected_snapshot = expected.clone();
            }
            _ => {
                drop(table.take_all());
                expected.clear();
            }
        }
        assert_eq!(
            table.entries().iter().map(|e| e.value).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            snapshot.iter().map(|e| e.value).collect::<Vec<_>>(),
            expected_snapshot
        );
    }
    drop(table);
    drop(snapshot);
    assert_eq!(dropped.load(Ordering::Relaxed), allocated);
});
