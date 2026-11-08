//! Opt-in renderer instrumentation, not a presentation-FPS benchmark.
use std::{
    cell::{Cell, RefCell},
    time::Instant,
};
#[derive(Default)]
pub struct RenderMetrics {
    active: Cell<bool>,
    started: Cell<Option<Instant>>,
    previous: Cell<Option<Instant>>,
    durations: RefCell<Vec<f64>>,
    intervals: RefCell<Vec<f64>>,
    idle_frames: Cell<u64>,
}
impl RenderMetrics {
    pub fn active(&self, value: bool) {
        if self.active.replace(value) != value {
            self.previous.set(None);
        }
    }
    pub fn frame(&self, state: slint::RenderingState) {
        match state {
            slint::RenderingState::BeforeRendering => self.started.set(Some(Instant::now())),
            slint::RenderingState::AfterRendering => {
                let now = Instant::now();
                if self.active.get() {
                    if let Some(start) = self.started.take()
                        && self.durations.borrow().len() < 8192
                    {
                        self.durations
                            .borrow_mut()
                            .push(now.duration_since(start).as_secs_f64() * 1000.);
                    }
                    if let Some(previous) = self.previous.replace(Some(now))
                        && self.intervals.borrow().len() < 8192
                    {
                        self.intervals
                            .borrow_mut()
                            .push(now.duration_since(previous).as_secs_f64() * 1000.);
                    }
                } else {
                    self.idle_frames.set(self.idle_frames.get() + 1);
                    self.previous.set(None);
                }
            }
            _ => {}
        }
    }
    pub fn summary(&self) -> String {
        let durations = self.durations.borrow();
        let mut intervals = self.intervals.borrow().clone();
        intervals.sort_by(f64::total_cmp);
        let mean = if durations.is_empty() {
            0.
        } else {
            durations.iter().sum::<f64>() / durations.len() as f64
        };
        let p95 = intervals
            .get(intervals.len().saturating_sub(1) * 95 / 100)
            .copied()
            .unwrap_or(0.);
        format!(
            "render_metrics animated_frames={} mean_draw_ms={mean:.3} p95_frame_interval_ms={p95:.3} other_frames={} (renderer callbacks before swap; not displayed FPS)",
            durations.len(),
            self.idle_frames.get()
        )
    }
}
