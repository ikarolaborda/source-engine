//! App-system startup, reverse-order rollback and module lifetime.
//!
//! Native callbacks still implement each subsystem. Rust owns which callbacks
//! run, how far startup got, and the rule that disconnection precedes unloading.

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create = 0,
    Connect = 1,
    PreInit = 2,
    Init = 3,
    Main = 4,
    Shutdown = 5,
    PostShutdown = 6,
    Disconnect = 7,
    RemoveSystems = 8,
    UnloadModules = 9,
    Destroy = 10,
}

/// Create returns the system count or a negative failure; Connect, PreInit and
/// Init return zero on success. Main's result is preserved verbatim. Cleanup
/// callbacks cannot fail. An unsuccessful Connect/Init must release its own
/// partially acquired resources: only completed operations get their inverse.
/// A completed PreInit gets PostShutdown even if a subsequent Init fails.
/// No process-global state or lock is held, so groups can run nested groups.
pub fn run(mut call: impl FnMut(Operation, u32) -> i32) -> i32 {
    let Some(mut group) = StartedGroup::startup(&mut call) else {
        return -1;
    };
    let result = call(Operation::Main, 0);
    group.shutdown(&mut call);
    result
}

/// Completed startup state for callers that own their event loop. No native
/// pointers or callbacks are stored. Call shutdown before releasing subsystems.
#[derive(Debug)]
pub struct StartedGroup {
    connected: u32,
    initialized: u32,
    preinitialized: bool,
    stopped: bool,
}

impl StartedGroup {
    /// Failure performs rollback immediately and returns no live group. Success
    /// leaves the group initialized without calling Main or any cleanup action.
    pub fn startup(call: &mut impl FnMut(Operation, u32) -> i32) -> Option<Self> {
        let mut group = Self {
            connected: 0,
            initialized: 0,
            preinitialized: false,
            stopped: false,
        };
        let count = call(Operation::Create, 0);

        'startup: {
            if count < 0 {
                break 'startup;
            }
            for index in 0..count as u32 {
                if call(Operation::Connect, index) != 0 {
                    break 'startup;
                }
                group.connected += 1;
            }
            if call(Operation::PreInit, 0) != 0 {
                break 'startup;
            }
            group.preinitialized = true;
            for index in 0..count as u32 {
                if call(Operation::Init, index) != 0 {
                    break 'startup;
                }
                group.initialized += 1;
            }
            return Some(group);
        }
        group.shutdown(call);
        None
    }

    /// Idempotent: a completed shutdown never dispatches native callbacks again.
    pub fn shutdown(&mut self, call: &mut impl FnMut(Operation, u32) -> i32) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        for index in (0..self.initialized).rev() {
            call(Operation::Shutdown, index);
        }
        if self.preinitialized {
            call(Operation::PostShutdown, 0);
        }
        for index in (0..self.connected).rev() {
            call(Operation::Disconnect, index);
        }
        call(Operation::RemoveSystems, 0);
        call(Operation::UnloadModules, 0);
        call(Operation::Destroy, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{run, Operation::*};

    fn exercise(
        count: u32,
        fail: Option<(Operation, u32)>,
        main: i32,
    ) -> (i32, Vec<(Operation, u32)>) {
        let mut trace = Vec::new();
        let result = run(|operation, index| {
            trace.push((operation, index));
            if fail == Some((operation, index)) {
                return -1;
            }
            match operation {
                Create => count as i32,
                Main => main,
                _ => 0,
            }
        });
        (result, trace)
    }

    #[test]
    fn success_and_main_error_both_clean_up_before_unloading() {
        for exit in [-1, 0, 1, 7] {
            let (result, trace) = exercise(2, None, exit);
            assert_eq!(result, exit);
            assert_eq!(
                trace,
                vec![
                    (Create, 0),
                    (Connect, 0),
                    (Connect, 1),
                    (PreInit, 0),
                    (Init, 0),
                    (Init, 1),
                    (Main, 0),
                    (Shutdown, 1),
                    (Shutdown, 0),
                    (PostShutdown, 0),
                    (Disconnect, 1),
                    (Disconnect, 0),
                    (RemoveSystems, 0),
                    (UnloadModules, 0),
                    (Destroy, 0)
                ]
            );
        }
    }

    #[test]
    fn failure_at_every_startup_boundary_unwinds_only_completed_work() {
        for count in 0..16 {
            let failures = [(Create, 0), (PreInit, 0)]
                .into_iter()
                .chain((0..count).map(|i| (Connect, i)))
                .chain((0..count).map(|i| (Init, i)));
            for fail in failures {
                let (result, trace) = exercise(count, Some(fail), 42);
                assert_eq!(result, -1);
                let at = trace.iter().position(|step| *step == fail).unwrap();
                let connected = if fail.0 == Create {
                    0
                } else if fail.0 == Connect {
                    fail.1
                } else {
                    count
                };
                let initialized = if fail.0 == Init { fail.1 } else { 0 };
                let mut expected: Vec<_> = (0..initialized).rev().map(|i| (Shutdown, i)).collect();
                if fail.0 == Init {
                    expected.push((PostShutdown, 0));
                }
                expected.extend((0..connected).rev().map(|i| (Disconnect, i)));
                expected.extend([(RemoveSystems, 0), (UnloadModules, 0), (Destroy, 0)]);
                assert_eq!(
                    &trace[at + 1..],
                    expected,
                    "failure {fail:?}, count {count}"
                );
                assert!(!trace.iter().any(|step| step.0 == Main));
            }
        }
    }

    #[test]
    fn empty_groups_and_nested_failed_groups_do_not_share_state() {
        let mut trace = Vec::new();
        let result = run(|op, index| {
            trace.push((op, index));
            match op {
                Create => 0,
                Main => exercise(3, Some((Init, 1)), 42).0,
                _ => 0,
            }
        });
        assert_eq!(result, -1);
        assert_eq!(
            trace,
            vec![
                (Create, 0),
                (PreInit, 0),
                (Main, 0),
                (PostShutdown, 0),
                (RemoveSystems, 0),
                (UnloadModules, 0),
                (Destroy, 0)
            ]
        );
    }

    #[test]
    fn split_startup_defers_cleanup_and_shutdown_is_idempotent() {
        let mut trace = Vec::new();
        let mut group = StartedGroup::startup(&mut |op, index| {
            trace.push((op, index));
            if op == Create {
                2
            } else {
                0
            }
        })
        .unwrap();
        assert_eq!(
            trace,
            vec![
                (Create, 0),
                (Connect, 0),
                (Connect, 1),
                (PreInit, 0),
                (Init, 0),
                (Init, 1)
            ]
        );
        group.shutdown(&mut |op, index| {
            trace.push((op, index));
            0
        });
        let after = trace.clone();
        group.shutdown(&mut |_, _| panic!("duplicate shutdown"));
        assert_eq!(trace, after);
        let (_, synchronous) = exercise(2, None, 42);
        assert_eq!(
            trace,
            synchronous
                .into_iter()
                .filter(|(op, _)| *op != Main)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn split_failures_match_the_synchronous_rollback() {
        for fail in [
            (Create, 0),
            (Connect, 0),
            (Connect, 1),
            (PreInit, 0),
            (Init, 0),
            (Init, 1),
        ] {
            let mut trace = Vec::new();
            assert!(StartedGroup::startup(&mut |op, index| {
                trace.push((op, index));
                if fail == (op, index) {
                    -1
                } else if op == Create {
                    2
                } else {
                    0
                }
            })
            .is_none());
            assert_eq!(trace, exercise(2, Some(fail), 42).1);
        }
    }
}
