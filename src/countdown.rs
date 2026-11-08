//! A local, event-driven countdown driven by the GUI's monotonic millisecond clock.
//!
//! The owner schedules `next_deadline` as a one-shot GUI timer. Delayed wakes
//! consume the actual elapsed time; there is no fixed-rate tick accumulation.

const MAX_DURATION_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub duration_seconds: u64,
    /// Ceiling of the remaining time: a newly started 5-minute timer reads 5:00.
    pub remaining_seconds: u64,
    pub running: bool,
    /// Remains true after completion until reset, reconfiguration, or restart.
    pub finished: bool,
}

#[derive(Clone, Debug)]
pub struct Countdown {
    duration_ms: u64,
    remaining_ms: u64,
    last_update: u64,
    running: bool,
    finished: bool,
    completion_pending: bool,
}

impl Default for Countdown {
    fn default() -> Self {
        Self::new(5 * 60)
    }
}

impl Countdown {
    pub fn new(duration_seconds: u64) -> Self {
        let duration_ms = duration_seconds.clamp(1, MAX_DURATION_SECONDS) * 1000;
        Self {
            duration_ms,
            remaining_ms: duration_ms,
            last_update: 0,
            running: false,
            finished: false,
            completion_pending: false,
        }
    }

    fn advance(&mut self, now_ms: u64) {
        // Regressing input clocks cannot add time or double-charge elapsed time.
        let now_ms = now_ms.max(self.last_update);
        if self.running {
            self.remaining_ms = self.remaining_ms.saturating_sub(now_ms - self.last_update);
            if self.remaining_ms == 0 {
                self.running = false;
                self.finished = true;
                self.completion_pending = true;
            }
        }
        self.last_update = now_ms;
    }

    /// Configure a new duration and leave it stopped at its full duration.
    pub fn set_duration(&mut self, duration_seconds: u64, now_ms: u64) {
        self.duration_ms = duration_seconds.clamp(1, MAX_DURATION_SECONDS) * 1000;
        self.reset(now_ms);
    }

    /// Start, resume a paused timer, or restart a completed timer.
    pub fn start(&mut self, now_ms: u64) {
        self.advance(now_ms);
        if self.running {
            return;
        }
        if self.remaining_ms == 0 {
            self.remaining_ms = self.duration_ms;
        }
        self.finished = false;
        self.completion_pending = false;
        self.running = true;
    }

    pub fn pause(&mut self, now_ms: u64) {
        self.advance(now_ms);
        self.running = false;
    }

    /// Stop and restore the configured duration, without emitting completion.
    pub fn reset(&mut self, now_ms: u64) {
        self.advance(now_ms);
        self.remaining_ms = self.duration_ms;
        self.running = false;
        self.finished = false;
        self.completion_pending = false;
    }

    pub fn snapshot(&mut self, now_ms: u64) -> Snapshot {
        self.advance(now_ms);
        Snapshot {
            duration_seconds: self.duration_ms / 1000,
            remaining_seconds: self.remaining_ms.div_ceil(1000),
            running: self.running,
            finished: self.finished,
        }
    }

    /// Consume the completion event once, even after a delayed GUI wake.
    pub fn take_finished(&mut self, now_ms: u64) -> bool {
        self.advance(now_ms);
        std::mem::take(&mut self.completion_pending)
    }

    /// Absolute clock time when the displayed remaining seconds next change.
    /// No wake is needed while paused or finished.
    pub fn next_deadline(&mut self, now_ms: u64) -> Option<u64> {
        self.advance(now_ms);
        self.running.then(|| {
            self.last_update
                .saturating_add((self.remaining_ms - 1) % 1000 + 1)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_ceiling_and_wakes_at_second_boundaries() {
        let mut timer = Countdown::new(5);
        assert_eq!(timer.next_deadline(50), None);
        timer.start(100);
        assert_eq!(timer.snapshot(101).remaining_seconds, 5);
        assert_eq!(timer.next_deadline(101), Some(1100));
        assert_eq!(timer.snapshot(1100).remaining_seconds, 4);
        assert_eq!(timer.next_deadline(1101), Some(2100));
    }

    #[test]
    fn delayed_wakes_consume_elapsed_time_and_emit_completion_once() {
        let mut timer = Countdown::new(10);
        timer.start(1000);
        assert_eq!(timer.snapshot(9500).remaining_seconds, 2);
        assert!(!timer.take_finished(9500));
        assert!(timer.take_finished(100_000));
        assert!(!timer.take_finished(100_001));
        let snapshot = timer.snapshot(100_002);
        assert_eq!(snapshot.remaining_seconds, 0);
        assert!(snapshot.finished);
        assert!(!snapshot.running);
        assert_eq!(timer.next_deadline(100_002), None);
    }

    #[test]
    fn pause_preserves_fractional_time_and_resume_is_exact() {
        let mut timer = Countdown::new(3);
        timer.start(100);
        timer.pause(600);
        assert_eq!(timer.snapshot(50_000).remaining_seconds, 3);
        assert_eq!(timer.next_deadline(50_000), None);
        timer.start(50_000);
        assert_eq!(timer.next_deadline(50_000), Some(50_500));
        assert_eq!(timer.snapshot(52_499).remaining_seconds, 1);
        assert!(timer.take_finished(52_500));
    }

    #[test]
    fn restart_and_reset_clear_completion_and_restore_full_duration() {
        let mut timer = Countdown::new(1);
        timer.start(0);
        assert!(timer.snapshot(1000).finished);
        timer.start(2000);
        assert!(!timer.take_finished(2000));
        assert_eq!(timer.snapshot(2000).remaining_seconds, 1);
        timer.reset(2500);
        let snapshot = timer.snapshot(10_000);
        assert_eq!(snapshot.remaining_seconds, 1);
        assert!(!snapshot.finished && !snapshot.running);
        assert!(!timer.take_finished(10_000));
    }

    #[test]
    fn repeated_start_is_idempotent_and_regressing_clocks_are_safe() {
        let mut timer = Countdown::new(3);
        timer.start(1000);
        timer.start(2500);
        assert_eq!(timer.snapshot(2000).remaining_seconds, 2);
        assert!(!timer.take_finished(3999));
        assert!(timer.take_finished(4000));
    }

    #[test]
    fn duration_input_is_bounded_and_reconfiguration_stops_timer() {
        let mut timer = Countdown::new(0);
        assert_eq!(timer.snapshot(0).duration_seconds, 1);
        timer.start(0);
        timer.set_duration(u64::MAX, 10);
        let snapshot = timer.snapshot(20);
        assert_eq!(snapshot.duration_seconds, MAX_DURATION_SECONDS);
        assert_eq!(snapshot.remaining_seconds, MAX_DURATION_SECONDS);
        assert!(!snapshot.running && !snapshot.finished);
        assert_eq!(timer.next_deadline(20), None);
    }

    #[test]
    fn extreme_elapsed_time_finishes_without_overflow() {
        let mut timer = Countdown::new(300);
        timer.start(0);
        assert!(timer.take_finished(u64::MAX));
        assert_eq!(timer.next_deadline(u64::MAX), None);
    }
}
