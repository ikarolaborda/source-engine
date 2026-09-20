//! Deterministic host lifecycle and fixed-step scheduling foundations.

pub mod app_system;

use std::fmt;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Created = 0,
    LauncherReady = 1,
    ContentReady = 2,
    LegacyRunning = 3,
    ShuttingDown = 4,
    Stopped = 5,
}

impl TryFrom<u32> for Phase {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Created),
            1 => Ok(Self::LauncherReady),
            2 => Ok(Self::ContentReady),
            3 => Ok(Self::LegacyRunning),
            4 => Ok(Self::ShuttingDown),
            5 => Ok(Self::Stopped),
            _ => Err(Error::InvalidPhase(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePlan {
    pub ticks: u32,
    pub interpolation_numerator_ns: u64,
    pub interpolation_denominator_ns: u64,
    pub dropped_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePacePlan {
    pub ready: bool,
    pub elapsed_ns: u64,
    pub wait_ns: u64,
}

#[derive(Debug, Clone, Default)]
pub struct FramePacer {
    last_frame_ns: Option<u64>,
}

impl FramePacer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decides whether a legacy frame is ready without sleeping in Rust.
    /// The caller retains platform-specific sleeping and message pumping.
    pub fn pace(&mut self, now_ns: u64, minimum_frame_ns: u64) -> FramePacePlan {
        let Some(previous_ns) = self.last_frame_ns else {
            self.last_frame_ns = Some(now_ns);
            return FramePacePlan {
                ready: true,
                elapsed_ns: minimum_frame_ns,
                wait_ns: 0,
            };
        };

        if now_ns < previous_ns {
            self.last_frame_ns = Some(now_ns);
            return FramePacePlan {
                ready: true,
                elapsed_ns: minimum_frame_ns,
                wait_ns: 0,
            };
        }

        let elapsed_ns = now_ns - previous_ns;
        if elapsed_ns < minimum_frame_ns {
            return FramePacePlan {
                ready: false,
                elapsed_ns,
                wait_ns: minimum_frame_ns - elapsed_ns,
            };
        }

        self.last_frame_ns = Some(now_ns);
        FramePacePlan {
            ready: true,
            elapsed_ns,
            wait_ns: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickPlan {
    pub previous_remainder: f64,
    pub remainder: f64,
    pub next_tick: f64,
    pub ticks: u32,
}

#[derive(Debug, Clone, Default)]
pub struct TickScheduler {
    remainder: f64,
}

impl TickScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reproduces Source's frame-time accumulator, including the single-player
    /// alternate-tick rule that may intentionally leave a negative remainder.
    pub fn schedule(
        &mut self,
        frame_time: f64,
        tick_interval: f64,
        start_tick: i32,
        accumulate: bool,
        alternate_ticks: bool,
    ) -> Result<TickPlan, Error> {
        if !frame_time.is_finite()
            || frame_time < 0.0
            || !tick_interval.is_finite()
            || tick_interval <= 0.0
        {
            return Err(Error::InvalidFrameTime);
        }

        let previous_remainder = self.remainder.max(0.0);
        if accumulate {
            self.remainder += frame_time;
        }
        if !self.remainder.is_finite() {
            return Err(Error::InvalidFrameTime);
        }

        let mut ticks = 0u32;
        if self.remainder >= tick_interval {
            let available = (self.remainder / tick_interval).floor();
            if available > f64::from(i32::MAX) {
                return Err(Error::TickCountOverflow);
            }
            ticks = available as u32;
            if alternate_ticks && (i64::from(start_tick) + i64::from(ticks)) & 1 != 0 {
                ticks = ticks.checked_add(1).ok_or(Error::TickCountOverflow)?;
                if ticks > i32::MAX as u32 {
                    return Err(Error::TickCountOverflow);
                }
            }
            self.remainder -= f64::from(ticks) * tick_interval;
        }

        Ok(TickPlan {
            previous_remainder,
            remainder: self.remainder,
            next_tick: tick_interval - self.remainder,
            ticks,
        })
    }
}

impl FramePlan {
    pub fn interpolation_fraction(self) -> f64 {
        self.interpolation_numerator_ns as f64 / self.interpolation_denominator_ns as f64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidPhase(u32),
    InvalidTransition { from: Phase, to: Phase },
    InvalidTickInterval,
    InvalidCatchUpLimit,
    InvalidFrameTime,
    TickCountOverflow,
    WrongPhase(Phase),
    InvalidSessionLimit,
    SessionLimitExceeded(u32),
    SessionFailed,
    FrameLimitExceeded(u64),
    FrameFailed,
    InvalidOperationKind(u32),
    InvalidOperation,
    ClockMovedBackwards { previous_ns: u64, now_ns: u64 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPhase(value) => write!(f, "invalid host phase {value}"),
            Self::InvalidTransition { from, to } => {
                write!(f, "invalid host transition {from:?} -> {to:?}")
            }
            Self::InvalidTickInterval => write!(f, "tick interval must be nonzero"),
            Self::InvalidCatchUpLimit => write!(f, "catch-up tick limit must be nonzero"),
            Self::InvalidFrameTime => write!(f, "frame and tick times must be finite and valid"),
            Self::TickCountOverflow => write!(f, "frame requires too many simulation ticks"),
            Self::WrongPhase(phase) => write!(f, "operation is invalid in host phase {phase:?}"),
            Self::InvalidSessionLimit => write!(f, "session limit must be nonzero"),
            Self::SessionLimitExceeded(limit) => {
                write!(f, "legacy session restart limit {limit} exceeded")
            }
            Self::SessionFailed => write!(f, "legacy session callback failed"),
            Self::FrameLimitExceeded(limit) => {
                write!(f, "legacy frame iteration limit {limit} exceeded")
            }
            Self::FrameFailed => write!(f, "legacy frame callback failed"),
            Self::InvalidOperationKind(kind) => write!(f, "invalid host operation kind {kind}"),
            Self::InvalidOperation => write!(f, "invalid host operation"),
            Self::ClockMovedBackwards {
                previous_ns,
                now_ns,
            } => write!(
                f,
                "monotonic clock moved backwards from {previous_ns} to {now_ns}"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct FixedStepScheduler {
    tick_interval_ns: u64,
    max_catch_up_ticks: u32,
    last_time_ns: Option<u64>,
    accumulated_ns: u64,
}

impl FixedStepScheduler {
    pub fn new(tick_interval_ns: u64, max_catch_up_ticks: u32) -> Result<Self, Error> {
        if tick_interval_ns == 0 {
            return Err(Error::InvalidTickInterval);
        }
        if max_catch_up_ticks == 0 {
            return Err(Error::InvalidCatchUpLimit);
        }
        Ok(Self {
            tick_interval_ns,
            max_catch_up_ticks,
            last_time_ns: None,
            accumulated_ns: 0,
        })
    }

    pub fn advance(&mut self, now_ns: u64) -> Result<FramePlan, Error> {
        let Some(previous_ns) = self.last_time_ns.replace(now_ns) else {
            return Ok(self.plan(0, 0));
        };
        if now_ns < previous_ns {
            self.last_time_ns = Some(previous_ns);
            return Err(Error::ClockMovedBackwards {
                previous_ns,
                now_ns,
            });
        }

        self.accumulated_ns = self
            .accumulated_ns
            .saturating_add(now_ns.saturating_sub(previous_ns));
        let available_ticks = self.accumulated_ns / self.tick_interval_ns;
        let ticks = available_ticks.min(u64::from(self.max_catch_up_ticks)) as u32;
        let dropped_ticks = available_ticks.saturating_sub(u64::from(ticks));
        let dropped_ns = dropped_ticks * self.tick_interval_ns;
        self.accumulated_ns -= (u64::from(ticks) + dropped_ticks) * self.tick_interval_ns;
        Ok(self.plan(ticks, dropped_ns))
    }

    pub fn reset(&mut self) {
        self.last_time_ns = None;
        self.accumulated_ns = 0;
    }

    fn plan(&self, ticks: u32, dropped_ns: u64) -> FramePlan {
        FramePlan {
            ticks,
            interpolation_numerator_ns: self.accumulated_ns.min(self.tick_interval_ns - 1),
            interpolation_denominator_ns: self.tick_interval_ns,
            dropped_ns,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Host {
    phase: Phase,
    scheduler: FixedStepScheduler,
    frame_pacer: FramePacer,
    tick_scheduler: TickScheduler,
    pending_operation: Option<HostOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    Stop,
    Restart,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameAction {
    Continue,
    Stop,
    Restart,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameLoopExit {
    Stop,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLoopReport {
    pub iterations: u64,
    pub exit: FrameLoopExit,
}

pub const HOST_OPERATION_REMEMBER_LOCATION: u32 = 1 << 0;
pub const HOST_OPERATION_BACKGROUND_LEVEL: u32 = 1 << 1;
pub const MAX_HOST_OPERATION_TEXT_BYTES: usize = 255;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOperationKind {
    NewGame = 1,
    LoadGame = 2,
    ChangeLevelSp = 3,
    ChangeLevelMp = 4,
    GameShutdown = 5,
    Shutdown = 6,
    Restart = 7,
}

impl TryFrom<u32> for HostOperationKind {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::NewGame),
            2 => Ok(Self::LoadGame),
            3 => Ok(Self::ChangeLevelSp),
            4 => Ok(Self::ChangeLevelMp),
            5 => Ok(Self::GameShutdown),
            6 => Ok(Self::Shutdown),
            7 => Ok(Self::Restart),
            _ => Err(Error::InvalidOperationKind(value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOperation {
    pub kind: HostOperationKind,
    pub target: String,
    pub landmark: String,
    pub flags: u32,
}

impl HostOperation {
    pub fn new(
        kind: HostOperationKind,
        target: &str,
        landmark: &str,
        flags: u32,
    ) -> Result<Self, Error> {
        if target.len() > MAX_HOST_OPERATION_TEXT_BYTES
            || landmark.len() > MAX_HOST_OPERATION_TEXT_BYTES
            || target.contains('\0')
            || landmark.contains('\0')
        {
            return Err(Error::InvalidOperation);
        }

        let requires_target = matches!(
            kind,
            HostOperationKind::NewGame
                | HostOperationKind::LoadGame
                | HostOperationKind::ChangeLevelSp
                | HostOperationKind::ChangeLevelMp
        );
        let allowed_flags = match kind {
            HostOperationKind::NewGame => {
                HOST_OPERATION_REMEMBER_LOCATION | HOST_OPERATION_BACKGROUND_LEVEL
            }
            HostOperationKind::LoadGame => HOST_OPERATION_REMEMBER_LOCATION,
            _ => 0,
        };
        let has_target = !target.is_empty();
        if requires_target != has_target
            || flags & !allowed_flags != 0
            || (!matches!(
                kind,
                HostOperationKind::ChangeLevelSp | HostOperationKind::ChangeLevelMp
            ) && !landmark.is_empty())
        {
            return Err(Error::InvalidOperation);
        }

        Ok(Self {
            kind,
            target: target.to_owned(),
            landmark: landmark.to_owned(),
            flags,
        })
    }
}

impl Default for Host {
    fn default() -> Self {
        Self {
            phase: Phase::Created,
            scheduler: FixedStepScheduler::new(15_000_000, 8).expect("valid defaults"),
            frame_pacer: FramePacer::new(),
            tick_scheduler: TickScheduler::new(),
            pending_operation: None,
        }
    }
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn transition(&mut self, to: Phase) -> Result<(), Error> {
        let valid = matches!(
            (self.phase, to),
            (Phase::Created, Phase::LauncherReady)
                | (Phase::LauncherReady, Phase::ContentReady)
                | (Phase::ContentReady, Phase::LegacyRunning)
                | (Phase::LegacyRunning, Phase::ShuttingDown)
                | (Phase::ContentReady, Phase::ShuttingDown)
                | (Phase::LauncherReady, Phase::ShuttingDown)
                | (Phase::ShuttingDown, Phase::Stopped)
        );
        if !valid {
            return Err(Error::InvalidTransition {
                from: self.phase,
                to,
            });
        }
        self.phase = to;
        if matches!(to, Phase::ShuttingDown | Phase::Stopped) {
            self.pending_operation = None;
        }
        Ok(())
    }

    pub fn configure_scheduler(
        &mut self,
        tick_interval_ns: u64,
        max_catch_up_ticks: u32,
    ) -> Result<(), Error> {
        self.scheduler = FixedStepScheduler::new(tick_interval_ns, max_catch_up_ticks)?;
        Ok(())
    }

    pub fn advance(&mut self, now_ns: u64) -> Result<FramePlan, Error> {
        self.scheduler.advance(now_ns)
    }

    pub fn pace_frame(&mut self, now_ns: u64, minimum_frame_ns: u64) -> FramePacePlan {
        self.frame_pacer.pace(now_ns, minimum_frame_ns)
    }

    pub fn schedule_ticks(
        &mut self,
        frame_time: f64,
        tick_interval: f64,
        start_tick: i32,
        accumulate: bool,
        alternate_ticks: bool,
    ) -> Result<TickPlan, Error> {
        self.ensure_legacy_running()?;
        self.tick_scheduler.schedule(
            frame_time,
            tick_interval,
            start_tick,
            accumulate,
            alternate_ticks,
        )
    }

    pub fn ensure_legacy_running(&self) -> Result<(), Error> {
        if self.phase != Phase::LegacyRunning {
            return Err(Error::WrongPhase(self.phase));
        }
        Ok(())
    }

    pub fn request_operation(&mut self, operation: HostOperation) -> Result<(), Error> {
        self.ensure_legacy_running()?;
        // The legacy host has one `m_nextState` slot, so the most recent
        // request deliberately replaces an earlier unconsumed request.
        self.pending_operation = Some(operation);
        Ok(())
    }

    pub fn pending_operation(&self) -> Result<Option<&HostOperation>, Error> {
        self.ensure_legacy_running()?;
        Ok(self.pending_operation.as_ref())
    }

    pub fn take_operation(&mut self) -> Result<Option<HostOperation>, Error> {
        self.ensure_legacy_running()?;
        Ok(self.pending_operation.take())
    }

    pub fn clear_operation(&mut self) -> Result<(), Error> {
        self.ensure_legacy_running()?;
        self.pending_operation = None;
        Ok(())
    }

    pub fn run_legacy_sessions(
        &mut self,
        max_sessions: u32,
        mut run: impl FnMut() -> SessionAction,
    ) -> Result<u32, Error> {
        self.ensure_legacy_running()?;
        run_legacy_sessions(max_sessions, &mut run)
    }
}

pub fn run_legacy_sessions(
    max_sessions: u32,
    mut run: impl FnMut() -> SessionAction,
) -> Result<u32, Error> {
    if max_sessions == 0 {
        return Err(Error::InvalidSessionLimit);
    }
    for session_count in 1..=max_sessions {
        match run() {
            SessionAction::Stop => return Ok(session_count),
            SessionAction::Failed => return Err(Error::SessionFailed),
            SessionAction::Restart if session_count == max_sessions => {
                return Err(Error::SessionLimitExceeded(max_sessions));
            }
            SessionAction::Restart => {}
        }
    }
    unreachable!("a nonzero bounded loop always returns")
}

/// Owns the active engine iteration loop. A zero limit means that the loop is
/// intentionally unbounded and may only terminate through its callback.
pub fn run_legacy_frames(
    max_iterations: u64,
    mut run: impl FnMut() -> FrameAction,
) -> Result<FrameLoopReport, Error> {
    let mut iterations = 0u64;
    loop {
        if max_iterations != 0 && iterations == max_iterations {
            return Err(Error::FrameLimitExceeded(max_iterations));
        }
        iterations = iterations.saturating_add(1);
        match run() {
            FrameAction::Continue => {}
            FrameAction::Stop => {
                return Ok(FrameLoopReport {
                    iterations,
                    exit: FrameLoopExit::Stop,
                })
            }
            FrameAction::Restart => {
                return Ok(FrameLoopReport {
                    iterations,
                    exit: FrameLoopExit::Restart,
                })
            }
            FrameAction::Failed => return Err(Error::FrameFailed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_launcher_and_shutdown_lifecycle() {
        let mut host = Host::new();
        assert_eq!(host.phase(), Phase::Created);
        assert!(host.transition(Phase::ContentReady).is_err());
        for phase in [
            Phase::LauncherReady,
            Phase::ContentReady,
            Phase::LegacyRunning,
            Phase::ShuttingDown,
            Phase::Stopped,
        ] {
            host.transition(phase).unwrap();
        }
        assert!(host.transition(Phase::Created).is_err());
    }

    #[test]
    fn schedules_fixed_ticks_and_bounds_catch_up() {
        let mut scheduler = FixedStepScheduler::new(15_000_000, 4).unwrap();
        assert_eq!(scheduler.advance(1_000).unwrap().ticks, 0);
        let plan = scheduler.advance(22_501_000).unwrap();
        assert_eq!(plan.ticks, 1);
        assert_eq!(plan.interpolation_numerator_ns, 7_500_000);
        assert_eq!(plan.interpolation_fraction(), 0.5);

        let plan = scheduler.advance(202_501_000).unwrap();
        assert_eq!(plan.ticks, 4);
        assert_eq!(plan.dropped_ns, 120_000_000);
        assert_eq!(plan.interpolation_fraction(), 0.5);
        assert!(scheduler.advance(1).is_err());
    }

    #[test]
    fn paces_frames_and_recovers_from_clock_rewinds() {
        let mut pacer = FramePacer::new();
        assert_eq!(
            pacer.pace(1_000, 15_000_000),
            FramePacePlan {
                ready: true,
                elapsed_ns: 15_000_000,
                wait_ns: 0,
            }
        );
        assert_eq!(
            pacer.pace(10_001_000, 15_000_000),
            FramePacePlan {
                ready: false,
                elapsed_ns: 10_000_000,
                wait_ns: 5_000_000,
            }
        );
        assert_eq!(
            pacer.pace(15_001_000, 15_000_000),
            FramePacePlan {
                ready: true,
                elapsed_ns: 15_000_000,
                wait_ns: 0,
            }
        );
        assert_eq!(
            pacer.pace(500, 15_000_000),
            FramePacePlan {
                ready: true,
                elapsed_ns: 15_000_000,
                wait_ns: 0,
            }
        );
    }

    #[test]
    fn schedules_live_ticks_and_preserves_alternate_tick_debt() {
        let mut scheduler = TickScheduler::new();
        let plan = scheduler.schedule(0.010, 0.015, 100, true, false).unwrap();
        assert_eq!(plan.ticks, 0);
        assert!((plan.previous_remainder - 0.0).abs() < f64::EPSILON);
        assert!((plan.remainder - 0.010).abs() < 1.0e-12);
        assert!((plan.next_tick - 0.005).abs() < 1.0e-12);

        let plan = scheduler.schedule(0.020, 0.015, 100, true, true).unwrap();
        assert_eq!(plan.ticks, 2);
        assert!(plan.remainder.abs() < 1.0e-12);

        let plan = scheduler.schedule(0.015, 0.015, 100, true, true).unwrap();
        assert_eq!(plan.ticks, 2);
        assert!((plan.remainder + 0.015).abs() < 1.0e-12);
        assert!((plan.next_tick - 0.030).abs() < 1.0e-12);
        let paused = scheduler.schedule(0.010, 0.015, 102, false, false).unwrap();
        assert_eq!(paused.ticks, 0);
        assert_eq!(paused.remainder, plan.remainder);
        assert_eq!(paused.previous_remainder, 0.0);

        assert!(scheduler.schedule(f64::NAN, 0.015, 0, true, false).is_err());
        assert!(scheduler.schedule(0.0, 0.0, 0, true, false).is_err());
    }

    #[test]
    fn owns_bounded_legacy_session_restarts() {
        let mut host = Host::new();
        assert!(matches!(
            host.run_legacy_sessions(1, || SessionAction::Stop),
            Err(Error::WrongPhase(Phase::Created))
        ));
        host.transition(Phase::LauncherReady).unwrap();
        host.transition(Phase::ContentReady).unwrap();
        host.transition(Phase::LegacyRunning).unwrap();

        let mut callbacks = 0;
        let sessions = host
            .run_legacy_sessions(4, || {
                callbacks += 1;
                if callbacks < 3 {
                    SessionAction::Restart
                } else {
                    SessionAction::Stop
                }
            })
            .unwrap();
        assert_eq!((sessions, callbacks), (3, 3));
        assert!(matches!(
            host.run_legacy_sessions(2, || SessionAction::Restart),
            Err(Error::SessionLimitExceeded(2))
        ));
        assert!(matches!(
            host.run_legacy_sessions(1, || SessionAction::Failed),
            Err(Error::SessionFailed)
        ));
    }

    #[test]
    fn owns_and_validates_the_single_pending_host_operation() {
        assert!(HostOperation::new(HostOperationKind::NewGame, "", "", 0).is_err());
        assert!(HostOperation::new(HostOperationKind::Shutdown, "map", "", 0).is_err());
        assert!(HostOperation::new(HostOperationKind::LoadGame, "slot", "", 2).is_err());
        assert!(HostOperation::new(
            HostOperationKind::NewGame,
            &"m".repeat(MAX_HOST_OPERATION_TEXT_BYTES + 1),
            "",
            0,
        )
        .is_err());

        let mut host = Host::new();
        let first = HostOperation::new(HostOperationKind::NewGame, "map_a", "", 0).unwrap();
        assert!(matches!(
            host.request_operation(first.clone()),
            Err(Error::WrongPhase(Phase::Created))
        ));
        host.transition(Phase::LauncherReady).unwrap();
        host.transition(Phase::ContentReady).unwrap();
        host.transition(Phase::LegacyRunning).unwrap();
        host.request_operation(first).unwrap();
        let replacement =
            HostOperation::new(HostOperationKind::ChangeLevelSp, "map_b", "landmark", 0).unwrap();
        host.request_operation(replacement.clone()).unwrap();
        assert_eq!(host.pending_operation().unwrap(), Some(&replacement));
        assert_eq!(host.take_operation().unwrap(), Some(replacement));
        assert_eq!(host.take_operation().unwrap(), None);
    }

    #[test]
    fn owns_legacy_frame_iteration_and_exit_decisions() {
        let mut callbacks = 0;
        let report = run_legacy_frames(8, || {
            callbacks += 1;
            if callbacks < 3 {
                FrameAction::Continue
            } else {
                FrameAction::Restart
            }
        })
        .unwrap();
        assert_eq!(report.iterations, 3);
        assert_eq!(report.exit, FrameLoopExit::Restart);
        assert!(matches!(
            run_legacy_frames(2, || FrameAction::Continue),
            Err(Error::FrameLimitExceeded(2))
        ));
        assert!(matches!(
            run_legacy_frames(1, || FrameAction::Failed),
            Err(Error::FrameFailed)
        ));
    }
}
